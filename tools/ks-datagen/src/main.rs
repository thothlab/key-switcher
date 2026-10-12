//! `ks-datagen` — генератор `data/`. Единственный крейт workspace с сетью.
//!
//! 1. Скачивает hunspell `ru_RU` и `en_US` из LibreOffice dictionaries (коммит зафиксирован и
//!    входит в ключ кэша), разворачивает словоформы (свой unmunch, [`hunspell`]), приводит к нижнему
//!    регистру, `ё→е`, отбрасывает формы с цифрами и знаками и пишет `fst::Set` (gzip).
//! 2. Обучает символьные n-граммные модели **только на этих словоформах** (Witten–Bell с
//!    интерполяцией до униграмм с add-one) и пишет плотную таблицу квантованных `−ln P`.
//! 3. Проверяет отложенность корпуса оценки ([`heldout`]): слово `data/eval`, которого нет в сырой
//!    развёртке hunspell, не должно попасть ни во fst, ни в обучение LM — иначе генератор падает.
//! 4. Проверяет курируемые списки из `tools/ks-datagen/curated/` и кладёт их в `data/`.
//!
//! Выборка Tatoeba в `data/eval/` создаётся, если её нет, и перезаписывается только по
//! `--resample-eval`: цифры `docs/eval.md` воспроизводимы на закоммиченном корпусе.
//! Сырьё кэшируется в `target/ks-datagen-cache/`; выгрузки Tatoeba — bz2, нужен `bunzip2`.
//!
//! `cargo run --release -p ks-datagen -- [--order N] [--refresh] [--resample-eval]`

mod heldout;
mod hunspell;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use hunspell::{Lang, normalize_form};

/// Коммит LibreOffice/dictionaries, из которого берутся словари: воспроизводимость данных.
const LO_COMMIT: &str = "32b006a2c22a4ac7e8ed3f03346f7b3d85a970a4";
const LO_RAW: &str = "https://raw.githubusercontent.com/LibreOffice/dictionaries";
const TATOEBA: &str = "https://downloads.tatoeba.org/exports/per_language";
/// Доля предложений Tatoeba в выборке, в десятитысячных. Порог на хэш id, а не «N наименьших»:
/// новые предложения следующей выгрузки не сдвигают уже отобранные.
const EVAL_SHARE_RU: u64 = 160;
const EVAL_SHARE_EN: u64 = 90;
const EVAL_SALT: u64 = 0x6b73_6576_616c_0001;
const DEFAULT_ORDER: usize = 3;
/// Квант `−ln P` в таблице LM: 1/10 ната, предел 25,5 ната.
const LM_SCALE: f64 = 10.0;

