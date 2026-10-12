//! Швы окна без отрисовки: один экземпляр окна, запись сочетания из событий egui, кириллица в
//! шрифтах egui.

use eframe::egui::{self, Event, Key, Modifiers as EguiMods};
use ks_core::Hotkey;
use ks_settings::Section;
use ks_settings::instance::{self, Instance};
use ks_settings::record::{Recorded, Recorder};

fn temp_dir(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ks-window-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Ревью 06: повторное «Настройки…» не открывает второе окно, а поднимает первое на разделе.
#[test]
fn second_window_raises_the_first_on_its_section_and_exits() {
    let dir = temp_dir("instance");
    let Instance::First(guard) = instance::acquire(&dir).unwrap() else {
        panic!("первое окно берёт блокировку");
    };
    assert!(matches!(instance::acquire(&dir).unwrap(), Instance::Second));
    instance::raise(&dir, Section::Permissions).unwrap();
    assert_eq!(instance::take_raise(&dir), Some(Section::Permissions));
    assert_eq!(instance::take_raise(&dir), None, "просьба снимается");
    drop(guard);
    assert!(
        matches!(instance::acquire(&dir).unwrap(), Instance::First(_)),
        "после закрытия окна новое открывается"
    );
}

fn key(k: Key, mods: EguiMods) -> Event {
    Event::Key {
        key: k,
        physical_key: Some(k),
        pressed: true,
        repeat: false,
        modifiers: mods,
    }
}

#[test]
fn ctrl_alt_k_is_recorded_by_physical_key() {
    let mods = EguiMods {
        ctrl: true,
        alt: true,
        ..EguiMods::NONE
    };
    let mut r = Recorder::new();
    assert_eq!(r.feed(&[], mods), Recorded::Pending);
    assert_eq!(
        r.feed(&[key(Key::K, mods)], mods),
        Recorded::Done("Ctrl+Alt+KeyK".parse::<Hotkey>().unwrap())
    );
}

#[test]
fn modifiers_only_gesture_is_recorded_on_release_and_a_single_modifier_is_not() {
    let alt = EguiMods {
        alt: true,
        ..EguiMods::NONE
    };
    let alt_shift = EguiMods { shift: true, ..alt };
    let mut r = Recorder::new();
    assert_eq!(r.feed(&[], alt), Recorded::Pending);
    assert_eq!(r.feed(&[], alt_shift), Recorded::Pending);
    assert_eq!(r.feed(&[], alt), Recorded::Pending);
    assert_eq!(
        r.feed(&[], EguiMods::NONE),
        Recorded::Done("Alt+Shift".parse::<Hotkey>().unwrap())
    );
    assert_eq!(r.feed(&[], alt), Recorded::Pending);
    assert_eq!(
        r.feed(&[], EguiMods::NONE),
        Recorded::Pending,
        "один Alt — не жест"
    );
}

#[test]
fn escape_cancels_and_a_key_without_name_is_refused() {
    let mut r = Recorder::new();
    assert_eq!(
        r.feed(&[key(Key::Escape, EguiMods::NONE)], EguiMods::NONE),
        Recorded::Cancelled
    );
    assert!(matches!(
        r.feed(&[key(Key::F5, EguiMods::NONE)], EguiMods::NONE),
        Recorded::Refused(_)
    ));
}

/// Подзадача 10: кириллица, знаки и стрелки всего, что окно показывает (лицензии «О программе»,
/// пояснения пустых списков, подсказки раздела «Разрешения»), есть в шрифтах окна.
#[test]
fn window_fonts_have_every_glyph_the_window_shows() {
    let ctx = egui::Context::default();
    ks_settings::fonts::install(&ctx);
    // Шрифты появляются после первого прохода; текстуры этого прохода никто не рисует. Ширина, а не
    // `has_glyph`: тот ошибается для знаков из шрифта, которому принадлежит «◻» замены.
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let hint = "Системные настройки → Конфиденциальность и безопасность → Мониторинг ввода";
    let mut texts = vec![
        ks_settings::about::THIRD_PARTY,
        ks_settings::EMPTY_APPS,
        ks_settings::EMPTY_WORDS,
        hint,
        "Слова-исключения: Ёё ЖЩЪ „Не исправлять …“ — RU↔EN «Общие»",
    ];
    texts.extend(ks_settings::about::NOTICES.iter().map(|(_, text)| *text));
    for font in [
        egui::FontId::proportional(14.0),
        egui::FontId::monospace(14.0),
    ] {
        let mut missing: Vec<char> = texts
            .iter()
            .flat_map(|t| t.chars())
            .filter(|c| !c.is_control() && ctx.fonts_mut(|f| f.glyph_width(&font, *c)) <= 0.0)
            .collect();
        missing.sort_unstable();
        missing.dedup();
        assert!(missing.is_empty(), "нет глифов {missing:?} в {font:?}");
    }
}
