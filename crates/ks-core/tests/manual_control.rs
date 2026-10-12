//! Сценарии `manual-control/spec.md` на шве `Engine::on_key` (+ `tick`, `convert_text`).

mod common;

use common::*;
use ks_core::{
    Action, Config, Context, Disposition, Engine, KeyDir, KeyStroke, LayoutMap, Modifiers, Os,
    Outcome, PhysicalKey as K, ReplaceKind, ResetReason, WordRules,
};

const GHBDTN: [K; 6] = [K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN];
/// Момент далеко за всеми окнами жестов.
const LATER: u64 = 60_000;

fn with_space(keys: &[K]) -> Vec<KeyStroke> {
    let mut strokes = plain(keys);
    strokes.push(KeyStroke {
        key: K::Space,
        shift: false,
    });
    strokes
}

fn replace(
    kind: ReplaceKind,
    erase: usize,
    keys: Vec<KeyStroke>,
    text: &str,
    target: &LayoutMap,
) -> Action {
    Action::Replace {
        erase,
        keys,
        text: text.to_owned(),
        target_layout: Some(target.id.clone()),
        kind,
    }
}

fn config() -> Config {
    Config::default_for(Os::MacOs)
}

fn autocorrect_off() -> Config {
    Config {
        autocorrect: false,
        ..config()
    }
}

fn mods(shift: bool, alt: bool, ctrl: bool, caps_lock: bool) -> Modifiers {
    Modifiers {
        shift,
        alt,
        ctrl,
        caps_lock,
        meta: false,
    }
}

/// ⌥⇧: Alt, затем Shift; отпускание в обратном порядке.
fn alt_shift_chord(e: &mut Engine, c: &Context, t: u64) -> Vec<Outcome> {
    let alt = mods(false, true, false, false);
    vec![
        key_at(e, c, K::AltLeft, KeyDir::Down, alt, t),
        key_at(
            e,
            c,
            K::ShiftLeft,
            KeyDir::Down,
            mods(true, true, false, false),
            t + 20,
        ),
        key_at(e, c, K::ShiftLeft, KeyDir::Up, alt, t + 60),
        key_at(e, c, K::AltLeft, KeyDir::Up, Modifiers::default(), t + 80),
    ]
}

/// «ghbdtn » в US исправлено на «привет »; дальше бэкенд держит русскую раскладку.
fn corrected_ghbdtn(e: &mut Engine, us: &LayoutMap, ru: &LayoutMap) {
    let c = ctx(us);
    type_text(e, &c, us, "ghbdtn");
    assert_eq!(
        space(e, &c),
        replaced("привет ", 6, with_space(&GHBDTN), ru)
    );
}

// --- Отмена автоисправления -------------------------------------------------------------

#[test]
fn double_shift_right_after_correction_undoes_it() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    corrected_ghbdtn(&mut e, &us, &ru);
    assert_eq!(e.last_undone(), None);

    let undo = double_shift(&mut e, &ctx(&ru), 1_000);
    assert_eq!(
        actions(&undo),
        vec![replace(
            ReplaceKind::Undo,
            7,
            with_space(&GHBDTN),
            "ghbdtn ",
            &us
        )]
    );
    assert_eq!(e.last_undone(), Some("ghbdtn"));
    assert_eq!(
        e.tick(LATER),
        vec![],
        "двойной Shift не переключает ещё раз"
    );

    // отменённое слово до перезапуска не исправляется (G03); навсегда — только «Не исправлять»
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(space(&mut e, &c), Outcome::pass());
    let mut fresh = engine(vec![us.clone(), ru.clone()]);
    corrected_ghbdtn(&mut fresh, &us, &ru);
}