fn main() -> Result<()> {
    let (mut order, mut refresh, mut resample) = (DEFAULT_ORDER, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--order" => order = args.next().context("--order N")?.parse()?,
            "--refresh" => refresh = true,
            "--resample-eval" => resample = true,
            other => {
                bail!("неизвестный аргумент {other}; есть --order N, --refresh, --resample-eval")
            }
        }
    }
    ensure!((2..=4).contains(&order), "порядок модели — от 2 до 4");

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let cache = root.join("target/ks-datagen-cache");
    let lo_cache = cache.join(format!("lo-{LO_COMMIT}"));
    let data = root.join("data");
    let curated = root.join("tools/ks-datagen/curated");
    for dir in [&lo_cache, &data.join("eval"), &data.join("licenses")] {
        fs::create_dir_all(dir)?;
    }

    // Корпус оценки: закоммиченная выборка, новая — только если её нет или по --resample-eval.
    let mut eval_section = String::new();
    let mut corpus: HashMap<Lang, String> = HashMap::new();
    for (lang, code3, share) in [
        (Lang::Ru, "rus", EVAL_SHARE_RU),
        (Lang::En, "eng", EVAL_SHARE_EN),
    ] {
        let path = data.join(format!("eval/tatoeba_{}.tsv", lang.code()));
        if resample || !path.exists() {
            let sample = sample_tatoeba(&cache, lang, code3, share, refresh)?;
            fs::write(&path, &sample.text)?;
            println!(
                "Tatoeba {code3}: подходящих {}, в выборке {} (выгрузка {})",
                sample.eligible, sample.picked, sample.export
            );
        } else {
            println!(
                "Tatoeba {code3}: data/eval оставлен как есть (новая выборка — --resample-eval)"
            );
        }
        let text = fs::read_to_string(&path)?;
        let sentences = text.lines().filter(|l| !l.starts_with('#')).count();
        writeln!(
            eval_section,
            "{code3}_export = \"{}\"\n{code3}_sentences = {sentences}\n{code3}_share_per_10000 = {share}",
            export_of(&text)
        )?;
        corpus.insert(lang, text);
    }

    let mut manifest = String::from("# Сгенерировано ks-datagen; не править руками.\n");
    writeln!(
        manifest,
        "\n[hunspell]\nsource = \"https://github.com/LibreOffice/dictionaries\"\ncommit = \"{LO_COMMIT}\""
    )?;
    // Словоформы обоих языков — для аудита курируемых списков после цикла.
    let mut forms: HashMap<Lang, BTreeSet<String>> = HashMap::new();
    for (lang, dir, name) in [(Lang::Ru, "ru_RU", "ru_RU"), (Lang::En, "en", "en_US")] {
        let url = |file: &str| format!("{LO_RAW}/{LO_COMMIT}/{dir}/{file}");
        let aff = fetch_text(&lo_cache, &url(&format!("{name}.aff")), refresh)?;
        let dic = fetch_text(&lo_cache, &url(&format!("{name}.dic")), refresh)?;
        let readme_name = format!("README_{name}.txt");
        let readme = fetch_text(&lo_cache, &url(&readme_name), refresh)?;
        fs::write(data.join("licenses").join(&readme_name), readme)?;

        let aff = hunspell::parse_aff(&aff).with_context(|| format!("{name}.aff"))?;
        let expansion =
            hunspell::unmunch(&aff, &dic, lang).with_context(|| format!("{name}.dic"))?;
        // В словарь и в обучение LM идёт ровно сырая развёртка hunspell; любая добавка сюда
        // проходит через проверку отложенности ниже.
        let dictionary: &BTreeSet<String> = &expansion.forms;
        let lm_train: &BTreeSet<String> = &expansion.forms;

        let fst_bytes = build_fst(dictionary)?;
        let eval_tokens = corpus_tokens(lang, &corpus[&lang]);
        let leaks = heldout::violations(&eval_tokens, &expansion.forms, &fst_bytes, lm_train)?;
        ensure!(
            leaks.is_empty(),
            "корпус оценки попал в словарь или в обучение LM {name}: {:?}",
            &leaks[..leaks.len().min(10)]
        );
        let outside = eval_tokens
            .iter()
            .filter(|t| !expansion.forms.contains(*t))
            .count();

        let fst_gz = gzip(&fst_bytes)?;
        fs::write(data.join(format!("words_{}.fst.gz", lang.code())), &fst_gz)?;
        let lm_bytes = encode_lm(lang, order, &train_lm(lang, lm_train, order));
        let lm_gz = gzip(&lm_bytes)?;
        fs::write(data.join(format!("lm_{}.bin.gz", lang.code())), &lm_gz)?;

        println!(
            "{name}: статей {}, развёрнуто {}, словоформ {}, fst {} Б (gzip {} Б), LM {} Б (gzip {} Б); \
             слов корпуса {}, из них вне hunspell {outside}, утечек 0",
            expansion.entries,
            expansion.raw,
            dictionary.len(),
            fst_bytes.len(),
            fst_gz.len(),
            lm_bytes.len(),
            lm_gz.len(),
            eval_tokens.len(),
        );
        writeln!(
            manifest,
            "\n[{code}]\ndic_entries = {}\nraw_forms = {}\nforms = {}\nfst_bytes = {}\nfst_gz_bytes = {}\n\
             lm_order = {order}\nlm_smoothing = \"Witten-Bell interpolated, unigram add-one\"\n\
             lm_train = \"только словоформы {name} — те же, что в words_{code}.fst.gz\"\n\
             lm_bytes = {}\nlm_gz_bytes = {}\n\
             held_out_eval_words = {}\nheld_out_outside_hunspell = {outside}\nheld_out_leaks = 0",
            expansion.entries,
            expansion.raw,
            dictionary.len(),
            fst_bytes.len(),
            fst_gz.len(),
            lm_bytes.len(),
            lm_gz.len(),
            eval_tokens.len(),
            code = lang.code(),
        )?;
        forms.insert(lang, expansion.forms);
    }

    let short = short_words(&fs::read_to_string(curated.join("short_words.tsv"))?)?;
    fs::write(data.join("short_words.tsv"), short)?;
    let apps = excluded_apps(&fs::read_to_string(curated.join("excluded_apps.tsv"))?)?;
    fs::write(data.join("excluded_apps.tsv"), apps)?;
    let adversarial_src = fs::read_to_string(curated.join("adversarial.tsv"))?;
    let (weak, weak_ru, weak_en) = weak_forms(
        &fs::read_to_string(curated.join("weak_forms.tsv"))?,
        &forms,
        &adversarial_src,
        &Tables::fixtures(),
    )?;
    fs::write(data.join("weak_forms.tsv"), weak)?;
    writeln!(
        manifest,
        "\n[weak_forms]\nsource = \"tools/ks-datagen/curated/weak_forms.tsv\"\n\
         rule = \"форма не защищена словарным вето, если образ — словарное слово другого языка и проходит θ_dict\"\n\
         ru = {weak_ru}\nen = {weak_en}"
    )?;
    let adversarial = adversarial(&adversarial_src)?;
    fs::write(data.join("eval/adversarial.tsv"), adversarial)?;

    writeln!(
        manifest,
        "\n[eval]\nsource = \"{TATOEBA}\"\nlicense = \"CC BY 2.0 FR\"\n\
         held_out = \"проверяется генератором: слова корпуса вне развёртки hunspell не попадают ни во fst, ни в обучение LM\"\n{eval_section}"
    )?;
    fs::write(
        data.join("manifest.toml"),
        manifest.trim_end().to_owned() + "\n",
    )?;
    println!("data/ записан: {}", data.display());
    Ok(())
}

