//! Швы бэкенда Windows без системных вызовов: коды клавиш и таблица KBDRU против KLC с
//! kbdlayout.info (`tests/fixtures/kbdru.klc`) и фикстуры задачи 01
//! (`ks-core/tests/fixtures/ru_win.tsv`), перевод событий хука в `InputEvent`, состояние
//! модификаторов в воспроизведении, формат `LayoutId`. Модель исполнения —
//! `ks-platform/tests/seams.rs`.

use std::path::Path;
use std::str::FromStr;

use ks_core::{KeyDir, Lang, LayoutId, LayoutMap, Level, PhysicalKey as K, PhysicalKey};
use ks_platform::intercept::InputKind;
use ks_windows::klc::{self, Klc};
use ks_windows::send::{self, MASK_VK, Stroke};
use ks_windows::tap::{MARK_OWN, ModState, RawEvent, RawKind, Tracker};
use ks_windows::{apps, keycode, layouts};

const LEVELS: [Level; 4] = [Level::Base, Level::Shift, Level::Caps, Level::ShiftCaps];

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Фикстура задачи 01 в формате `ks-core/tests/common/mod.rs`; плюс список её клавиш.
fn fixture() -> (LayoutMap, Vec<PhysicalKey>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ks-core/tests/fixtures/ru_win.tsv");
    let text = read(&path);
    let meta = |tag: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(tag))
            .map(|v| v.trim().to_owned())
            .unwrap_or_else(|| panic!("{tag}"))
    };
    let lang = match meta("@lang").as_str() {
        "ru" => Lang::Ru,
        "en" => Lang::En,
        _ => Lang::Other,
    };
    let mut map = LayoutMap::new(LayoutId(meta("@id")), lang, meta("@name"));
    let mut keys = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with('@') || line.trim().is_empty() {
            continue;
        }
        let mut cols = line.split('\t');
        let key = PhysicalKey::from_str(cols.next().unwrap_or("")).expect("код W3C");
        for (level, cell) in LEVELS.into_iter().zip(cols) {
            if let Some(ch) = cell.chars().next() {
                map.insert(key, level, ch);
            }
        }
        keys.push(key);
    }
    (map, keys)
}

fn kbdru() -> Klc {
    Klc::parse(&read(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kbdru.klc"),
    ))
}

#[test]
fn every_kbdru_fixture_key_has_the_klc_scan_code_and_vk() {
    let klc = kbdru();
    let (fixture, keys) = fixture();
    let printable: Vec<PhysicalKey> = keycode::printable().map(|(k, _, _)| k).collect();
    assert_eq!(printable.len(), 48);
    for key in &printable {
        assert!(keys.contains(key), "{key} нет в фикстуре KBDRU");
    }
    for key in keys {
        let scan = keycode::scan_of(key).unwrap_or_else(|| panic!("{key}: нет скан-кода"));
        let row = klc
            .row(scan)
            .unwrap_or_else(|| panic!("{key}: в KLC нет скан-кода {scan:02x}"));
        let vk = klc::vk_code(&row.vk).unwrap_or_else(|| panic!("VK {}", row.vk));
        assert_eq!(keycode::vk_of(key), Some(vk), "{key}: VK {}", row.vk);
        assert_eq!(keycode::to_physical(vk, u32::from(scan), false), key);
        // Позиция: символ KLC на этом скан-коде — символ фикстуры на этой клавише.
        assert_eq!(
            klc.translate(scan, Level::Base),
            fixture.char_for(key, Level::Base),
            "{key}"
        );
    }
}

