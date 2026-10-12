//! Данные KeySwitcher: словари EN и RU, символьные языковые модели, короткие слова,
//! встроенный список приложений, где автоматика выключена.
//!
//! Всё генерирует `tools/ks-datagen` в `data/` из hunspell `en_US`/`ru_RU` и курируемых списков;
//! здесь файлы встраиваются в бинарник и распаковываются один раз на процесс. Формат файлов —
//! внутреннее дело крейта:
//! - `words_{en,ru}.fst.gz` — `fst::Set` словоформ (нижний регистр, `ё→е`), gzip;
//! - `lm_{en,ru}.bin.gz` — символьная n-граммная модель, gzip: `KSLM`, версия 1, порядок,
//!   квант, размер алфавита со служебными символами (0 — граница, 1 — символ вне алфавита),
//!   длина алфавита в байтах (`u16` LE), алфавит UTF-8, плотная таблица квантованных `−ln P`;
//! - `short_words.tsv` — `язык\tслово`;
//! - `excluded_apps.tsv` — `ОС\tкатегория\tидентификатор\tсверка`; встраиваются все ОС,
//!   в [`Lexicon`] остаются строки текущей;
//! - `weak_forms.tsv` — `язык\tформа\tобраз`: редкие словарные формы, которые не защищены
//!   словарным вето (курируемый список, обоснования — в `tools/ks-datagen/curated/weak_forms.tsv`).
//!
//! Здесь же живут `Lang` и `AppId`: их реэкспортирует `ks-core`, который зависит от этого крейта.

use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Read;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Ru,
    Other,
}

/// bundle id (macOS), имя exe в нижнем регистре (Windows), app_id/WM_CLASS (Linux).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppId(pub String);

/// Оценка [`Lexicon::lm_score`] для языка без модели и нижняя граница любой оценки (в натах).
pub const LM_MIN_SCORE: f32 = -25.5;

/// Символ, допустимый в слове: буква, дефис, апостроф. Единое определение для ядра.
pub fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '-' || c == '\''
}

const WORDS_EN: &[u8] = include_bytes!("../../../data/words_en.fst.gz");
const WORDS_RU: &[u8] = include_bytes!("../../../data/words_ru.fst.gz");
const LM_EN: &[u8] = include_bytes!("../../../data/lm_en.bin.gz");
const LM_RU: &[u8] = include_bytes!("../../../data/lm_ru.bin.gz");
const SHORT_WORDS: &str = include_str!("../../../data/short_words.tsv");
const EXCLUDED_APPS: &str = include_str!("../../../data/excluded_apps.tsv");
const WEAK_FORMS: &str = include_str!("../../../data/weak_forms.tsv");

/// Код ОС сборки — первая колонка `excluded_apps.tsv`; та же цепочка, что `Os::current()` в ядре.
const CURRENT_OS: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

struct LangData {
    words: fst::Set<Vec<u8>>,
    lm: CharLm,
    /// Короткие слова длины 1 и 2.
    short: [HashSet<String>; 2],
    /// Слабые словарные формы (нижний регистр, `ё→е`).
    weak: HashSet<String>,
}

pub struct Lexicon {
    en: LangData,
    ru: LangData,
    empty: HashSet<String>,
    excluded_apps: Vec<AppId>,
}

impl Lexicon {
    /// Данные, встроенные в бинарник; распаковываются один раз на процесс.
    pub fn embedded() -> Arc<Lexicon> {
        static EMBEDDED: OnceLock<Arc<Lexicon>> = OnceLock::new();
        Arc::clone(EMBEDDED.get_or_init(|| Arc::new(Lexicon::load_embedded())))
    }

    fn load_embedded() -> Lexicon {
        let lang = |code: &str, words: &[u8], lm: &[u8]| LangData {
            words: fst::Set::new(gunzip(words, code))
                .unwrap_or_else(|e| panic!("встроенный словарь {code} повреждён: {e}")),
            lm: CharLm::decode(&gunzip(lm, code))
                .unwrap_or_else(|| panic!("встроенная LM {code} повреждена")),
            short: short_words(code),
            weak: rows(WEAK_FORMS)
                .filter_map(|row| row.strip_prefix(code)?.strip_prefix('\t'))
                .filter_map(|rest| rest.split('\t').next())
                .map(|form| normalize(form).into_owned())
                .collect(),
        };
        Lexicon {
            en: lang("en", WORDS_EN, LM_EN),
            ru: lang("ru", WORDS_RU, LM_RU),
            empty: HashSet::new(),
            excluded_apps: rows(EXCLUDED_APPS)
                .filter_map(|row| {
                    let mut cols = row.split('\t');
                    let os = cols.next()?;
                    (os == CURRENT_OS).then(|| cols.nth(1)).flatten()
                })
                .map(|id| AppId(id.to_owned()))
                .collect(),
        }
    }

    fn data(&self, lang: Lang) -> Option<&LangData> {
        match lang {
            Lang::En => Some(&self.en),
            Lang::Ru => Some(&self.ru),
            Lang::Other => None,
        }
    }