/// Слова предложений корпуса, приведённые так же, как словоформы словаря.
fn corpus_tokens(lang: Lang, text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .flat_map(|(_, sentence)| sentence.split_whitespace())
        .filter_map(|t| normalize_form(lang, t.trim_matches(|c: char| !c.is_alphabetic())))
        .collect()
}

/// Дата выгрузки из заголовка файла корпуса.
fn export_of(text: &str) -> String {
    text.lines()
        .next()
        .and_then(|l| l.split_once("выгрузка "))
        .map(|(_, e)| e.trim().trim_end_matches('.').to_owned())
        .unwrap_or_else(|| "неизвестно".to_owned())
}

// ---------------------------------------------------------------------------------------------
// Скачивание

/// Скачивает файл в каталог кэша (если его там нет или просили `--refresh`) и возвращает путь.
fn fetch(cache: &Path, url: &str, refresh: bool) -> Result<PathBuf> {
    let name = url.rsplit('/').next().context("пустой URL")?;
    let path = cache.join(name);
    if path.exists() && !refresh {
        return Ok(path);
    }
    eprintln!("скачиваю {url}");
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build(),
    );
    let mut resp = agent.get(url).call().with_context(|| {
        format!(
            "{url}: не скачалось; положите файл в {} вручную (tools/ks-datagen/fetch-ranged.sh)",
            cache.display()
        )
    })?;
    if let Some(modified) = resp
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
    {
        fs::write(cache.join(format!("{name}.last-modified")), modified)?;
    }
    let part = cache.join(format!("{name}.part"));
    let mut file = fs::File::create(&part)?;
    std::io::copy(&mut resp.body_mut().as_reader(), &mut file).with_context(|| {
        format!("{url}: загрузка оборвалась; докачка по частям — tools/ks-datagen/fetch-ranged.sh")
    })?;
    file.flush()?;
    fs::rename(&part, &path)?;
    Ok(path)
}

