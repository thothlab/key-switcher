//! Швы бэкенда macOS без системных вызовов: коды клавиш и перевод событий перехвата в
//! нейтральный `InputEvent`. Модель исполнения проверяется в `ks-platform/tests/seams.rs`.

use ks_core::{KeyDir, PhysicalKey as K};
use ks_macos::keycode;
use ks_macos::tap::{MARK_OWN, MARK_SELFTEST, RawEvent, RawKind, input_event};
use ks_platform::intercept::InputKind;

#[test]
fn mac_keycodes_map_to_w3c_keys_and_back() {
    let carbon = [
        (0, K::KeyA),
        (5, K::KeyG),
        (18, K::Digit1),
        (43, K::Comma),
        (50, K::Backquote),
        (10, K::IntlBackslash),
        (36, K::Enter),
        (48, K::Tab),
        (49, K::Space),
        (51, K::Backspace),
        (56, K::ShiftLeft),
        (60, K::ShiftRight),
        (55, K::MetaLeft),
        (123, K::ArrowLeft),
        (126, K::ArrowUp),
    ];
    for (code, key) in carbon {
        assert_eq!(keycode::to_physical(code), key, "kVK {code}");
        assert_eq!(keycode::from_physical(key), Some(code), "{key}");
    }
    assert_eq!(keycode::to_physical(105), K::Other(105));
    assert_eq!(keycode::from_physical(K::Pause), None);
    assert_eq!(keycode::printable().count(), 48);
}

#[test]
fn left_and_right_shift_are_told_apart_by_device_bits() {
    // NX_DEVICELSHIFTKEYMASK = 0x2, NX_DEVICERSHIFTKEYMASK = 0x4, Shift = 0x20000
    assert_eq!(
        keycode::modifier_dir(K::ShiftLeft, 0x2_0002),
        Some(KeyDir::Down)
    );
    assert_eq!(
        keycode::modifier_dir(K::ShiftRight, 0x2_0002),
        Some(KeyDir::Up)
    );
    assert_eq!(keycode::modifier_dir(K::ShiftLeft, 0), Some(KeyDir::Up));
    assert!(keycode::modifiers(0x2_0002).shift);
    assert!(keycode::modifiers(0x1_0000).caps_lock);
}

#[test]
fn flags_changed_becomes_a_sided_modifier_event_and_own_events_are_marked() {
    // kVK_Shift = 56; Shift + NX_DEVICELSHIFTKEYMASK
    let shift = input_event(&RawEvent::key(RawKind::FlagsChanged, 56, 0x2_0002));
    assert_eq!(shift.kind, InputKind::Modifier);
    let key = shift.key.expect("модификатор переведён");
    assert_eq!((key.key, key.dir), (K::ShiftLeft, KeyDir::Down));
    assert!(!shift.own);

    let mut raw = RawEvent::key(RawKind::KeyUp, 5, 0);
    raw.user_data = MARK_OWN;
    let own = input_event(&raw);
    assert!(own.own, "своё событие замены помечено");
    assert_eq!(own.kind, InputKind::Key);
    assert_eq!(own.key.map(|k| (k.key, k.dir)), Some((K::KeyG, KeyDir::Up)));
    assert!(!own.native.down);

    raw.user_data = MARK_SELFTEST;
    assert!(
        !input_event(&raw).own,
        "нажатия --selftest идут как пользовательские"
    );
    for kind in [RawKind::DisabledByTimeout, RawKind::DisabledByUserInput] {
        assert_eq!(
            input_event(&RawEvent::key(kind, 0, 0)).kind,
            InputKind::Disabled
        );
    }
}

/// Жест из модификаторов, собранный `modifier_steps`, перехват читает как отдельные нажатия и
/// отпускания со стороной — иначе ⇧⇧ и ⌥⇧ проверочного режима до движка не доходят.
#[test]
fn modifier_gesture_steps_read_back_as_sided_presses_and_releases() {
    let read = |keys: &[K]| -> Vec<(K, KeyDir, bool, bool)> {
        keycode::modifier_steps(keys)
            .expect("жест из модификаторов")
            .into_iter()
            .map(|(code, flags)| {
                let ev = RawEvent::key(RawKind::FlagsChanged, code, flags);
                let k = ks_macos::tap::to_key_event(&ev).expect("событие модификатора");
                (k.key, k.dir, k.mods.shift, k.mods.alt)
            })
            .collect()
    };
    assert_eq!(
        read(&[K::ShiftLeft]),
        vec![
            (K::ShiftLeft, KeyDir::Down, true, false),
            (K::ShiftLeft, KeyDir::Up, false, false),
        ]
    );
    assert_eq!(
        read(&[K::AltLeft, K::ShiftLeft]),
        vec![
            (K::AltLeft, KeyDir::Down, false, true),
            (K::ShiftLeft, KeyDir::Down, true, true),
            (K::ShiftLeft, KeyDir::Up, false, true),
            (K::AltLeft, KeyDir::Up, false, false),
        ]
    );
    assert!(keycode::modifier_steps(&[K::KeyA]).is_none());
    assert!(keycode::modifier_steps(&[K::CapsLock]).is_none());
}