#[test]
fn hook_codes_of_non_printing_keys_and_fallbacks() {
    let cases = [
        (0x0D, 0x1C, false, K::Enter),
        (0x08, 0x0E, false, K::Backspace),
        (0x25, 0x4B, true, K::ArrowLeft),
        (0x2E, 0x53, true, K::Delete),
        (0xA3, 0x1D, true, K::ControlRight),
        (0xA5, 0x38, true, K::AltRight),
        (0xA1, 0x36, false, K::ShiftRight),
        (0x5B, 0x5B, true, K::MetaLeft),
        (0x13, 0x45, false, K::Pause),
    ];
    for (vk, scan, ext, key) in cases {
        assert_eq!(keycode::to_physical(vk, scan, ext), key, "{key}");
    }
    // Чужой SendInput только с VK: клавиша узнаётся по VK.
    assert_eq!(keycode::to_physical(0x41, 0, false), K::KeyA);
    // Незнакомый скан-код у клавиши с не зависящим от раскладки VK — по VK: Enter цифрового
    // блока (E0 1C) остаётся границей слова, стрелка цифрового блока без NumLock — стрелкой.
    assert_eq!(keycode::to_physical(0x0D, 0x1C, true), K::Enter);
    assert_eq!(keycode::to_physical(0x25, 0x4B, false), K::ArrowLeft);
    // F1: ни скан-кода, ни VK в таблице — сырой скан-код.
    assert_eq!(keycode::to_physical(0x70, 0x3B, false), K::Other(0x3B));
    assert_eq!(keycode::scan_of(K::ArrowUp), Some(keycode::EXT | 0x48));
}

#[test]
fn klc_table_builds_the_kbdru_fixture() {
    let klc = kbdru();
    let (fixture, _) = fixture();
    let hkl = layouts::hkl_for_klid(&klc.locale_id).expect("KLID стандартной раскладки");
    let built = layouts::build_layout(
        layouts::layout_id(hkl),
        layouts::lang_of(hkl),
        "Russian",
        |_, scan, level| klc.translate(scan, level),
    );
    assert_eq!(built.id, fixture.id, "@id фикстуры — HKL бэкенда");
    assert_eq!(built.lang, fixture.lang);
    for (key, _, _) in keycode::printable() {
        for level in LEVELS {
            assert_eq!(
                built.char_for(key, level),
                fixture.char_for(key, level),
                "{key} {level:?}"
            );
        }
    }
    // AltGr+8 = ₽ и десятичная клавиша с `,` есть в KLC, но в четыре уровня таблицы не входят.
    assert_eq!(
        klc.row(0x09)
            .and_then(|r| r.values.get(3).copied().flatten()),
        Some('₽')
    );
    assert_eq!(klc.translate(0x53, Level::Base), Some(','));
}

#[test]
fn layout_ids_are_hkl_hex_and_round_trip() {
    assert_eq!(layouts::layout_id(0x0419_0419).0, "04190419");
    assert_eq!(layouts::hkl_for_klid("00000409"), Some(0x0409_0409));
    let dvorak = 0xF002_0409_u32 as i32 as isize;
    assert_eq!(layouts::layout_id(dvorak).0, "F0020409");
    assert_eq!(
        layouts::hkl_of(&LayoutId("F0020409".to_owned())),
        Some(dvorak)
    );
    assert_eq!(layouts::hkl_of(&LayoutId("us".to_owned())), None);
    assert_eq!(layouts::lang_of(dvorak), Lang::En);
    assert_eq!(layouts::klid_of(0x0419_0419).as_deref(), Some("00000419"));
    assert_eq!(layouts::klid_of(dvorak), None);
}

#[test]
fn tounicode_state_and_output() {
    let state = layouts::key_state(Level::ShiftCaps);
    assert_eq!((state[0x10], state[0x14]), (0x80, 0x01));
    assert_eq!(layouts::key_state(Level::Caps)[0x10], 0);
    assert_eq!(layouts::printable_char(&[0x0444, 0], 1), Some('ф'));
    // Мёртвая клавиша: отрицательный возврат, хотя символ записан.
    assert_eq!(layouts::printable_char(&[0x005E, 0], -1), None);
    assert_eq!(layouts::printable_char(&[0x001B, 0], 1), None);
}

fn key(kind: RawKind, vk: u16, scan: u32, ext: bool) -> RawEvent {
    RawEvent::key(kind, vk, scan, ext)
}