fn fetch_text(cache: &Path, url: &str, refresh: bool) -> Result<String> {
    let path = fetch(cache, url, refresh)?;
    fs::read_to_string(&path).with_context(|| path.display().to_string())
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

fn build_fst(forms: &BTreeSet<String>) -> Result<Vec<u8>> {
    let mut builder = fst::SetBuilder::memory();
    for form in forms {
        builder.insert(form)?;
    }
    Ok(builder.into_inner()?)
}

// ---------------------------------------------------------------------------------------------
// Символьная LM

fn symbol(alphabet: &[char], c: char) -> usize {
    alphabet.iter().position(|&a| a == c).map_or(1, |p| p + 2)
}

/// Таблица `−ln P(c | h)` для всех n-грамм порядка `order`, квантованная в `u8`.
///
/// Последовательность слова — `order−1` маркеров границы, буквы, маркер границы. Сглаживание —
/// Witten–Bell с интерполяцией: `P_m(c|h) = (C(h,c) + T(h)·P_{m−1}(c|h′)) / (C(h) + T(h))`, где
/// `T(h)` — число разных символов после `h`, `h′` — `h` без старшего символа; у невиданного
/// контекста `P_m = P_{m−1}`. Униграммы — add-one. Символ вне алфавита в знакомом контексте
/// упирается в нижнюю границу таблицы (−25,5 ната), n-граммы с ним в контексте — около −14…−17.
fn train_lm(lang: Lang, forms: &BTreeSet<String>, order: usize) -> Vec<u8> {
    let alphabet = lang.alphabet();
    let v = alphabet.len() + 2;
    let mut counts: Vec<Vec<u64>> = (1..=order).map(|m| vec![0u64; v.pow(m as u32)]).collect();
    let mut seq = Vec::new();
    for form in forms {
        seq.clear();
        seq.extend(std::iter::repeat_n(0usize, order - 1));
        seq.extend(form.chars().map(|c| symbol(&alphabet, c)));
        seq.push(0);
        for i in (order - 1)..seq.len() {
            let (mut idx, mut pow) = (0usize, 1usize);
            for (m, table) in counts.iter_mut().enumerate() {
                idx += seq[i - m] * pow;
                table[idx] += 1;
                pow *= v;
            }
        }
    }
    let total: u64 = counts[0].iter().sum();
    let mut prob: Vec<f64> = counts[0]
        .iter()
        .map(|&c| (c as f64 + 1.0) / (total as f64 + v as f64))
        .collect();
    for table in &counts[1..] {
        let lower_len = prob.len();
        let mut next = vec![0f64; table.len()];
        for (h, row) in table.chunks(v).enumerate() {
            let seen: u64 = row.iter().sum();
            let types = row.iter().filter(|&&c| c > 0).count() as f64;
            for (c, &count) in row.iter().enumerate() {
                let idx = h * v + c;
                let lower = prob[idx % lower_len];
                next[idx] = if seen == 0 {
                    lower
                } else {
                    (count as f64 + types * lower) / (seen as f64 + types)
                };
            }
        }
        prob = next;
    }
    prob.iter()
        .map(|p| (-p.ln() * LM_SCALE).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// Формат (до gzip): `KSLM`, версия 1, порядок, квант (`u8`, 1/квант ната), размер алфавита
/// с двумя служебными символами, длина алфавита в байтах (`u16` LE), алфавит UTF-8, таблица.
fn encode_lm(lang: Lang, order: usize, table: &[u8]) -> Vec<u8> {
    let alphabet: String = lang.alphabet().into_iter().collect();
    let mut out = b"KSLM".to_vec();
    out.extend([
        1,
        order as u8,
        LM_SCALE as u8,
        (lang.alphabet().len() + 2) as u8,
    ]);
    out.extend((alphabet.len() as u16).to_le_bytes());
    out.extend(alphabet.as_bytes());
    out.extend(table);
    out
}

// ---------------------------------------------------------------------------------------------
// Курируемые списки

fn data_rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').collect())
}

/// Буквенные клавиши US и «ЙЦУКЕН» на тех же позициях — одинаковы во всех трёх русских
/// раскладках (`analysis/detection.md` §7); `ё` стоит на `` ` `` («ПК», KBDRU) или на `\` (Apple).
const US_KEYS: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.";
const RU_KEYS: &str = "йцукенгшщзхъфывапролджэячсмитьбю";

fn ru_to_en(word: &str) -> Vec<String> {
    let mut variants = vec![String::new()];
    for c in word.chars() {
        let options: Vec<char> = match c {
            'ё' => vec!['`', '\\'],
            c => match RU_KEYS.chars().position(|r| r == c) {
                Some(p) => vec![US_KEYS.chars().nth(p).unwrap_or('?')],
                None => vec!['?'],
            },
        };
        variants = variants
            .iter()
            .flat_map(|v| options.iter().map(move |o| format!("{v}{o}")))
            .collect();
    }
    variants
}

fn en_to_ru(word: &str) -> String {
    word.chars()
        .map(|c| match US_KEYS.chars().position(|u| u == c) {
            Some(p) => RU_KEYS.chars().nth(p).unwrap_or('?'),
            None => '?',
        })
        .collect()
}

/// Аудит коротких слов: у слова вида `word` образ в другой раскладке не должен быть словом
/// другого списка, если пара не объявлена `ambiguous`; у `ambiguous` образ обязан быть в ru-списке.
fn short_words(curated: &str) -> Result<String> {
    let mut words: HashMap<Lang, BTreeSet<String>> = HashMap::new();
    let mut ambiguous = BTreeSet::new();
    let mut out = String::from(
        "# Сгенерировано ks-datagen из tools/ks-datagen/curated/short_words.tsv; не править руками.\n\
         # Колонки: язык, слово. Двухбуквенные — по RuSwitcher ShortWords.swift (MIT, Copyright (c) 2025 Rashns).\n",
    );
    let mut rows = Vec::new();
    for row in data_rows(curated) {
        let [lang, word, kind] = row.as_slice() else {
            bail!("short_words: ожидалось 3 колонки: {row:?}");
        };
        let lang = Lang::parse(lang)?;
        let len = word.chars().count();
        ensure!(
            (1..=2).contains(&len) && word.chars().all(|c| lang.is_letter(c) || c == 'ё'),
            "short_words: {word:?} — не 1–2 буквы языка"
        );
        ensure!(
            words.entry(lang).or_default().insert((*word).to_owned()),
            "short_words: повтор {word:?}"
        );
        match *kind {
            "word" => {}
            "ambiguous" if lang == Lang::En => {
                ambiguous.insert((*word).to_owned());
            }
            other => bail!("short_words: вид {other:?} у {word:?}"),
        }
        rows.push((lang, (*word).to_owned()));
    }
    let (en, ru) = (&words[&Lang::En], &words[&Lang::Ru]);
    let mut pairs = Vec::new();
    for word in en {
        let image = en_to_ru(word);
        let collides = ru.contains(&image);
        if ambiguous.contains(word) {
            ensure!(
                collides,
                "short_words: ambiguous {word:?} — образ {image:?} не в ru-списке"
            );
            pairs.push(format!("{word}↔{image}"));
        } else {
            ensure!(
                !collides,
                "short_words: коллизия {word:?}↔{image:?} не объявлена ambiguous"
            );
        }
    }
    for word in ru {
        for image in ru_to_en(word) {
            ensure!(
                !en.contains(&image) || ambiguous.contains(&image),
                "short_words: коллизия {word:?}↔{image:?} не объявлена ambiguous"
            );
        }
    }
    writeln!(
        out,
        "# Аудит коллизий пройден. Неоднозначные пары (в обоих списках, симметричный гейт не трогает их): {}",
        pairs.join(" ")
    )?;
    for (lang, word) in rows {
        writeln!(out, "{}\t{word}", lang.code())?;
    }
    println!(
        "короткие слова: en {}, ru {}, неоднозначных пар {}",
        en.len(),
        ru.len(),
        pairs.len()
    );
    Ok(out)
}

fn excluded_apps(curated: &str) -> Result<String> {
    let mut out = String::from(
        "# Сгенерировано ks-datagen из tools/ks-datagen/curated/excluded_apps.tsv; не править руками.\n\
         # Колонки: ОС, категория, идентификатор, сверка (installed — прочитан с установленного .app; unverified — не сверен),\n\
         # название в окне настроек, семейство (одна строка окна на несколько приложений; «-» — нет).\n",
    );
    let mut seen = HashSet::new();
    for row in data_rows(curated) {
        let [os, category, id, check, name, family] = row.as_slice() else {
            bail!("excluded_apps: ожидалось 6 колонок: {row:?}");
        };
        ensure!(
            matches!(*os, "macos" | "windows" | "linux"),
            "excluded_apps: ОС {os:?}"
        );
        ensure!(
            matches!(
                *category,
                "terminal" | "ide" | "editor" | "password" | "remote"
            ),
            "excluded_apps: категория {category:?}"
        );
        ensure!(
            matches!(*check, "installed" | "unverified"),
            "excluded_apps: сверка {check:?}"
        );
        ensure!(
            !id.is_empty() && !id.contains(char::is_whitespace),
            "excluded_apps: id {id:?}"
        );
        if *os == "windows" {
            ensure!(
                id.ends_with(".exe") && id.to_lowercase() == *id,
                "excluded_apps: Windows — exe в нижнем регистре: {id:?}"
            );
        }
        ensure!(
            !name.trim().is_empty() && !family.trim().is_empty(),
            "excluded_apps: у {id:?} нет названия или семейства («-» — нет семейства)"
        );
        ensure!(seen.insert((*os, *id)), "excluded_apps: повтор {id:?}");
        writeln!(out, "{os}\t{category}\t{id}\t{check}\t{name}\t{family}")?;
    }
    println!("исключённые приложения: {}", seen.len());
    Ok(out)
}

/// Таблицы раскладок пары (базовый уровень, код W3C → символ) — фикстуры задачи 01, по ним же
/// меряет `ks-eval`: US и все три русские (macOS «Русская», «Русская – ПК», Windows KBDRU).
struct Tables {
    us: HashMap<String, char>,
    ru: Vec<(&'static str, HashMap<String, char>)>,
}

impl Tables {
    fn fixtures() -> Tables {
        Tables {
            us: base_table(include_str!(
                "../../../crates/ks-core/tests/fixtures/us.tsv"
            )),
            ru: vec![
                (
                    "ru_apple",
                    base_table(include_str!(
                        "../../../crates/ks-core/tests/fixtures/ru_apple.tsv"
                    )),
                ),
                (
                    "ru_pc",
                    base_table(include_str!(
                        "../../../crates/ks-core/tests/fixtures/ru_pc.tsv"
                    )),
                ),
                (
                    "ru_win",
                    base_table(include_str!(
                        "../../../crates/ks-core/tests/fixtures/ru_win.tsv"
                    )),
                ),
            ],
        }
    }
}

/// Строки фикстуры `код\tBase\t…` без комментариев и `@`-полей.
fn base_table(text: &str) -> HashMap<String, char> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('@'))
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let key = cols.next()?;
            let mut base = cols.next()?.chars();
            let c = base.next()?;
            base.next().is_none().then(|| (key.to_owned(), c))
        })
        .collect()
}

