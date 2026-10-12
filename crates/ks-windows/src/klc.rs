//! Разбор таблицы раскладки в формате KLC (Microsoft Keyboard Layout Creator; такие файлы
//! публикует kbdlayout.info по `kbd*.dll`): секции `SHIFTSTATE` и `LAYOUT`. Нужен, чтобы
//! сверить построение таблицы (`layouts::build_layout`) с первоисточником без живой Windows:
//! [`Klc::translate`] отвечает так же, как `ToUnicodeEx` на уровнях Base/Shift/Caps/Shift+Caps.

use ks_core::Level;

/// Строка секции `LAYOUT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KlcRow {
    /// Скан-код (без префикса E0: в KLC печатающие клавиши его не имеют).
    pub scan: u16,
    /// Имя VK из KLC: `A`, `1`, `OEM_3`, `SPACE`, `DECIMAL`…
    pub vk: String,
    /// Колонка `Cap`: бит 1 — CapsLock действует как Shift (`CAPLOK`).
    pub cap: u8,
    /// Символы по колонкам `SHIFTSTATE`; `None` — нет символа, мёртвая клавиша или лигатура.
    pub values: Vec<Option<char>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Klc {
    /// `LOCALEID` — KLID раскладки (`00000419`).
    pub locale_id: String,
    /// `SHIFTSTATE` по порядку колонок: 0 — без модификаторов, 1 — Shift, 2 — Ctrl, 6 — AltGr…
    pub shift_states: Vec<u8>,
    pub rows: Vec<KlcRow>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Head,
    ShiftState,
    Layout,
    Rest,
}

fn value(token: &str) -> Option<char> {
    if token == "-1" || token == "%%" || token.ends_with('@') {
        return None;
    }
    let mut chars = token.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        return Some(ch);
    }
    char::from_u32(u32::from_str_radix(token, 16).ok()?)
}

impl Klc {
    /// Разбор текста KLC (UTF-8). Строки, которые не похожи на данные, пропускаются.
    pub fn parse(text: &str) -> Klc {
        let mut klc = Klc::default();
        let mut section = Section::Head;
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            let mut tokens = line.split_whitespace();
            let Some(first) = tokens.next() else {
                continue;
            };
            match first {
                "LOCALEID" => {
                    klc.locale_id = tokens.next().unwrap_or("").trim_matches('"').to_owned();
                    continue;
                }
                "SHIFTSTATE" => {
                    section = Section::ShiftState;
                    continue;
                }
                "LAYOUT" => {
                    section = Section::Layout;
                    continue;
                }
                _ if first.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                    && first.len() > 3 =>
                {
                    section = Section::Rest;
                    continue;
                }
                _ => {}
            }
            match section {
                Section::ShiftState => {
                    if let Ok(state) = first.parse() {
                        klc.shift_states.push(state);
                    }
                }
                Section::Layout => {
                    // Продолжение SGCap начинается с `-1` вместо скан-кода — пропускается.
                    let (Ok(scan), Some(vk), Some(cap)) =
                        (u16::from_str_radix(first, 16), tokens.next(), tokens.next())
                    else {
                        continue;
                    };
                    klc.rows.push(KlcRow {
                        scan,
                        vk: vk.to_owned(),
                        cap: if cap == "SGCap" {
                            2
                        } else {
                            cap.parse().unwrap_or(0)
                        },
                        values: tokens.map(value).collect(),
                    });
                }
                Section::Head | Section::Rest => {}
            }
        }
        klc
    }

    pub fn row(&self, scan: u16) -> Option<&KlcRow> {
        self.rows.iter().find(|r| r.scan == scan)
    }

    fn column(&self, row: &KlcRow, state: u8) -> Option<char> {
        let i = self.shift_states.iter().position(|s| *s == state)?;
        row.values.get(i).copied().flatten()
    }

    /// Символ клавиши со скан-кодом `scan` на уровне — как `ToUnicodeEx`: CapsLock меняет
    /// Base↔Shift только у клавиш с `CAPLOK`; управляющие символы не печатаются.
    pub fn translate(&self, scan: u16, level: Level) -> Option<char> {
        let row = self.row(scan)?;
        let caplok = row.cap & 1 != 0;
        let shifted = match level {
            Level::Base => false,
            Level::Shift => true,
            Level::Caps => caplok,
            Level::ShiftCaps => !caplok,
        };
        let ch = self.column(row, u8::from(shifted))?;
        ((ch as u32) >= 0x20 && ch != '\u{7f}').then_some(ch)
    }
}

/// Имя VK из KLC → код `VK_*`.
pub fn vk_code(name: &str) -> Option<u16> {
    let mut chars = name.chars();
    if let (Some(ch), None) = (chars.next(), chars.next())
        && (ch.is_ascii_digit() || ch.is_ascii_uppercase())
    {
        return Some(ch as u16);
    }
    Some(match name {
        "SPACE" => 0x20,
        "DECIMAL" => 0x6E,
        "OEM_1" => 0xBA,
        "OEM_PLUS" => 0xBB,
        "OEM_COMMA" => 0xBC,
        "OEM_MINUS" => 0xBD,
        "OEM_PERIOD" => 0xBE,
        "OEM_2" => 0xBF,
        "OEM_3" => 0xC0,
        "OEM_4" => 0xDB,
        "OEM_5" => 0xDC,
        "OEM_6" => 0xDD,
        "OEM_7" => 0xDE,
        "OEM_8" => 0xDF,
        "OEM_102" => 0xE2,
        _ => return None,
    })
}
