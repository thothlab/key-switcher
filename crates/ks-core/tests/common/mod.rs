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
    engine_with(Config::default_for(Os::MacOs), layouts)
}

pub fn engine_with(config: Config, layouts: Vec<LayoutMap>) -> Engine {
    Engine::new(config, Lexicon::embedded(), LayoutSet(layouts))
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
    event_at(key, dir, mods, 0)
}

/// Событие с временем нажатия (часы ядра — только `time_ms`).
pub fn event_at(key: PhysicalKey, dir: KeyDir, mods: Modifiers, time_ms: u64) -> KeyEvent {
    KeyEvent {
        key,
        dir,
        mods,
        repeat: false,
        time_ms,
    }
}

/// Одно событие клавиши в момент `time_ms`.
pub fn key_at(
    engine: &mut Engine,
    ctx: &Context,
    key: PhysicalKey,
    dir: KeyDir,
    mods: Modifiers,
    time_ms: u64,
) -> Outcome {
    engine.on_key(&event_at(key, dir, mods, time_ms), ctx)
}

/// Нажатие в `time_ms` и отпускание через 30 мс; исходы обоих событий.
pub fn press_at(
    engine: &mut Engine,
    ctx: &Context,
    key: PhysicalKey,
    mods: Modifiers,
    time_ms: u64,
) -> Vec<Outcome> {
    vec![
        key_at(engine, ctx, key, KeyDir::Down, mods, time_ms),
        key_at(engine, ctx, key, KeyDir::Up, mods, time_ms + 30),
    ]
}

/// Одиночный Shift: нажат в `time_ms`, отпущен через 50 мс, без других клавиш.
pub fn shift_tap(
    engine: &mut Engine,
    ctx: &Context,
    key: PhysicalKey,
    time_ms: u64,
) -> Vec<Outcome> {
    vec![
        key_at(engine, ctx, key, KeyDir::Down, shift(), time_ms),
        key_at(
            engine,
            ctx,
            key,
            KeyDir::Up,
            Modifiers::default(),
            time_ms + 50,
        ),
    ]
}

/// Двойной левый Shift: два касания, второе через 150 мс после начала первого.
pub fn double_shift(engine: &mut Engine, ctx: &Context, time_ms: u64) -> Vec<Outcome> {
    let mut outcomes = shift_tap(engine, ctx, PhysicalKey::ShiftLeft, time_ms);
    outcomes.extend(shift_tap(
        engine,
        ctx,
        PhysicalKey::ShiftLeft,
        time_ms + 150,
    ));
    outcomes
}

/// Все действия из последовательности исходов, по порядку.
pub fn actions(outcomes: &[Outcome]) -> Vec<Action> {
    outcomes.iter().flat_map(|o| o.actions.clone()).collect()
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