/// Те же клавиши базового уровня в другой таблице; `None`, если символа нет.
fn keyed_image(
    word: &str,
    from: &HashMap<String, char>,
    to: &HashMap<String, char>,
) -> Option<String> {
    word.chars()
        .map(|c| {
            let key = from.iter().find(|(_, v)| **v == c)?.0;
            to.get(key).copied()
        })
        .collect()
}

/// Срез `adversarial.tsv` с защищёнными коллизиями (PRD история 12).
const COLLISION_SLICE: &str = "коллизии";

/// Токены защищённого среза. Пустой набор — ошибка: иначе проверка коллизий молча проходит.
fn protected_collisions(adversarial: &str) -> Result<HashSet<&str>> {
    let mut set = HashSet::new();
    for row in data_rows(adversarial) {
        let [slice, _lang, token] = row.as_slice() else {
            bail!("adversarial: ожидалось 3 колонки: {row:?}");
        };
        if *slice == COLLISION_SLICE {
            set.insert(*token);
        }
    }
    ensure!(
        !set.is_empty(),
        "weak_forms: в adversarial.tsv нет среза «{COLLISION_SLICE}» — защищённые коллизии не проверить"
    );
    Ok(set)
}

/// Аудит слабых форм: форма — словарное слово своего языка от трёх букв, образ — те же клавиши
/// в другой раскладке, один и тот же во всех трёх русских таблицах, и словарное слово другого
/// языка; ни форма, ни образ не стоят в защищённом срезе «коллизии» (`adversarial.tsv`, PRD
/// история 12) и не ослабляются в обе стороны; частота промаха на tune и причина указаны.
/// Возвращает файл и число форм RU и EN.
fn weak_forms(
    curated: &str,
    forms: &HashMap<Lang, BTreeSet<String>>,
    adversarial: &str,
    tables: &Tables,
) -> Result<(String, usize, usize)> {
    let protected = protected_collisions(adversarial)?;
    let mut out = String::from(
        "# Сгенерировано ks-datagen из tools/ks-datagen/curated/weak_forms.tsv; не править руками.\n\
         # Колонки: язык, форма, образ в другой раскладке. Словарное вето форму не защищает;\n\
         # обоснование каждой строки (промахи на tune, почему форма редкая) — в исходнике.\n",
    );
    let mut rows: Vec<(Lang, String, String)> = Vec::new();
    for row in data_rows(curated) {
        let [lang, form, pair, misses, why] = row.as_slice() else {
            bail!("weak_forms: ожидалось 5 колонок: {row:?}");
        };
        let lang = Lang::parse(lang)?;
        let other = match lang {
            Lang::Ru => Lang::En,
            Lang::En => Lang::Ru,
        };
        ensure!(
            form.chars().count() >= 3 && form.chars().all(|c| lang.is_letter(c)),
            "weak_forms: {form:?} — не слово от трёх букв языка"
        );
        ensure!(
            forms.get(&lang).is_some_and(|f| f.contains(*form)),
            "weak_forms: {form:?} нет в словаре — вето его и так не защищает"
        );
        // образ привязан к паре только если не зависит от русской раскладки
        for (name, ru) in &tables.ru {
            let image = match lang {
                Lang::Ru => keyed_image(form, ru, &tables.us),
                Lang::En => keyed_image(form, &tables.us, ru),
            };
            ensure!(
                image.as_deref() == Some(*pair),
                "weak_forms: образ {form:?} в {name} — {image:?}, а не {pair:?}: зависит от раскладки"
            );
        }
        ensure!(
            forms.get(&other).is_some_and(|f| f.contains(*pair)),
            "weak_forms: образ {pair:?} нет в словаре другого языка"
        );
        ensure!(
            !protected.contains(form) && !protected.contains(pair),
            "weak_forms: {form:?}↔{pair:?} — защищённая коллизия (adversarial.tsv)"
        );
        ensure!(
            misses.parse::<u32>().is_ok_and(|n| n > 0),
            "weak_forms: у {form:?} нет частоты промаха на tune"
        );
        ensure!(!why.trim().is_empty(), "weak_forms: у {form:?} нет причины");
        ensure!(
            !rows.iter().any(|(l, f, _)| *l == lang && f == form),
            "weak_forms: повтор {form:?}"
        );
        rows.push((lang, (*form).to_owned(), (*pair).to_owned()));
    }
    for (lang, form, pair) in &rows {
        ensure!(
            !rows.iter().any(|(l, f, _)| l != lang && f == pair),
            "weak_forms: {form:?}↔{pair:?} ослаблены в обе стороны"
        );
        writeln!(out, "{}\t{form}\t{pair}", lang.code())?;
    }
    let count = |lang: Lang| rows.iter().filter(|(l, ..)| *l == lang).count();
    println!(
        "слабые формы: ru {}, en {}",
        count(Lang::Ru),
        count(Lang::En)
    );
    Ok((out, count(Lang::Ru), count(Lang::En)))
}