#[test]
fn modifiers_are_sided_events_and_own_events_are_marked() {
    let mut t = Tracker::new(false);
    let rshift = t.input_event(&key(RawKind::KeyDown, 0xA1, 0x36, false));
    assert_eq!(rshift.kind, InputKind::Modifier);
    let kev = rshift.key.expect("модификатор переведён");
    assert_eq!((kev.key, kev.dir), (K::ShiftRight, KeyDir::Down));
    assert!(kev.mods.shift);
    // Автоповтор зажатого Shift движку не отдаётся.
    let repeat = t.input_event(&key(RawKind::KeyDown, 0xA1, 0x36, false));
    assert_eq!(repeat.kind, InputKind::Modifier);
    assert!(repeat.key.is_none());
    let a = t.input_event(&key(RawKind::KeyDown, 0x41, 0x1E, false));
    assert_eq!(a.kind, InputKind::Key);
    assert!(a.key.expect("буква").mods.shift);
    let up = t.input_event(&key(RawKind::KeyUp, 0xA1, 0x36, false));
    assert_eq!(up.key.expect("отпускание").dir, KeyDir::Up);
    assert_eq!(t.mods(), ModState::default());

    // CapsLock — модификатор, переключает состояние на нажатии.
    let caps = t.input_event(&key(RawKind::KeyDown, 0x14, 0x3A, false));
    assert_eq!(caps.kind, InputKind::Modifier);
    assert!(caps.key.expect("CapsLock").mods.caps_lock);

    let mut own = key(RawKind::KeyDown, 0x08, 0x0E, false);
    own.injected = true;
    own.extra = MARK_OWN;
    assert!(t.input_event(&own).own);
    own.extra = 0;
    assert!(
        !t.input_event(&own).own,
        "чужой SendInput — нажатие пользователя"
    );

    // Фантомный левый Ctrl перед AltGr: не нажатие, состояние не меняется.
    let phantom = t.input_event(&key(RawKind::KeyDown, 0xA2, 0x21D, false));
    assert!(phantom.key.is_none());
    assert_eq!(t.mods().0 & ModState::CTRL, 0);

    let mut t = Tracker::new(false);
    let click = t.input_event(&RawEvent::other(RawKind::MouseDown, 5));
    assert_eq!(click.kind, InputKind::MouseDown);
    let gone = t.input_event(&RawEvent::other(RawKind::Disabled, 6));
    assert_eq!(gone.kind, InputKind::Disabled);
}

fn k(vk: u16, scan: u16, up: bool) -> Stroke {
    Stroke::Key { vk, scan, up }
}

#[test]
fn held_key_replays_with_modifiers_of_the_moment_of_capture() {
    // Shift+a во время замены: Shift прошёл и отпущен сразу, «a» придержана и воспроизводится
    // позже, когда Shift физически уже не зажат.
    let mut t = Tracker::new(false);
    t.input_event(&key(RawKind::KeyDown, 0xA0, 0x2A, false));
    let a = t.input_event(&key(RawKind::KeyDown, 0x41, 0x1E, false));
    t.input_event(&key(RawKind::KeyUp, 0xA0, 0x2A, false));
    assert_eq!(
        send::replay_strokes(&a.native, t.mods()),
        vec![
            k(0xA0, 0x2A, false),
            k(0x41, 0x1E, false),
            k(0xA0, 0x2A, true)
        ]
    );
    // Наоборот: «a» без Shift, а к воспроизведению зажат правый Shift.
    let mut t = Tracker::new(false);
    let a = t.input_event(&key(RawKind::KeyDown, 0x41, 0x1E, false));
    t.input_event(&key(RawKind::KeyDown, 0xA1, 0x36, false));
    assert_eq!(
        send::replay_strokes(&a.native, t.mods()),
        vec![
            k(0xA1, 0x36, true),
            k(0x41, 0x1E, false),
            k(0xA1, 0x36, false)
        ]
    );
}

