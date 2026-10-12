//! Раскладки Linux по именам XKB: разбор `_XKB_RULES_NAMES`, списков окружений и каталога
//! `evdev.lst` (описание «Russian (Windows)» → `ru(winkeys)`): Sway и Hyprland отдают активную
//! раскладку описанием, а таблицы строятся по коду. Без системных вызовов, кроме чтения
//! каталога с диска.

use ks_core::{Lang, LayoutId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutSpec {
    /// Код XKB: `us`, `ru`; пусто — код неизвестен (есть только описание).
    pub layout: String,
    /// Вариант XKB: `winkeys`; пусто — основной.
    pub variant: String,
    /// Описание XKB: `English (US)`; пусто — неизвестно.
    pub description: String,
}

impl LayoutSpec {
    pub fn new(layout: &str, variant: &str) -> LayoutSpec {
        LayoutSpec {
            layout: layout.trim().to_owned(),
            variant: variant.trim().to_owned(),
            description: String::new(),
        }
    }

    /// `us`, `ru(winkeys)`; без кода — описание.
    pub fn id(&self) -> LayoutId {
        LayoutId(match (self.layout.is_empty(), self.variant.is_empty()) {
            (true, _) => self.description.clone(),
            (false, true) => self.layout.clone(),
            (false, false) => format!("{}({})", self.layout, self.variant),
        })
    }

    pub fn lang(&self) -> Lang {
        match self.layout.as_str() {
            "ru" => Lang::Ru,
            "us" | "gb" => Lang::En,
            _ => Lang::Other,
        }
    }

    /// Имя для меню и `--status`.
    pub fn label(&self) -> String {
        if self.description.is_empty() {
            self.id().0
        } else {
            self.description.clone()
        }
    }
}

/// Коды и варианты через запятую («us,ru» и «,winkeys») — пары по позиции.
pub fn specs_from_lists(layouts: &str, variants: &str) -> Vec<LayoutSpec> {
    let variants: Vec<&str> = variants.split(',').collect();
    layouts
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, l)| LayoutSpec::new(l, variants.get(i).copied().unwrap_or("")))
        .collect()
}

/// Свойство `_XKB_RULES_NAMES` корневого окна X11: `rules\0model\0layout\0variant\0options\0`.
pub fn parse_rules_names(raw: &[u8]) -> Vec<LayoutSpec> {
    let text = String::from_utf8_lossy(raw);
    let mut parts = text.split('\0');
    let (_rules, _model) = (parts.next(), parts.next());
    let layouts = parts.next().unwrap_or("");
    let variants = parts.next().unwrap_or("");
    specs_from_lists(layouts, variants)
}

/// Позиция раскладки в списке окружения — это и есть индекс для команды переключения.
pub fn index_of(specs: &[LayoutSpec], id: &LayoutId) -> Option<usize> {
    specs.iter().position(|s| s.id() == *id)
}

/// Каталог XKB «описание ↔ код и вариант».
#[derive(Clone, Debug, Default)]
pub struct XkbCatalog {
    entries: Vec<LayoutSpec>,
}

const BUILTIN: &[(&str, &str, &str)] = &[
    ("us", "", "English (US)"),
    ("gb", "", "English (UK)"),
    ("ru", "", "Russian"),
    ("ru", "winkeys", "Russian (Windows)"),
    ("ru", "typewriter", "Russian (typewriter)"),
    ("ru", "phonetic", "Russian (phonetic)"),
];

impl XkbCatalog {
    pub const SYSTEM_PATHS: [&'static str; 2] = [
        "/usr/share/X11/xkb/rules/evdev.lst",
        "/usr/share/X11/xkb/rules/base.lst",
    ];

    /// Разбор `evdev.lst`: секции `! layout` («us  English (US)») и `! variant`
    /// («winkeys  ru: Russian (Windows)»).
    pub fn parse(text: &str) -> XkbCatalog {
        let mut entries = Vec::new();
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('!') {
                section = match name.trim() {
                    "layout" => "layout",
                    "variant" => "variant",
                    _ => "",
                };
                continue;
            }
            let Some((name, rest)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            let rest = rest.trim();
            match section {
                "layout" => entries.push(LayoutSpec {
                    layout: name.to_owned(),
                    variant: String::new(),
                    description: rest.to_owned(),
                }),
                "variant" => {
                    if let Some((layout, description)) = rest.split_once(": ") {
                        entries.push(LayoutSpec {
                            layout: layout.trim().to_owned(),
                            variant: name.to_owned(),
                            description: description.trim().to_owned(),
                        });
                    }
                }
                _ => {}
            }
        }
        XkbCatalog { entries }
    }

    /// Минимальный каталог на случай, если системного нет.
    pub fn builtin() -> XkbCatalog {
        XkbCatalog {
            entries: BUILTIN
                .iter()
                .map(|(l, v, d)| LayoutSpec {
                    layout: (*l).to_owned(),
                    variant: (*v).to_owned(),
                    description: (*d).to_owned(),
                })
                .collect(),
        }
    }

    /// Системный каталог (первый читаемый из `SYSTEM_PATHS`) плюс встроенный.
    pub fn system() -> XkbCatalog {
        let mut catalog = XkbCatalog::SYSTEM_PATHS
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .map(|t| XkbCatalog::parse(&t))
            .unwrap_or_default();
        catalog.entries.extend(XkbCatalog::builtin().entries);
        catalog
    }

    /// Код и вариант по описанию; неизвестное описание — `None`.
    pub fn by_description(&self, description: &str) -> Option<LayoutSpec> {
        let description = description.trim();
        self.entries
            .iter()
            .find(|e| e.description == description)
            .cloned()
    }

    /// Описание для кода и варианта; `spec` с уже заполненным описанием не меняется.
    pub fn describe(&self, mut spec: LayoutSpec) -> LayoutSpec {
        if spec.description.is_empty()
            && let Some(e) = self
                .entries
                .iter()
                .find(|e| e.layout == spec.layout && e.variant == spec.variant)
        {
            spec.description = e.description.clone();
        }
        spec
    }

    /// Описание → раскладка; неизвестное описание — раскладка без кода.
    pub fn resolve(&self, description: &str) -> LayoutSpec {
        self.by_description(description).unwrap_or(LayoutSpec {
            description: description.trim().to_owned(),
            ..LayoutSpec::default()
        })
    }
}