fn adversarial(curated: &str) -> Result<String> {
    let mut out = String::from(
        "# Сгенерировано ks-datagen из tools/ks-datagen/curated/adversarial.tsv; составлено вручную.\n\
         # Колонки: срез, язык (раскладка, в которой токен набран правильно), токен.\n",
    );
    let mut n = 0;
    for row in data_rows(curated) {
        let [slice, lang, token] = row.as_slice() else {
            bail!("adversarial: ожидалось 3 колонки: {row:?}");
        };
        Lang::parse(lang)?;
        ensure!(
            !token.is_empty() && !token.contains(char::is_whitespace),
            "adversarial: токен {token:?}"
        );
        writeln!(out, "{slice}\t{lang}\t{token}")?;
        n += 1;
    }
    println!("состязательные токены: {n}");
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Корпус оценки (Tatoeba)

struct Sample {
    text: String,
    eligible: usize,
    picked: usize,
    export: String,
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn is_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Живое предложение языка: 3–30 токенов; RU — кириллица, латиницы не больше пятой части букв;
/// EN — только ASCII-латиница.
fn eligible_sentence(lang: Lang, text: &str) -> bool {
    let tokens = text.split_whitespace().count();
    if !(3..=30).contains(&tokens) {
        return false;
    }
    let (mut cyr, mut lat, mut other) = (0usize, 0usize, 0usize);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        if is_cyrillic(c) {
            cyr += 1;
        } else if c.is_ascii_alphabetic() {
            lat += 1;
        } else {
            other += 1;
        }
    }
    match lang {
        Lang::Ru => cyr > 0 && other == 0 && lat * 5 <= cyr,
        Lang::En => lat > 0 && cyr == 0 && other == 0,
    }
}

fn sample_tatoeba(
    cache: &Path,
    lang: Lang,
    code3: &str,
    share: u64,
    refresh: bool,
) -> Result<Sample> {
    let file = format!("{code3}_sentences.tsv.bz2");
    let path = fetch(cache, &format!("{TATOEBA}/{code3}/{file}"), refresh)?;
    let export = fs::read_to_string(cache.join(format!("{file}.last-modified")))
        .unwrap_or_else(|_| "неизвестно".to_owned());
    let mut child = Command::new("bunzip2")
        .arg("-c")
        .arg(&path)
        .stdout(Stdio::piped())
        .spawn()
        .context("нужен bunzip2 в PATH")?;
    let stdout = child.stdout.take().context("нет stdout у bunzip2")?;
    let mut picked = Vec::new();
    let mut eligible = 0usize;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        let mut cols = line.splitn(3, '\t');
        let (Some(id), Some(_), Some(text)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let Ok(id) = id.parse::<u64>() else {
            continue;
        };
        if !eligible_sentence(lang, text) {
            continue;
        }
        eligible += 1;
        if splitmix64(id ^ EVAL_SALT) % 10_000 < share {
            picked.push((id, text.to_owned()));
        }
    }
    ensure!(child.wait()?.success(), "bunzip2 завершился с ошибкой");
    picked.sort();
    let mut text = format!(
        "# Tatoeba (https://tatoeba.org), предложения {code3}, выгрузка {export}.\n\
         # Лицензия CC BY 2.0 FR (https://creativecommons.org/licenses/by/2.0/fr/); автор предложения — по id:\n\
         # https://tatoeba.org/sentences/show/<id>. Текст не изменён; отбор — splitmix64(id ^ соль) % 10000 < {share}, 3–30 токенов.\n\
         # Отложенный корпус: только для ks-eval, в словари и обучение LM не входит.\n"
    );
    for (id, sentence) in &picked {
        writeln!(text, "{id}\t{sentence}")?;
    }
    Ok(Sample {
        text,
        eligible,
        picked: picked.len(),
        export: export.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Tables {
        let us = "KeyT\tt\nKeyH\th\nKeyE\te\nKeyR\tr\n";
        let ru = "KeyT\tе\nKeyH\tр\nKeyE\tу\nKeyR\tк\n";
        Tables {
            us: base_table(us),
            ru: vec![
                ("a", base_table(ru)),
                ("b", base_table(ru)),
                ("c", base_table(ru)),
            ],
        }
    }

    fn forms() -> HashMap<Lang, BTreeSet<String>> {
        let set = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect();
        HashMap::from([
            (Lang::Ru, set(&["еру", "руку"])),
            (Lang::En, set(&["the", "here"])),
        ])
    }

    const ADVERSARIAL: &str = "коллизии\tru\tруку\nколлизии\ten\there\n";

    #[test]
    fn rare_form_with_a_fixed_image_passes() {
        let (out, ru, en) = weak_forms(
            "ru\tеру\tthe\t2455\tимя буквы\n",
            &forms(),
            ADVERSARIAL,
            &tables(),
        )
        .unwrap();
        assert!(out.contains("ru\tеру\tthe\n"));
        assert_eq!((ru, en), (1, 0));
    }

    #[test]
    fn protected_collision_is_rejected() {
        let err = weak_forms(
            "ru\tруку\there\t158\tчастая\n",
            &forms(),
            ADVERSARIAL,
            &tables(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("защищённая коллизия"), "{err}");
    }

    #[test]
    fn missing_collision_slice_fails_instead_of_passing_silently() {
        for adversarial in ["", "коллизия\tru\tруку\n", "код\ten\tfoo_bar\n"] {
            let err =
                weak_forms("ru\tеру\tthe\t1\tx\n", &forms(), adversarial, &tables()).unwrap_err();
            assert!(
                err.to_string().contains("нет среза"),
                "{adversarial:?}: {err}"
            );
        }
    }

    #[test]
    fn layout_dependent_image_is_rejected() {
        // в таблице «c» буква «у» на KeyR, а не на KeyE: образ «еру» там — «thr», а не «the»
        let mut t = tables();
        t.ru[2].1.remove("KeyE");
        t.ru[2].1.insert("KeyR".into(), 'у');
        let err = weak_forms("ru\tеру\tthe\t1\tx\n", &forms(), ADVERSARIAL, &t).unwrap_err();
        assert!(err.to_string().contains("зависит от раскладки"), "{err}");
    }
}