#[test]
fn replacement_typing_releases_held_modifiers_and_restores_them() {
    let rctrl = ModState(ModState::RCTRL);
    assert_eq!(
        send::key_strokes(K::KeyA, true, true, rctrl),
        Some(vec![
            k(0xA3, keycode::EXT | 0x1D, true),
            k(0xA0, 0x2A, false),
            k(0x41, 0x1E, false),
            k(0xA0, 0x2A, true),
            k(0xA3, keycode::EXT | 0x1D, false),
        ])
    );
    assert_eq!(
        send::key_strokes(K::Backspace, true, false, ModState::default()),
        Some(vec![k(0x08, 0x0E, false)])
    );
    assert_eq!(send::key_strokes(K::Other(0x77), true, false, rctrl), None);
    // Отпускание Alt маскируется пустой клавишей, иначе откроется меню окна.
    assert_eq!(
        send::unicode_strokes("ф", ModState(ModState::LALT)),
        vec![
            k(MASK_VK, 0, false),
            k(MASK_VK, 0, true),
            k(0xA4, 0x38, true),
            Stroke::Unicode {
                unit: 0x0444,
                up: false
            },
            Stroke::Unicode {
                unit: 0x0444,
                up: true
            },
            k(0xA4, 0x38, false),
        ]
    );
    assert_eq!(
        send::chord_strokes(keycode::VK_C, ModState::default()),
        vec![
            k(0xA2, 0x1D, false),
            k(0x43, 0x2E, false),
            k(0x43, 0x2E, true),
            k(0xA2, 0x1D, true),
        ]
    );
}

#[test]
fn apps_are_exe_names_and_terminals_do_not_paste() {
    let app = apps::app_id(r"C:\Program Files\Mozilla Firefox\Firefox.EXE");
    assert_eq!(app.0, "firefox.exe");
    assert!(apps::paste_usable(&app));
    assert!(!apps::paste_usable(&apps::app_id(
        r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal\WindowsTerminal.exe"
    )));
    assert_eq!(
        apps::run_command(r"C:\Tools\Key Switcher\keyswitcher.exe"),
        r#""C:\Tools\Key Switcher\keyswitcher.exe""#
    );
}

/// Дозапрос ревью задачи 08: модификаторы «сейчас» из хука, AltGr, `VK_PACKET`, буфер обмена,
/// управляющие символы в запасном пути.
mod review {
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    use ks_core::{Action, AppId, KeyStroke, ReplaceKind};
    use ks_platform::exec::{Executor, Lease, Poster, Switcher};
    use ks_windows::clip::{ClipboardOs, EXCLUDE_MARKS, GuardedClipboard, Item};
    use ks_windows::send::{Sink, Typist};
    use ks_windows::tap::HeldMods;

    use super::*;

    /// Пачки `SendInput` в список; Ctrl+C «копирует» выделение в поддельный буфер.
    #[derive(Default)]
    struct LogSink {
        strokes: Mutex<Vec<Stroke>>,
        copy_into: Option<Arc<FakeOs>>,
    }

    impl LogSink {
        fn strokes(&self) -> Vec<Stroke> {
            self.strokes.lock().unwrap().clone()
        }
    }

    impl Sink for LogSink {
        fn send(&self, strokes: &[Stroke]) {
            if let Some(os) = &self.copy_into
                && strokes.contains(&k(keycode::VK_C, 0x2E, false))
            {
                os.set(vec![("13".to_owned(), utf16("ghbdtn"))]);
            }
            self.strokes.lock().unwrap().extend_from_slice(strokes);
        }
        fn pause_ms(&self, _ms: u64) {}
    }

    struct NoSwitch;

    impl Switcher for NoSwitch {
        fn post_allowed(&self) -> bool {
            true
        }
        fn select_layout(&self, _id: &LayoutId) -> bool {
            false
        }
    }

    struct Free;

    impl Lease for Free {
        fn alive(&self) -> bool {
            true
        }
        fn reserve(&self, _ms: u64) -> bool {
            true
        }
    }

    /// Поддельный системный буфер: `None` — занят другим приложением.
    #[derive(Default)]
    struct FakeOs {
        items: Mutex<Option<Vec<Item>>>,
        writes: Mutex<Vec<Vec<Item>>>,
        seq: AtomicI64,
    }

    impl FakeOs {
        fn set(&self, items: Vec<Item>) {
            *self.items.lock().unwrap() = Some(items);
            self.seq.fetch_add(1, Ordering::SeqCst);
        }
        fn writes(&self) -> Vec<Vec<Item>> {
            self.writes.lock().unwrap().clone()
        }
    }

    struct SharedOs(Arc<FakeOs>);