    /// Есть ли словоформа в словаре языка; `ё` и `е` не различаются.
    pub fn contains(&self, lang: Lang, word_lower: &str) -> bool {
        self.data(lang)
            .is_some_and(|d| d.words.contains(normalize(word_lower).as_bytes()))
    }

    /// Средний `ln P` на n-грамму (порядок по умолчанию 3) с маркерами границ слова: чем ближе
    /// к нулю, тем правдоподобнее строка для языка. Символ вне алфавита в знакомом
    /// контексте упирается в нижнюю границу (`−25,5`), n-граммы после него стоят около
    /// `−14…−17` (замер на `ghbdtn`, `пщщв`); для [`Lang::Other`] — [`LM_MIN_SCORE`]. Без аллокаций.
    pub fn lm_score(&self, lang: Lang, word_lower: &str) -> f32 {
        self.data(lang)
            .map_or(LM_MIN_SCORE, |d| d.lm.score(word_lower))
    }

    /// Частые слова длины `len` ∈ {1, 2}, нижний регистр; «её» есть в обоих написаниях.
    /// В en-списке есть неоднозначные токены (`vs`, `dj`, `kb`, `jr`, `bp`, `ye`, `ds`), чей образ
    /// в другой раскладке — частое ru-слово: они рассчитаны на симметричный гейт («набранное не
    /// в своём списке, а конверсия — в списке цели») и не дают конвертировать пару ни в одну сторону.
    pub fn short_words(&self, lang: Lang, len: usize) -> &HashSet<String> {
        match (self.data(lang), len) {
            (Some(d), 1 | 2) => &d.short[len - 1],
            _ => &self.empty,
        }
    }

    /// Слабая словарная форма: есть в словаре языка, но сама редкая, а её образ в другой
    /// раскладке — частое слово («еру» = «the»). Словарное вето такую форму не защищает;
    /// `ё` и `е` не различаются.
    pub fn is_weak(&self, lang: Lang, word_lower: &str) -> bool {
        self.data(lang)
            .is_some_and(|d| d.weak.contains(normalize(word_lower).as_ref()))
    }

    /// Встроенный список исключённых приложений текущей ОС: терминалы, IDE и редакторы кода,
    /// менеджеры паролей, удалённые рабочие столы.
    pub fn default_excluded_apps(&self) -> &[AppId] {
        &self.excluded_apps
    }
}

/// Символьная n-граммная модель: плотная таблица `−ln P(c | h)` в квантах `1/scale` ната.
struct CharLm {
    order: usize,
    scale: f32,
    v: usize,
    alphabet: Vec<char>,
    table: Vec<u8>,
}

impl CharLm {
    fn decode(bytes: &[u8]) -> Option<CharLm> {
        let rest = bytes.strip_prefix(b"KSLM")?;
        let ([version, order, scale, v], rest) = rest.split_first_chunk::<4>()?;
        let (len, rest) = rest.split_first_chunk::<2>()?;
        let len = usize::from(u16::from_le_bytes(*len));
        let alphabet: Vec<char> = std::str::from_utf8(rest.get(..len)?)
            .ok()?
            .chars()
            .collect();
        let table = rest.get(len..)?.to_vec();
        let (order, v) = (usize::from(*order), usize::from(*v));
        let valid = *version == 1
            && order >= 2
            && *scale > 0
            && alphabet.len() + 2 == v
            && v.checked_pow(order as u32) == Some(table.len());
        valid.then_some(CharLm {
            order,
            scale: f32::from(*scale),
            v,
            alphabet,
            table,
        })
    }

    fn symbol(&self, c: char) -> usize {
        let c = match c {
            'ё' | 'Ё' => 'е',
            c if c.is_uppercase() => c.to_lowercase().next().unwrap_or(c),
            c => c,
        };
        self.alphabet
            .iter()
            .position(|&a| a == c)
            .map_or(1, |p| p + 2)
    }

    /// Контекст начинается с `order−1` маркеров границы (символ 0), в конце — ещё один маркер.
    fn score(&self, word: &str) -> f32 {
        let contexts = self.v.pow(self.order as u32 - 1);
        let (mut ctx, mut sum, mut count) = (0usize, 0u32, 0u32);
        for s in word
            .chars()
            .map(|c| self.symbol(c))
            .chain(std::iter::once(0))
        {
            let idx = ctx * self.v + s;
            sum += u32::from(self.table[idx]);
            count += 1;
            ctx = idx % contexts;
        }
        -(sum as f32) / (self.scale * count as f32)
    }
}

fn gunzip(bytes: &[u8], what: &str) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap_or_else(|e| panic!("встроенные данные {what} повреждены: {e}"));
    out
}

fn rows(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

fn short_words(code: &str) -> [HashSet<String>; 2] {
    let mut out = [HashSet::new(), HashSet::new()];
    for (lang, word) in rows(SHORT_WORDS).filter_map(|row| row.split_once('\t')) {
        let len = word.chars().count();
        if lang == code && (1..=2).contains(&len) {
            out[len - 1].insert(word.to_owned());
        }
    }
    out
}

fn normalize(word: &str) -> Cow<'_, str> {
    if word.contains(['ё', 'Ё']) {
        Cow::Owned(word.replace(['ё', 'Ё'], "е"))
    } else {
        Cow::Borrowed(word)
    }
}
