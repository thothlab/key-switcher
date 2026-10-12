//! Разбор на хосте: события evdev → события модели (`tap::Tracker`), `_XKB_RULES_NAMES`,
//! каталог `evdev.lst`, JSON ответов IPC, выбор клавиатуры среди нескольких устройств.
//! Коды — из `linux/input-event-codes.h`.

use ks_core::{KeyDir, PhysicalKey as K};
use ks_linux::ipc::{hyprland, sway};
use ks_linux::json::Json;
use ks_linux::layouts::{LayoutSpec, XkbCatalog, parse_rules_names};
use ks_linux::tap::Tracker;
use ks_platform::intercept::InputKind;

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_LED: u16 = 0x11;
const LED_CAPSL: u16 = 1;

#[test]
fn tracker_reports_modifiers_at_capture_time() {
    let mut t = Tracker::default();
    let shift = t.input_event(EV_KEY, 42, 1, 1).expect("Shift");
    assert_eq!(shift.kind, InputKind::Modifier);
    assert_eq!(
        shift.key.map(|k| (k.key, k.dir)),
        Some((K::ShiftLeft, KeyDir::Down))
    );
    let a = t.input_event(EV_KEY, 30, 1, 2).expect("A");
    assert_eq!(a.kind, InputKind::Key);
    assert!(a.key.unwrap().mods.shift && a.native.mods.shift);
    assert_eq!(a.native.code, 30);
    t.input_event(EV_KEY, 54, 1, 3); // правый Shift
    t.input_event(EV_KEY, 42, 0, 4); // левый отпущен, правый держится
    assert!(
        t.input_event(EV_KEY, 30, 2, 5)
            .unwrap()
            .key
            .unwrap()
            .mods
            .shift
    );
    t.input_event(EV_KEY, 54, 0, 6);
    let plain = t.input_event(EV_KEY, 30, 1, 7).unwrap().key.unwrap();
    assert!(!plain.mods.shift && !plain.repeat);
    let repeat = t.input_event(EV_KEY, 30, 2, 8).unwrap().key.unwrap();
    assert!(repeat.repeat && repeat.dir == KeyDir::Down);
    t.input_event(EV_KEY, 97, 1, 9); // правый Ctrl
    let same_ms = t.input_event(EV_KEY, 30, 1, 9).unwrap();
    assert_eq!(same_ms.time_ms, 10, "время строго возрастает");
    assert!(
        t.input_event(EV_KEY, 44, 1, 11)
            .unwrap()
            .key
            .unwrap()
            .mods
            .ctrl
    );
}

#[test]
fn tracker_follows_caps_lock_led_and_toggle() {
    let mut t = Tracker::default();
    assert!(t.input_event(EV_LED, LED_CAPSL, 1, 0).is_none());
    assert!(
        t.input_event(EV_KEY, 30, 1, 1)
            .unwrap()
            .key
            .unwrap()
            .mods
            .caps_lock
    );
    t.input_event(EV_LED, LED_CAPSL, 0, 2);
    assert!(
        !t.input_event(EV_KEY, 30, 1, 3)
            .unwrap()
            .key
            .unwrap()
            .mods
            .caps_lock
    );
    t.input_event(EV_KEY, 58, 1, 4); // KEY_CAPSLOCK
    assert!(
        t.input_event(EV_KEY, 30, 1, 5)
            .unwrap()
            .key
            .unwrap()
            .mods
            .caps_lock
    );
}

#[test]
fn tracker_turns_clicks_into_mouse_down_and_drops_other_buttons() {
    let mut t = Tracker::default();
    let click = t.input_event(EV_KEY, 0x110, 1, 0).expect("BTN_LEFT");
    assert_eq!(click.kind, InputKind::MouseDown);
    assert!(click.key.is_none());
    assert!(
        t.input_event(EV_KEY, 0x110, 0, 1).is_none(),
        "отпускание кнопки"
    );
    assert_eq!(
        t.input_event(EV_KEY, 0x14a, 1, 2).map(|e| e.kind),
        Some(InputKind::MouseDown)
    );
    assert!(
        t.input_event(EV_KEY, 0x145, 1, 3).is_none(),
        "BTN_TOOL_FINGER"
    );
    assert!(
        t.input_event(EV_KEY, 0x130, 1, 4).is_none(),
        "BTN_A геймпада"
    );
    assert!(t.input_event(EV_REL, 0, 5, 5).is_none(), "движение мыши");
    let unknown = t.input_event(EV_KEY, 0x2a0, 1, 6).unwrap().key.unwrap();
    assert_eq!(unknown.key, K::Other(0x2a0));
}