    impl ClipboardOs for SharedOs {
        fn read_all(&self) -> Option<Vec<Item>> {
            self.0.items.lock().unwrap().clone()
        }
        fn read(&self, format: &str) -> Option<Vec<u8>> {
            let items = self.0.items.lock().unwrap().clone()?;
            items.into_iter().find(|(f, _)| f == format).map(|(_, b)| b)
        }
        fn replace(&self, items: &[Item]) -> bool {
            if self.0.items.lock().unwrap().is_none() {
                return false;
            }
            self.0.writes.lock().unwrap().push(items.to_vec());
            self.0.set(items.to_vec());
            true
        }
        fn sequence(&self) -> i64 {
            self.0.seq.load(Ordering::SeqCst)
        }
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn own(vk: u16, scan: u32, ext: bool, up: bool) -> RawEvent {
        let kind = if up { RawKind::KeyUp } else { RawKind::KeyDown };
        let mut ev = RawEvent::key(kind, vk, scan, ext);
        ev.injected = true;
        ev.extra = MARK_OWN;
        ev
    }

    fn is_shift(vk: u16) -> bool {
        matches!(vk, 0x10 | 0xA0 | 0xA1)
    }

    #[test]
    fn double_shift_job_leaves_no_shift_pressed() {
        let held = Arc::new(HeldMods::default());
        let mut t = Tracker::new(false).with_published(held.clone());
        for kind in [
            RawKind::KeyDown,
            RawKind::KeyUp,
            RawKind::KeyDown,
            RawKind::KeyUp,
        ] {
            t.input_event(&key(kind, 0xA0, 0x2A, false));
        }
        assert_eq!(
            held.get(),
            ModState::default(),
            "хук уже применил отпускание"
        );
        // Задание, поставленное на последнем отпускании (⇧⇧ → отмена): перепечатка с заглавной.
        let typist = Arc::new(Typist::new(held, LogSink::default()));
        let exec = Executor::new(typist.clone(), Arc::new(NoSwitch));
        exec.execute(&Action::Replace {
            erase: 2,
            keys: vec![
                KeyStroke {
                    key: K::KeyG,
                    shift: true,
                },
                KeyStroke {
                    key: K::KeyH,
                    shift: false,
                },
            ],
            text: "Gh".to_owned(),
            target_layout: None,
            kind: ReplaceKind::Undo,
        })
        .expect("замена");
        let strokes = typist.sink().strokes();
        let last_shift = strokes.iter().rev().find_map(|s| match *s {
            Stroke::Key { vk, up, .. } if is_shift(vk) => Some(up),
            _ => None,
        });
        assert_ne!(last_shift, Some(false), "Shift остался нажатым");
        let presses = strokes
            .iter()
            .filter(|s| matches!(**s, Stroke::Key { vk, up: false, .. } if is_shift(vk)))
            .count();
        assert_eq!(
            presses, 2,
            "Shift нажимается только вокруг заглавной (нажатие и отпускание G)"
        );
    }

    #[test]
    fn restoring_altgr_leaves_no_left_ctrl_in_the_tracker() {
        let held = Arc::new(HeldMods::default());
        let mut t = Tracker::new(false).with_published(held.clone());
        // AltGr зажат: фантомный LCtrl (скан-код 0x21D) и правый Alt.
        t.input_event(&key(RawKind::KeyDown, 0xA2, 0x21D, false));
        t.input_event(&key(RawKind::KeyDown, 0xA5, 0x38, true));
        let typist = Typist::new(held.clone(), LogSink::default());
        typist.key(K::KeyA, true, false);
        typist.key(K::KeyA, false, false);
        let strokes = typist.sink().strokes();
        assert!(
            !strokes
                .iter()
                .any(|s| matches!(*s, Stroke::Key { vk: 0xA2, .. })),
            "свой LCtrl не нажимается"
        );
        // Хук видит свои события; на свой правый Alt система добавляет фантомный LCtrl.
        for s in &strokes {
            if let Stroke::Key { vk, scan, up } = *s {
                if vk == 0xA5 {
                    t.input_event(&own(0xA2, 0x21D, false, up));
                }
                t.input_event(&own(
                    vk,
                    u32::from(scan & 0xFF),
                    scan & keycode::EXT != 0,
                    up,
                ));
            }
        }
        t.input_event(&key(RawKind::KeyUp, 0xA2, 0x21D, false));
        t.input_event(&key(RawKind::KeyUp, 0xA5, 0x38, true));
        assert_eq!(t.mods(), ModState::default());
        assert_eq!(held.get(), ModState::default());
    }

