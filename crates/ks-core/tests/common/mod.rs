//! Общие помощники тестов ядра: загрузка фикстур раскладок и набор текста нажатиями.
//!
//! Формат фикстуры (`tests/fixtures/*.tsv`): строки `#` — комментарии; `@id`, `@lang`, `@name`
//! через табуляцию — заголовок; остальное — `код W3C\tBase\tShift\tCaps\tShift+Caps`, пусто — нет символа.
#![allow(dead_code)]

use std::path::Path;

use ks_core::{
    Action, AppId, Config, Context, Disposition, Engine, KeyDir, KeyEvent, KeyStroke, Lang,
    LayoutId, LayoutMap, LayoutSet, Level, Lexicon, Modifiers, Os, Outcome, PhysicalKey,
    ReplaceKind,
};

pub fn layout(file: &str) -> LayoutMap {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (mut id, mut lang, mut name) = (None, None, None);
    let mut rows = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut cols = line.split('\t');
        match cols.next().unwrap() {
            "@id" => id = cols.next().map(str::to_owned),
            "@lang" => {
                lang = Some(match cols.next() {
                    Some("en") => Lang::En,
                    Some("ru") => Lang::Ru,
                    _ => Lang::Other,
                })
            }
            "@name" => name = cols.next().map(str::to_owned),
            code => {
                let key: PhysicalKey = code.parse().unwrap_or_else(|e| panic!("{file}: {e}"));
                rows.push((key, cols.map(|c| c.chars().next()).collect::<Vec<_>>()));
            }
        }
    }
    let mut map = LayoutMap::new(
        LayoutId(id.expect("@id")),
        lang.expect("@lang"),
        name.expect("@name"),
    );
    let levels = [Level::Base, Level::Shift, Level::Caps, Level::ShiftCaps];
    for (key, chars) in rows {
        for (level, ch) in levels.into_iter().zip(chars) {
            if let Some(ch) = ch {
                map.insert(key, level, ch);
            }
        }
    }
    map
}

/// macOS U.S.
pub fn us() -> LayoutMap {
    layout("us.tsv")
}

/// macOS «Русская» (Apple).
pub fn ru_apple() -> LayoutMap {
    layout("ru_apple.tsv")
}

/// macOS «Русская – ПК».
pub fn ru_pc() -> LayoutMap {
    layout("ru_pc.tsv")
}

/// Windows «Русская» (KBDRU).
pub fn ru_win() -> LayoutMap {
    layout("ru_win.tsv")
}

pub fn engine(layouts: Vec<LayoutMap>) -> Engine {
    Engine::new(
        Config::default_for(Os::MacOs),
        Lexicon::embedded(),
        LayoutSet(layouts),
    )
}

pub fn ctx(layout: &LayoutMap) -> Context {
    Context {
        app: AppId("com.apple.TextEdit".to_owned()),
        layout: layout.id.clone(),
        secure_input: false,
        password_field: false,
    }
}

pub fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

fn event(key: PhysicalKey, dir: KeyDir, mods: Modifiers) -> KeyEvent {
    KeyEvent {
        key,
        dir,
        mods,
        repeat: false,
        time_ms: 0,
    }
}

/// Нажатие и отпускание клавиши; возвращает исход нажатия.
pub fn press(engine: &mut Engine, ctx: &Context, key: PhysicalKey, mods: Modifiers) -> Outcome {
    let down = engine.on_key(&event(key, KeyDir::Down, mods), ctx);
    let up = engine.on_key(&event(key, KeyDir::Up, mods), ctx);
    assert_eq!(up, Outcome::pass(), "отпускание {key} должно проходить");
    down
}

/// Набирает строку физическими клавишами раскладки `map`; заглавные — с настоящим ShiftLeft.
/// Возвращает исходы нажатий символьных клавиш.
pub fn type_text(engine: &mut Engine, ctx: &Context, map: &LayoutMap, text: &str) -> Vec<Outcome> {
    text.chars()
        .map(|c| {
            let stroke = map
                .stroke_for(c)
                .unwrap_or_else(|| panic!("в {} нет символа {c:?}", map.name));
            if !stroke.shift {
                return press(engine, ctx, stroke.key, Modifiers::default());
            }
            engine.on_key(&event(PhysicalKey::ShiftLeft, KeyDir::Down, shift()), ctx);
            let out = press(engine, ctx, stroke.key, shift());
            engine.on_key(
                &event(PhysicalKey::ShiftLeft, KeyDir::Up, Modifiers::default()),
                ctx,
            );
            out
        })
        .collect()
}

pub fn space(engine: &mut Engine, ctx: &Context) -> Outcome {
    press(engine, ctx, PhysicalKey::Space, Modifiers::default())
}

pub fn all_pass(outcomes: &[Outcome]) -> bool {
    outcomes.iter().all(|o| *o == Outcome::pass())
}

/// Нажатия без Shift.
pub fn plain(keys: &[PhysicalKey]) -> Vec<KeyStroke> {
    keys.iter()
        .map(|&key| KeyStroke { key, shift: false })
        .collect()
}

/// Ожидаемый исход автоисправления с проглоченной границей.
pub fn replaced(text: &str, erase: usize, keys: Vec<KeyStroke>, target: &LayoutMap) -> Outcome {
    Outcome {
        disposition: Disposition::Swallow,
        actions: vec![Action::Replace {
            erase,
            keys,
            text: text.to_owned(),
            target_layout: Some(target.id.clone()),
            kind: ReplaceKind::Auto,
        }],
    }
}