fn ids(specs: &[LayoutSpec]) -> Vec<String> {
    specs.iter().map(|s| s.id().0).collect()
}

#[test]
fn xkb_rules_names_and_catalog_are_parsed() {
    let rules = b"evdev\0pc105\0us,ru\0,winkeys\0grp:alt_shift_toggle\0";
    assert_eq!(ids(&parse_rules_names(rules)), ["us", "ru(winkeys)"]);
    assert_eq!(ids(&parse_rules_names(b"evdev\0pc105\0us\0\0\0")), ["us"]);

    let lst = "! model\n  pc105           Generic 105-key PC\n\n! layout\n  us              English (US)\n  ru              Russian\n\n! variant\n  winkeys         ru: Russian (Windows)\n  phonetic        ru: Russian (phonetic)\n\n! option\n  grp                  Switching to another layout\n";
    let catalog = XkbCatalog::parse(lst);
    let win = catalog
        .by_description("Russian (Windows)")
        .expect("вариант");
    assert_eq!(
        (win.layout.as_str(), win.variant.as_str()),
        ("ru", "winkeys")
    );
    assert_eq!(
        catalog.by_description("English (US)").map(|s| s.id().0),
        Some("us".to_owned())
    );
    assert!(
        catalog.by_description("Generic 105-key PC").is_none(),
        "модели — не раскладки"
    );
    assert!(
        catalog
            .by_description("Switching to another layout")
            .is_none()
    );
    let described = catalog.describe(LayoutSpec::new("ru", "phonetic"));
    assert_eq!(described.description, "Russian (phonetic)");
    assert_eq!(catalog.resolve("Klingon").id().0, "Klingon");
}

#[test]
fn json_handles_escapes_surrogates_and_numbers() {
    let v = Json::parse(
        r#"{"s": "ж\"\\\/\n\t", "u": "Новая", "n": [1, -2.5, 3e2, 0], "t": true, "z": null}"#,
    )
    .expect("документ");
    assert_eq!(v.get("s").and_then(Json::as_str), Some("ж\"\\/\n\t"));
    assert_eq!(v.get("u").and_then(Json::as_str), Some("Новая"));
    let n = v.get("n").unwrap().items();
    assert_eq!(n[0].as_u64(), Some(1));
    assert_eq!(n[1], Json::Num(-2.5));
    assert_eq!(n[1].as_u64(), None);
    assert_eq!(n[2].as_u64(), Some(300));
    assert_eq!(v.get("t").and_then(Json::as_bool), Some(true));
    assert_eq!(v.get("z"), Some(&Json::Null));
    assert_eq!(Json::parse(r#""😀""#), Some(Json::Str("😀".to_owned())));
    assert_eq!(Json::parse(r#""\ud83d""#), None, "одиночный суррогат");
    assert_eq!(Json::parse("{} x"), None, "хвост после документа");
    assert_eq!(Json::parse(&"[".repeat(300)), None, "слишком глубоко");
    assert_eq!(Json::parse(r#"{"a" 1}"#), None);
}

#[test]
fn sway_picks_the_physical_keyboard_with_both_layouts() {
    let inputs = r#"[
      {"name": "Power Button", "type": "keyboard", "xkb_layout_names": ["English (US)"],
       "xkb_active_layout_index": 0},
      {"name": "KeySwitcher virtual keyboard", "type": "keyboard",
       "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 0},
      {"name": "Logitech USB Keyboard", "type": "keyboard",
       "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 1}
    ]"#;
    let (specs, active) = sway::keyboard_layouts(
        inputs,
        "KeySwitcher virtual keyboard",
        &XkbCatalog::builtin(),
    )
    .expect("клавиатура");
    assert_eq!(
        (ids(&specs), active),
        (vec!["us".to_owned(), "ru".to_owned()], 1)
    );
}

#[test]
fn hyprland_skips_own_device_and_power_button() {
    let devices = r#"{"keyboards": [
      {"name": "keyswitcher-virtual-keyboard", "layout": "us,ru", "variant": "",
       "active_keymap": "English (US)", "main": true},
      {"name": "power-button", "layout": "us", "variant": "", "active_keymap": "English (US)",
       "main": false},
      {"name": "at-translated-set-2-keyboard", "layout": "us,ru", "variant": "",
       "active_keymap": "Russian", "main": false}
    ]}"#;
    let (specs, active) = hyprland::keyboard_layouts(
        devices,
        "KeySwitcher virtual keyboard",
        &XkbCatalog::builtin(),
    )
    .expect("клавиатура");
    assert_eq!(ids(&specs), ["us", "ru"]);
    assert_eq!(active, "Russian");
}