    #[test]
    fn foreign_unicode_packet_is_an_other_key_and_replays_as_unicode() {
        // U+041E в scanCode: младший байт 0x1E — позиция KeyA, но это не клавиша.
        assert!(matches!(
            keycode::to_physical(keycode::VK_PACKET, 0x041E, false),
            K::Other(_)
        ));
        let mut t = Tracker::new(false);
        let mut ev = key(RawKind::KeyDown, keycode::VK_PACKET, 0x041E, false);
        ev.injected = true;
        let out = t.input_event(&ev);
        assert_eq!(out.kind, InputKind::Key);
        assert!(!out.own);
        assert!(matches!(out.key.expect("событие").key, K::Other(_)));
        assert_eq!(
            send::replay_strokes(&out.native, ModState::default()),
            vec![Stroke::Unicode {
                unit: 0x041E,
                up: false
            }]
        );
    }

    #[test]
    fn unicode_fallback_types_enter_and_tab_as_real_keys() {
        assert_eq!(
            send::unicode_strokes("ы\n\t", ModState::default()),
            vec![
                Stroke::Unicode {
                    unit: 0x044B,
                    up: false
                },
                Stroke::Unicode {
                    unit: 0x044B,
                    up: true
                },
                k(0x0D, 0x1C, false),
                k(0x0D, 0x1C, true),
                k(0x09, 0x0F, false),
                k(0x09, 0x0F, true),
            ]
        );
        assert_eq!(
            send::unicode_strokes("\r\n", ModState::default()),
            vec![k(0x0D, 0x1C, false), k(0x0D, 0x1C, true)]
        );
    }

    fn selection_run(os: Arc<FakeOs>) -> Vec<Stroke> {
        let clip = Arc::new(GuardedClipboard::new(SharedOs(os.clone())));
        let sink = LogSink {
            copy_into: Some(os),
            ..LogSink::default()
        };
        let typist = Arc::new(Typist::new(Arc::default(), sink));
        let exec = Executor::new(typist.clone(), Arc::new(NoSwitch)).with_clipboard(clip);
        exec.convert_selection(
            &AppId("notepad.exe".to_owned()),
            &|text| (text == "ghbdtn").then(|| "привет".to_owned()),
            &Free,
        )
        .expect("конвертация");
        typist.sink().strokes()
    }

    #[test]
    fn busy_clipboard_cancels_selection_conversion_without_touching_it() {
        let os = Arc::new(FakeOs::default());
        let strokes = selection_run(os.clone());
        assert!(strokes.is_empty(), "Ctrl+C не отправлен");
        assert!(os.writes().is_empty(), "буфер пользователя не тронут");
    }

    #[test]
    fn own_clipboard_writes_are_excluded_from_history_and_restore_the_user_copy() {
        let os = Arc::new(FakeOs::default());
        let mine = ("13".to_owned(), utf16("моё"));
        os.set(vec![mine.clone()]);
        let strokes = selection_run(os.clone());
        assert!(
            strokes.contains(&k(keycode::VK_V, 0x2F, false)),
            "Ctrl+V отправлен"
        );
        let writes = os.writes();
        assert_eq!(writes.len(), 2, "результат и восстановление");
        assert!(writes[0].contains(&("13".to_owned(), utf16("привет"))));
        assert!(writes[1].contains(&mine));
        for write in &writes {
            for mark in EXCLUDE_MARKS {
                assert!(write.iter().any(|(f, _)| f == mark), "{mark}");
            }
        }
    }

    /// Система и хук с запаздыванием: нажатия пачки применяются сразу, а хук видит их позже;
    /// отпускание пользователя может попасть между чтением состояния и `SendInput`.
    struct Sim {
        tracker: Tracker,
        pressed: Vec<u16>,
        lagging: Vec<Stroke>,
        release_before_next_send: Option<(u16, u32)>,
    }

    struct SimSink(Arc<Mutex<Sim>>);