#[test]
fn pause_right_after_correction_undoes_it_and_is_swallowed() {
    let (us, ru) = (us(), ru_apple());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    corrected_ghbdtn(&mut e, &us, &ru);
    let out = press_at(&mut e, &ctx(&ru), K::Pause, Modifiers::default(), 1_000);
    let undo = replace(ReplaceKind::Undo, 7, with_space(&GHBDTN), "ghbdtn ", &us);
    assert_eq!(
        out,
        vec![
            Outcome {
                disposition: Disposition::Swallow,
                actions: vec![undo]
            },
            Outcome::pass()
        ]
    );
}

#[test]
fn letter_after_correction_closes_the_undo_window() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    corrected_ghbdtn(&mut e, &us, &ru);
    let c = ctx(&ru);
    type_text(&mut e, &c, &ru, "к");
    // «привет» не отменяется; ⇧⇧ работает как ручная конвертация текущего слова
    assert_eq!(
        actions(&double_shift(&mut e, &c, 1_000)),
        vec![replace(ReplaceKind::Manual, 1, plain(&[K::KeyR]), "r", &us)]
    );
    assert_eq!(e.last_undone(), None);
}

#[test]
fn after_enter_double_shift_does_not_retype_the_line() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_ne!(
        press(&mut e, &c, K::Enter, Modifiers::default()),
        Outcome::pass()
    );
    assert_eq!(actions(&double_shift(&mut e, &ctx(&ru), 1_000)), vec![]);
}

// --- Ручная конвертация последнего слова ------------------------------------------------

#[test]
fn double_shift_converts_a_word_autocorrect_left_alone() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine_with(autocorrect_off(), vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "z");
    assert_eq!(space(&mut e, &c), Outcome::pass());
    assert_eq!(
        actions(&double_shift(&mut e, &c, 1_000)),
        vec![replace(
            ReplaceKind::Manual,
            2,
            with_space(&[K::KeyZ]),
            "я ",
            &ru
        )]
    );
    assert_eq!(e.tick(LATER), vec![]);
}

#[test]
fn after_click_or_arrow_double_shift_does_nothing() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine_with(autocorrect_off(), vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "z");
    space(&mut e, &c);
    e.on_reset(ResetReason::MouseClick);
    assert_eq!(actions(&double_shift(&mut e, &c, 1_000)), vec![]);

    type_text(&mut e, &c, &us, "z");
    space(&mut e, &c);
    press(&mut e, &c, K::ArrowLeft, Modifiers::default());
    assert_eq!(actions(&double_shift(&mut e, &c, 3_000)), vec![]);
    assert_eq!(e.tick(LATER), vec![]);
}

// --- Одиночный Shift ---------------------------------------------------------------------

#[test]
fn single_shift_switches_layout_once_the_double_window_passes() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    assert_eq!(actions(&shift_tap(&mut e, &c, K::ShiftLeft, 1_000)), vec![]);
    // отпущен в 1050, второе нажатие ждётся до 1450 включительно
    assert_eq!(e.next_deadline(), Some(1_450));
    assert_eq!(e.tick(1_450), vec![]);
    assert_eq!(e.tick(1_451), vec![Action::SwitchLayout(ru.id.clone())]);
    assert_eq!(e.tick(LATER), vec![]);
    assert_eq!(e.next_deadline(), None);

    // без tick ожидание разрешает следующее нажатие; из русской — в английскую
    let c = ctx(&ru);
    shift_tap(&mut e, &c, K::ShiftRight, 2_000);
    let next = press_at(&mut e, &c, K::KeyJ, Modifiers::default(), 2_100);
    assert_eq!(actions(&next), vec![Action::SwitchLayout(us.id.clone())]);
}

#[test]
fn shift_with_a_letter_a_click_or_a_long_hold_does_not_switch() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    let mut out = vec![key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Down,
        shift(),
        1_000,
    )];
    out.extend(press_at(&mut e, &c, K::KeyA, shift(), 1_040));
    out.push(key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Up,
        Modifiers::default(),
        1_100,
    ));
    out.push(key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Down,
        shift(),
        5_000,
    ));
    e.on_reset(ResetReason::MouseClick);
    out.push(key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Up,
        Modifiers::default(),
        5_050,
    ));
    out.push(key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Down,
        shift(),
        9_000,
    ));
    out.push(key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Up,
        Modifiers::default(),
        9_301,
    ));
    assert_eq!(actions(&out), vec![]);
    assert_eq!(e.tick(LATER), vec![]);
}

