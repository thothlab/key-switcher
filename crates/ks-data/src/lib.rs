//! Данные KeySwitcher: словари EN и RU, языковые модели, короткие слова,
//! встроенный список приложений, где автоматика выключена.
//!
//! Сейчас словари — временные списки основ hunspell без развёртки форм
//! (`data/stems_*.txt`); полные словари и LM приносит задача 03.
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

const STEMS_EN: &str = include_str!("../../../data/stems_en.txt");
const STEMS_RU_GZ: &[u8] = include_bytes!("../../../data/stems_ru.txt.gz");

/// Терминалы и IDE: там набирают команды и код, а не прозу. TODO(task 03): уточнить состав.
const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "com.apple.dt.Xcode",
    "com.microsoft.VSCode",
    "com.jetbrains.intellij",
    "com.jetbrains.pycharm",
    "windowsterminal.exe",
    "code.exe",
    "idea64.exe",
    "org.gnome.Terminal",
    "org.kde.konsole",
    "code",
    "jetbrains-idea",
];

pub struct Lexicon {
    en: HashSet<String>,
    ru: HashSet<String>,
    no_words: HashSet<String>,
    excluded_apps: Vec<AppId>,
}

impl Lexicon {
    /// Данные, встроенные в бинарник; распаковываются один раз на процесс.
    pub fn embedded() -> Arc<Lexicon> {
        static EMBEDDED: OnceLock<Arc<Lexicon>> = OnceLock::new();
        Arc::clone(EMBEDDED.get_or_init(|| Arc::new(Lexicon::load_embedded())))
    }

    fn load_embedded() -> Lexicon {
        let mut ru = String::new();
        flate2::read::GzDecoder::new(STEMS_RU_GZ)
            .read_to_string(&mut ru)
            .expect("встроенный словарь RU повреждён");
        Lexicon {
            en: word_set(STEMS_EN),
            ru: word_set(&ru),
            no_words: HashSet::new(),
            excluded_apps: DEFAULT_EXCLUDED_APPS
                .iter()
                .map(|id| AppId((*id).to_owned()))
                .collect(),
        }
    }

    /// Есть ли слово в словаре языка; `ё` и `е` не различаются.
    pub fn contains(&self, lang: Lang, word_lower: &str) -> bool {
        let word = normalize(word_lower);
        match lang {
            Lang::En => self.en.contains(word.as_ref()),
            Lang::Ru => self.ru.contains(word.as_ref()),
            Lang::Other => false,
        }
    }

    /// Средний ln P на n-грамму с маркерами границ.
    /// TODO(task 03): символьная n-граммная модель; пока 0.0 для любого слова.
    pub fn lm_score(&self, _lang: Lang, _word_lower: &str) -> f32 {
        0.0
    }

    /// Частые слова длины `len` ∈ {1, 2}. TODO(task 03): списки; пока пустой набор.
    pub fn short_words(&self, _lang: Lang, _len: usize) -> &HashSet<String> {
        &self.no_words
    }

    pub fn default_excluded_apps(&self) -> &[AppId] {
        &self.excluded_apps
    }
}

fn word_set(text: &str) -> HashSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|word| {
            word.chars()
                .all(|c| c.is_alphabetic() || c == '-' || c == '\'')
        })
        .map(|word| normalize(&word.to_lowercase()).into_owned())
        .collect()
}

fn normalize(word: &str) -> Cow<'_, str> {
    if word.contains(['ё', 'Ё']) {
        Cow::Owned(word.replace(['ё', 'Ё'], "е"))
    } else {
        Cow::Borrowed(word)
    }
}