    impl Sink for SimSink {
        fn send(&self, strokes: &[Stroke]) {
            let mut sim = self.0.lock().unwrap();
            if let Some((vk, scan)) = sim.release_before_next_send.take() {
                sim.pressed.retain(|p| *p != vk);
                sim.tracker
                    .input_event(&key(RawKind::KeyUp, vk, scan, false));
            }
            for s in strokes {
                if let Stroke::Key { vk, up, .. } = *s {
                    sim.pressed.retain(|p| *p != vk);
                    if !up {
                        sim.pressed.push(vk);
                    }
                }
            }
            sim.lagging.extend_from_slice(strokes);
        }
        fn pause_ms(&self, _ms: u64) {}
    }

    fn sim_with_shift_held(held: &Arc<HeldMods>) -> Arc<Mutex<Sim>> {
        let mut tracker = Tracker::new(false).with_published(held.clone());
        tracker.input_event(&key(RawKind::KeyDown, 0xA0, 0x2A, false));
        Arc::new(Mutex::new(Sim {
            tracker,
            pressed: vec![0xA0],
            lagging: Vec::new(),
            release_before_next_send: Some((0xA0, 0x2A)),
        }))
    }

    /// Хук наконец видит свои пачки: состояние пользователя они не меняют.
    fn hook_catches_up(sim: &Arc<Mutex<Sim>>) {
        let mut sim = sim.lock().unwrap();
        let lagging = std::mem::take(&mut sim.lagging);
        for s in lagging {
            if let Stroke::Key { vk, scan, up } = s {
                sim.tracker.input_event(&own(
                    vk,
                    u32::from(scan & 0xFF),
                    scan & keycode::EXT != 0,
                    up,
                ));
            }
        }
    }

    #[test]
    fn batches_read_before_the_hook_sees_them_leave_no_shift_pressed() {
        // Пользователь держит Shift и отпускает его между чтением и SendInput первой пачки.
        let held = Arc::new(HeldMods::default());
        let sim = sim_with_shift_held(&held);
        let typist = Typist::new(held.clone(), SimSink(sim.clone()));
        typist.key(K::KeyA, true, false);
        typist.key(K::KeyA, false, false);
        hook_catches_up(&sim);
        assert!(!sim.lock().unwrap().pressed.iter().any(|vk| is_shift(*vk)));
        assert_eq!(held.get(), ModState::default());

        // Последняя пачка задания (Ctrl+C) вернула отпущенный Shift — сверка после задания.
        let held = Arc::new(HeldMods::default());
        let sim = sim_with_shift_held(&held);
        let typist = Typist::new(held.clone(), SimSink(sim.clone()));
        typist.chord(ks_platform::exec::Chord::Copy);
        assert!(
            sim.lock().unwrap().pressed.contains(&0xA0),
            "без сверки Shift залип бы"
        );
        assert!(typist.owes());
        typist.settle();
        hook_catches_up(&sim);
        assert!(!sim.lock().unwrap().pressed.iter().any(|vk| is_shift(*vk)));
        assert!(!typist.owes());
        assert_eq!(held.get(), ModState::default());
    }

    #[test]
    fn stale_modifier_is_dropped_when_the_hook_may_have_missed_input() {
        let held = Arc::new(HeldMods::default());
        let real = Arc::new(Mutex::new(ModState::default()));
        let source = real.clone();
        let mut t = Tracker::new(false)
            .with_published(held.clone())
            .with_key_state(ks_windows::tap::KeyState(Arc::new(move || {
                *source.lock().unwrap()
            })));
        // Win+L: нажатие Win видно, отпускание — уже на защищённом рабочем столе.
        t.input_event(&key(RawKind::KeyDown, 0x5B, 0x5B, true));
        assert_eq!(held.get(), ModState(ModState::LWIN));
        t.input_event(&RawEvent::other(RawKind::Disabled, 10));
        assert_eq!(held.get(), ModState::default());
        assert_eq!(t.mods(), ModState::default());
        // Зажатое на самом деле пересинхронизация сохраняет; следующее нажатие — автоповтор.
        *real.lock().unwrap() = ModState(ModState::RSHIFT);
        t.resync_from_os();
        assert_eq!(held.get(), ModState(ModState::RSHIFT));
        assert!(
            t.input_event(&key(RawKind::KeyDown, 0xA1, 0x36, false))
                .key
                .is_none()
        );
    }
}