#[test]
fn single_shift_switch_turned_off_does_nothing() {
    let (us, ru) = (us(), ru_pc());
    let cfg = Config {
        single_shift_switch: false,
        ..config()
    };
    let mut e = engine_with(cfg, vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    assert_eq!(actions(&shift_tap(&mut e, &c, K::ShiftLeft, 1_000)), vec![]);
    assert_eq!(e.tick(LATER), vec![]);
}

#[test]
fn double_shift_with_nothing_to_do_gives_no_extra_switch() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    assert_eq!(actions(&double_shift(&mut e, &c, 1_000)), vec![]);
    assert_eq!(e.tick(LATER), vec![]);
}

#[test]
fn unbound_double_shift_does_not_delay_single_shift() {
    let (us, ru) = (us(), ru_pc());
    let mut cfg = config();
    cfg.hotkeys.undo_or_convert_word = vec!["Pause".parse().unwrap()];
    let mut e = engine_with(cfg, vec![us.clone(), ru.clone()]);
    corrected_ghbdtn(&mut e, &us, &ru);
    // ⇧ после исправления — не отмена, а сразу переключение
    let tap = shift_tap(&mut e, &ctx(&ru), K::ShiftLeft, 1_000);
    assert_eq!(actions(&tap), vec![Action::SwitchLayout(us.id.clone())]);
    assert_eq!(e.next_deadline(), None);
}

// --- Оба Shift, хоткеи --------------------------------------------------------------------

#[test]
fn both_shifts_toggle_autocorrect_without_switching() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    let out = vec![
        key_at(&mut e, &c, K::ShiftLeft, KeyDir::Down, shift(), 1_000),
        key_at(&mut e, &c, K::ShiftRight, KeyDir::Down, shift(), 1_030),
        key_at(&mut e, &c, K::ShiftRight, KeyDir::Up, shift(), 1_080),
        key_at(
            &mut e,
            &c,
            K::ShiftLeft,
            KeyDir::Up,
            Modifiers::default(),
            1_100,
        ),
    ];
    assert_eq!(actions(&out), vec![Action::ToggleAutocorrect]);
    assert_eq!(e.tick(LATER), vec![]);
}

#[test]
fn hotkeys_are_matched_by_physical_keys_in_russian_layout() {
    let (us, ru) = (us(), ru_pc());
    let c = ctx(&ru);
    let mut e = engine(vec![us.clone(), ru.clone()]);
    assert_eq!(
        actions(&alt_shift_chord(&mut e, &c, 1_000)),
        vec![Action::ConvertSelection]
    );
    assert_eq!(e.tick(LATER), vec![], "Shift внутри ⌥⇧ — не одиночный");

    let mut cfg = config();
    cfg.hotkeys.convert_selection = vec!["Ctrl+Alt+KeyK".parse().unwrap()];
    let mut e = engine_with(cfg, vec![us.clone(), ru.clone()]);
    let out = press_at(&mut e, &c, K::KeyK, mods(false, true, true, false), 1_000);
    let fired = Outcome {
        disposition: Disposition::Swallow,
        actions: vec![Action::ConvertSelection],
    };
    assert_eq!(out, vec![fired, Outcome::pass()]);
}

