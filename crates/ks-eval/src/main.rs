//! `ks-eval [КОРПУС] [--part tune|control|all] [--limit N] [--write ФАЙЛ]` — замер качества.
//!
//! КОРПУС — каталог с `tatoeba_ru.tsv`, `tatoeba_en.tsv`, `adversarial.tsv` (по умолчанию
//! `data/eval/`). `--part` — часть корпуса по хэшу id (по умолчанию `control`; `tune` — для
//! подбора порогов). `--limit N` — первые N предложений каждого языка в части. `--write ФАЙЛ` —
//! заменить в файле блок между `<!-- ks-eval:begin -->` и `<!-- ks-eval:end -->` отчётом.
//! Если связка с движком не работает (`smoke_check`), замер отменяется с ненулевым кодом выхода
//! и файл не трогается.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use ks_core::{Config, Lang, Lexicon, Os};
use ks_eval::{Layouts, Part, RecallWord, evaluate, load_corpus, recall_words, smoke_check};

const BEGIN: &str = "<!-- ks-eval:begin -->";
const END: &str = "<!-- ks-eval:end -->";

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dir = root.join("data/eval");
    let (mut part, mut limit, mut write) = (Part::Control, usize::MAX, None::<PathBuf>);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--part" => {
                let name = args.next().context("--part tune|control|all")?;
                part = Part::parse(&name)
                    .with_context(|| format!("--part {name}: есть tune, control, all"))?;
            }
            "--limit" => limit = args.next().context("--limit N")?.parse()?,
            "--write" => write = Some(args.next().context("--write ФАЙЛ")?.into()),
            flag if flag.starts_with("--") => {
                bail!("неизвестный флаг {flag}; есть --part, --limit N, --write ФАЙЛ")
            }
            path => dir = path.into(),
        }
    }

    let started = Instant::now();
    let lexicon = Lexicon::embedded();
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let layouts = Layouts::fixtures();
    let config = Config::default_for(Os::MacOs);
    if let Err(why) = smoke_check(&lexicon, &layouts, &config) {
        bail!("связка с движком не работает: {why}; замер отменён, отчёт не записан");
    }

    let corpus = load_corpus(&dir, part, limit)?;
    let tally = evaluate(&lexicon, &layouts, &corpus, &config);
    let ru_words = recall_words(&corpus.ru, Lang::Ru, &lexicon);
    let en_words = recall_words(&corpus.en, Lang::En, &lexicon);

    let (worst_fp, worst_recall) = tally.headline();
    let oov = |w: &[RecallWord]| {
        w.iter().filter(|w| !w.in_dict).count() as f64 * 100.0 / w.len().max(1) as f64
    };
    let mut report = String::new();
    writeln!(
        report,
        "Часть корпуса: **{}** (половина `data/eval` по хэшу id предложения) — {} предложений RU, {} EN; \
         слов для полноты — {} RU ({:.1}% вне словаря), {} EN ({:.1}% вне словаря).  ",
        part.label(),
        corpus.ru.len(),
        corpus.en.len(),
        ru_words.len(),
        oov(&ru_words),
        en_words.len(),
        oov(&en_words)
    )?;
    writeln!(
        report,
        "Загрузка `Lexicon::embedded()`: {load_ms:.0} мс; данные: {}.  ",
        data_sizes(&root.join("data"))
    )?;
    writeln!(
        report,
        "Связка с движком проверена: «ghbdtn» → «привет», «Ghbdtn» → «Привет», «привет» не тронут. \
         Время замера: {:.0} с.\n",
        started.elapsed().as_secs_f64()
    )?;
    writeln!(
        report,
        "**Итог:** худший срез живого текста — {worst_fp:.1} FP на 10 000 токенов (цель ≤ 10); \
         худшая полнота по словарным словам — {:.1}% (цель ≥ 97%).\n",
        worst_recall * 100.0
    )?;
    report.push_str(&tally.render_markdown());
    println!("{report}");

    if let Some(path) = write {
        let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
        let (Some(b), Some(e)) = (text.find(BEGIN), text.find(END)) else {
            bail!("в {} нет меток {BEGIN} … {END}", path.display());
        };
        let updated = format!("{}{BEGIN}\n{report}\n{}", &text[..b], &text[e..]);
        std::fs::write(&path, updated)?;
        eprintln!("отчёт записан в {}", path.display());
    }
    Ok(())
}

fn data_sizes(data: &Path) -> String {
    let size = |name: &str| std::fs::metadata(data.join(name)).map_or(0, |m| m.len());
    let files = [
        "words_ru.fst.gz",
        "words_en.fst.gz",
        "lm_ru.bin.gz",
        "lm_en.bin.gz",
    ];
    let total: u64 = files.iter().map(|f| size(f)).sum();
    let parts: Vec<String> = files
        .iter()
        .map(|f| format!("`{f}` {} КБ", size(f) / 1024))
        .collect();
    format!("{} (всего {} КБ)", parts.join(", "), total / 1024)
}