#[test]
fn shift_pause_and_bare_pause_are_different_hotkeys_even_with_caps_lock() {
    let (us, ru) = (us(), ru_win());
    let mut e = engine_with(
        Config::default_for(Os::Windows),
        vec![us.clone(), ru.clone()],
    );
    let c = ctx(&ru);
    let out = press_at(&mut e, &c, K::Pause, mods(true, false, false, true), 1_000);
    assert_eq!(
        out[0],
        Outcome {
            disposition: Disposition::Swallow,
            actions: vec![Action::ConvertSelection]
        }
    );
    // голый Pause — «отменить / конвертировать слово»; отменять нечего
    let out = press_at(&mut e, &c, K::Pause, mods(false, false, false, true), 2_000);
    assert_eq!(
        out[0],
        Outcome {
            disposition: Disposition::Swallow,
            actions: vec![]
        }
    );
}

#[test]
fn disabled_convert_selection_hotkey_does_nothing() {
    let (us, ru) = (us(), ru_pc());
    let mut cfg = config();
    cfg.hotkeys.convert_selection = vec![];
    let mut e = engine_with(cfg, vec![us.clone(), ru.clone()]);
    let c = ctx(&ru);
    assert_eq!(actions(&alt_shift_chord(&mut e, &c, 1_000)), vec![]);
    assert_eq!(e.tick(LATER), vec![]);
}

// --- Слова-исключения ---------------------------------------------------------------------

#[test]
fn word_in_never_list_is_not_corrected() {
    let (us, ru) = (us(), ru_pc());
    let mut cfg = config();
    cfg.words.never = vec!["ghbdtn".to_owned()];
    let mut e = engine_with(cfg, vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(space(&mut e, &c), Outcome::pass());
    type_text(&mut e, &c, &us, "Ghbdtn");
    assert_eq!(space(&mut e, &c), Outcome::pass());
}

#[test]
fn never_list_takes_no_one_or_two_letter_words() {
    assert!(!WordRules::accepts("z"));
    assert!(!WordRules::accepts(" rf "));
    assert!(WordRules::accepts("ghbdtn"));
    let mut words = WordRules::default();
    assert!(!words.add("rf"));
    assert!(words.add("ghbdtn"));
    assert!(!words.add("Ghbdtn"), "повтор не добавляется");
    assert_eq!(words.never, vec!["ghbdtn".to_owned()]);
}

// --- Конвертация выделения ----------------------------------------------------------------

#[test]
fn convert_text_goes_by_the_majority_script() {
    let (us, ru) = (us(), ru_pc());
    let e = engine(vec![us.clone(), ru.clone()]);
    assert_eq!(
        e.convert_text("Ghbdtn? rfr ltkf&").as_deref(),
        Some("Привет, как дела?")
    );
    assert_eq!(
        e.convert_text("Привет, как дела?").as_deref(),
        Some("Ghbdtn? rfr ltkf&")
    );
    assert_eq!(e.convert_text(""), None);
    assert_eq!(e.convert_text("123 - 45"), None);
    // поровну латиницы и кириллицы — «иначе в английскую»
    assert_eq!(e.convert_text("zя").as_deref(), Some("zz"));
}

#[test]
fn shift_release_lost_behind_secure_input_does_not_count_as_both_shifts() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    // отпускание левого Shift ушло в поле пароля и до движка не дошло
    key_at(&mut e, &c, K::ShiftLeft, KeyDir::Down, shift(), 1_000);
    e.on_reset(ResetReason::SecureInput);
    let tap = shift_tap(&mut e, &c, K::ShiftRight, 5_000);
    assert_eq!(actions(&tap), vec![]);
    assert_eq!(e.tick(LATER), vec![Action::SwitchLayout(ru.id.clone())]);
}

#[test]
fn click_after_single_shift_cancels_the_pending_switch() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru]);
    let c = ctx(&us);
    shift_tap(&mut e, &c, K::ShiftLeft, 1_000);
    assert!(
        e.next_deadline().is_some(),
        "предусловие: одиночный Shift ждёт второго"
    );
    e.on_reset(ResetReason::MouseClick);
    assert_eq!(e.next_deadline(), None);
    assert_eq!(e.tick(LATER), vec![], "Shift с кликом не переключает");
}
