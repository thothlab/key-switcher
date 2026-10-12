//! Задача 10 на шве `Engine::on_key`: отмена стандартным ⌘Z/Ctrl+Z (`manual-control/spec.md`,
//! «Отмена стандартным сочетанием»), память отменённых слов сеанса («Отменённое слово не
//! исправляется снова в этом сеансе») и контекст сегмента после ручной смены раскладки.

mod common;

use common::*;
use ks_core::{
    Action, Config, Context, Disposition, Engine, EngineStatus, KeyDir, KeyStroke, LayoutMap,
    Level, Modifiers, Os, Outcome, PhysicalKey as K, ReplaceKind, ResetReason,
};

const GHBDTN: [K; 6] = [K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN];

fn with_space(keys: &[K]) -> Vec<KeyStroke> {
    let mut strokes = plain(keys);
    strokes.push(KeyStroke {
        key: K::Space,
        shift: false,
    });
    strokes
}

fn meta() -> Modifiers {
    Modifiers {
        meta: true,
        ..Modifiers::default()
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

/// Сочетание с Z так, как его шлёт бэкенд: модификатор отдельным событием, Z с ним,
/// отпускание Z, отпускание модификатора. Исходы всех четырёх событий.
fn chord_z(e: &mut Engine, c: &Context, modifier: K, mods: Modifiers, t: u64) -> Vec<Outcome> {
    vec![
        key_at(e, c, modifier, KeyDir::Down, mods, t),
        key_at(e, c, K::KeyZ, KeyDir::Down, mods, t + 20),
        key_at(e, c, K::KeyZ, KeyDir::Up, mods, t + 60),
        key_at(e, c, modifier, KeyDir::Up, Modifiers::default(), t + 80),
    ]
}

fn cmd_z(e: &mut Engine, c: &Context, t: u64) -> Vec<Outcome> {
    chord_z(e, c, K::MetaLeft, meta(), t)
}

fn ctrl_z(e: &mut Engine, c: &Context, t: u64) -> Vec<Outcome> {
    chord_z(e, c, K::ControlLeft, ctrl(), t)
}

/// «привет » (7 символов с пробелом) стирается, печатается «ghbdtn », раскладка — US.
fn undo_ghbdtn(us: &LayoutMap) -> Action {
    Action::Replace {
        erase: 7,
        keys: with_space(&GHBDTN),
        text: "ghbdtn ".to_owned(),
        target_layout: Some(us.id.clone()),
        kind: ReplaceKind::Undo,
    }
}

/// Z проглочен, пока Cmd/Ctrl зажат — никаких действий; отмена уходит на отпускании
/// модификатора (иначе стирание станет Ctrl+Backspace / Cmd+Backspace).
fn swallowed_undo(us: &LayoutMap) -> Vec<Outcome> {
    vec![
        Outcome::pass(),
        Outcome {
            disposition: Disposition::Swallow,
            actions: vec![],
        },
        Outcome::pass(),
        Outcome {
            disposition: Disposition::Pass,
            actions: vec![undo_ghbdtn(us)],
        },
    ]
}

/// Одиночное нажатие Z с флагами `mods` без отдельного события модификатора: окно отмены
/// до Z ничто не закрыло.
fn bare_z(e: &mut Engine, c: &Context, mods: Modifiers, t: u64) -> Vec<Outcome> {
    vec![
        key_at(e, c, K::KeyZ, KeyDir::Down, mods, t),
        key_at(e, c, K::KeyZ, KeyDir::Up, mods, t + 40),
    ]
}

fn swallow() -> Outcome {
    Outcome {
        disposition: Disposition::Swallow,
        actions: vec![],
    }
}

fn is_undo(actions: &[Action]) -> bool {
    actions.iter().any(|a| {
        matches!(
            a,
            Action::Replace {
                kind: ReplaceKind::Undo,
                ..
            }
        )
    })
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

fn mac_engine(us: &LayoutMap, ru: &LayoutMap) -> Engine {
    engine(vec![us.clone(), ru.clone()]).with_os(Os::MacOs)
}

// --- Отмена стандартным сочетанием ------------------------------------------------------

#[test]
fn cmd_z_right_after_correction_undoes_it_on_cmd_release_and_the_app_gets_nothing() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    assert_eq!(cmd_z(&mut e, &ctx(&ru), 1_000), swallowed_undo(&us));
    assert_eq!(e.last_undone(), Some("ghbdtn"));
}

#[test]
fn held_cmd_z_autorepeat_is_swallowed_and_undo_waits_for_cmd_release() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    let c = ctx(&ru);
    key_at(&mut e, &c, K::MetaLeft, KeyDir::Down, meta(), 1_000);
    assert_eq!(
        key_at(&mut e, &c, K::KeyZ, KeyDir::Down, meta(), 1_020),
        swallow()
    );
    let mut repeat = event_at(K::KeyZ, KeyDir::Down, meta(), 1_500);
    repeat.repeat = true;
    assert_eq!(
        e.on_key(&repeat, &c),
        swallow(),
        "автоповтор не уходит в приложение"
    );
    assert_eq!(
        key_at(&mut e, &c, K::KeyZ, KeyDir::Up, meta(), 1_600),
        Outcome::pass()
    );
    let release = key_at(
        &mut e,
        &c,
        K::MetaLeft,
        KeyDir::Up,
        Modifiers::default(),
        1_700,
    );
    assert_eq!(release.actions, vec![undo_ghbdtn(&us)]);
    assert_eq!(release.disposition, Disposition::Pass);
    // следующее, отдельное ⌘Z — уже собственная отмена приложения
    assert!(all_pass(&cmd_z(&mut e, &c, 2_000)));
}

#[test]
fn waiting_undo_is_dropped_by_another_key_a_second_z_or_a_click() {
    let (us, ru) = (us(), ru_apple());
    let c = ctx(&ru);
    type Interrupt = fn(&mut Engine, &Context) -> Outcome;
    let interrupts: [(&str, Interrupt); 3] = [
        ("второе нажатие Z", |e, c| {
            key_at(e, c, K::KeyZ, KeyDir::Down, meta(), 1_100)
        }),
        ("⌘V", |e, c| {
            key_at(e, c, K::KeyV, KeyDir::Down, meta(), 1_100)
        }),
        ("клик", |e, _| {
            e.on_reset(ks_core::ResetReason::MouseClick);
            Outcome::pass()
        }),
    ];
    for (name, interrupt) in interrupts {
        let mut e = mac_engine(&us, &ru);
        corrected_ghbdtn(&mut e, &us, &ru);
        key_at(&mut e, &c, K::MetaLeft, KeyDir::Down, meta(), 1_000);
        key_at(&mut e, &c, K::KeyZ, KeyDir::Down, meta(), 1_020);
        key_at(&mut e, &c, K::KeyZ, KeyDir::Up, meta(), 1_060);
        assert_eq!(interrupt(&mut e, &c), Outcome::pass(), "{name}");
        let release = key_at(
            &mut e,
            &c,
            K::MetaLeft,
            KeyDir::Up,
            Modifiers::default(),
            1_200,
        );
        assert_eq!(release, Outcome::pass(), "{name}: отмена не уходит");
        assert_eq!(e.last_undone(), None, "{name}");
    }
}

#[test]
fn releasing_cmd_or_ctrl_without_z_closes_the_undo_window() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    let c = ctx(&ru);
    key_at(&mut e, &c, K::MetaLeft, KeyDir::Down, meta(), 1_000);
    key_at(
        &mut e,
        &c,
        K::MetaLeft,
        KeyDir::Up,
        Modifiers::default(),
        1_100,
    );
    assert!(all_pass(&cmd_z(&mut e, &c, 2_000)));
    assert_eq!(e.last_undone(), None);

    // Windows/Linux: Ctrl+Shift сменил раскладку — ⇧⇧ после этого не отменяет исправление
    for os in [Os::Windows, Os::Linux] {
        let ru = ru_win();
        let mut e = engine_with(Config::default_for(os), vec![us.clone(), ru.clone()]).with_os(os);
        corrected_ghbdtn(&mut e, &us, &ru);
        let c = ctx(&ru);
        let both = Modifiers {
            shift: true,
            ..ctrl()
        };
        key_at(&mut e, &c, K::ControlLeft, KeyDir::Down, ctrl(), 1_000);
        key_at(&mut e, &c, K::ShiftLeft, KeyDir::Down, both, 1_020);
        key_at(&mut e, &c, K::ShiftLeft, KeyDir::Up, ctrl(), 1_060);
        key_at(
            &mut e,
            &c,
            K::ControlLeft,
            KeyDir::Up,
            Modifiers::default(),
            1_080,
        );
        let after = double_shift(&mut e, &c, 2_000);
        assert!(!is_undo(&actions(&after)), "{os:?}: {after:?}");
        assert_eq!(e.last_undone(), None, "{os:?}");
    }
}

#[test]
fn cmd_z_after_typing_on_passes_to_the_app() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    let c = ctx(&ru);
    type_text(&mut e, &c, &ru, "к");
    assert!(all_pass(&cmd_z(&mut e, &c, 1_000)));
    assert_eq!(e.last_undone(), None);
}

#[test]
fn cmd_z_after_a_correction_closed_by_enter_passes_to_the_app() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    let enter = press(&mut e, &c, K::Enter, Modifiers::default());
    assert!(
        matches!(enter.actions.as_slice(), [Action::Replace { text, .. }] if text == "привет\n"),
        "предусловие: {enter:?}"
    );
    // строка могла уйти: повторный \n отправил бы её второй раз
    assert!(all_pass(&cmd_z(&mut e, &ctx(&ru), 1_000)));
    assert_eq!(e.last_undone(), None);
}

#[test]
fn on_macos_only_cmd_z_undoes_ctrl_z_and_cmd_shift_z_pass() {
    let (us, ru) = (us(), ru_apple());

    // Z с Ctrl приходит в открытое окно и всё равно проходит в приложение
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    assert!(all_pass(&bare_z(&mut e, &ctx(&ru), ctrl(), 1_000)));
    assert_eq!(e.last_undone(), None);

    // ⌘⇧Z — «повторить», не отмена
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    let c = ctx(&ru);
    let redo = Modifiers {
        shift: true,
        ..meta()
    };
    key_at(&mut e, &c, K::ShiftLeft, KeyDir::Down, shift(), 1_000);
    let out = chord_z(&mut e, &c, K::MetaLeft, redo, 1_010);
    key_at(
        &mut e,
        &c,
        K::ShiftLeft,
        KeyDir::Up,
        Modifiers::default(),
        1_200,
    );
    assert!(all_pass(&out));
    assert_eq!(e.last_undone(), None);
}

#[test]
fn on_windows_and_linux_ctrl_z_undoes_and_win_z_passes() {
    for os in [Os::Windows, Os::Linux] {
        let (us, ru) = (us(), ru_win());
        let make =
            || engine_with(Config::default_for(os), vec![us.clone(), ru.clone()]).with_os(os);

        let mut e = make();
        corrected_ghbdtn(&mut e, &us, &ru);
        assert_eq!(
            ctrl_z(&mut e, &ctx(&ru), 1_000),
            swallowed_undo(&us),
            "{os:?}"
        );

        // Z с Win приходит в открытое окно и проходит в приложение
        let mut e = make();
        corrected_ghbdtn(&mut e, &us, &ru);
        assert!(
            all_pass(&bare_z(&mut e, &ctx(&ru), meta(), 1_000)),
            "{os:?}"
        );
        assert_eq!(e.last_undone(), None, "{os:?}");
    }
}

// --- Отменённое слово не исправляется снова в этом сеансе -------------------------------

#[test]
fn undone_word_typed_again_is_left_alone_until_restart() {
    let (us, ru) = (us(), ru_apple());
    let c = ctx(&us);
    let retype = |e: &mut Engine| {
        type_text(e, &c, &us, "ghbdtn");
        space(e, &c)
    };
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    assert_eq!(cmd_z(&mut e, &ctx(&ru), 1_000), swallowed_undo(&us));

    assert_eq!(
        retype(&mut e),
        Outcome::pass(),
        "снова «ghbdtn » — осталось как набрано"
    );
    // память сеанса переживает сброс слова и контекста и переключение автоматики
    e.on_reset(ResetReason::MouseClick);
    e.set_config(Config::default_for(Os::MacOs));
    assert_eq!(retype(&mut e), Outcome::pass(), "после клика и set_config");
    assert_eq!(e.status(), EngineStatus::Working);

    // после перезапуска (новый движок) память пуста: слово снова исправляется
    let mut fresh = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut fresh, &us, &ru);
}

#[test]
fn undone_word_with_a_trailing_sign_protects_the_bare_word() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    let c = ctx(&us);
    // «ghbdtn^» — «привет,» в «Русской» (Apple): запятая на Shift+Digit6
    type_text(&mut e, &c, &us, "ghbdtn^");
    let first = space(&mut e, &c);
    assert!(
        matches!(first.actions.as_slice(), [Action::Replace { text, .. }] if text == "привет, "),
        "предусловие: {first:?}"
    );
    assert!(is_undo(&actions(&cmd_z(&mut e, &ctx(&ru), 1_000))));
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        Outcome::pass(),
        "то же слово без знака не исправлено"
    );
}

#[test]
fn retyped_undone_word_can_still_be_converted_by_double_shift() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    corrected_ghbdtn(&mut e, &us, &ru);
    assert_eq!(cmd_z(&mut e, &ctx(&ru), 1_000), swallowed_undo(&us));
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(space(&mut e, &c), Outcome::pass());
    let conv = actions(&double_shift(&mut e, &c, 5_000));
    assert!(
        matches!(
            conv.as_slice(),
            [Action::Replace { kind: ReplaceKind::Manual, erase: 7, text, target_layout: Some(id), .. }]
                if text == "привет " && *id == ru.id
        ),
        "{conv:?}"
    );
}

/// Частые русские слова от пяти букв: набранные в US, они исправляются без контекста.
const RU_WORDS: [&str; 100] = [
    "привет",
    "спасибо",
    "слово",
    "работа",
    "город",
    "человек",
    "время",
    "жизнь",
    "голова",
    "сторона",
    "страна",
    "место",
    "вопрос",
    "сила",
    "отец",
    "земля",
    "конец",
    "часть",
    "голос",
    "машина",
    "книга",
    "письмо",
    "окно",
    "дверь",
    "улица",
    "школа",
    "деньги",
    "история",
    "работать",
    "говорить",
    "думать",
    "знать",
    "видеть",
    "хотеть",
    "сделать",
    "сказать",
    "понимать",
    "любовь",
    "солнце",
    "небо",
    "вода",
    "огонь",
    "ветер",
    "дорога",
    "комната",
    "ребенок",
    "женщина",
    "мужчина",
    "семья",
    "утро",
    "вечер",
    "ночь",
    "неделя",
    "месяц",
    "минута",
    "секунда",
    "система",
    "проблема",
    "решение",
    "компания",
    "кошка",
    "собака",
    "молоко",
    "хлеб",
    "погода",
    "праздник",
    "подарок",
    "музыка",
    "картина",
    "театр",
    "фильм",
    "песня",
    "новости",
    "здоровье",
    "счастье",
    "красивый",
    "хороший",
    "большой",
    "маленький",
    "новый",
    "старый",
    "быстро",
    "медленно",
    "сегодня",
    "завтра",
    "вчера",
    "лицо",
    "друг",
    "мама",
    "рука",
    "дело",
    "глаза",
    "вместе",
    "всегда",
    "теперь",
    "просто",
    "очень",
    "сейчас",
    "потом",
    "только",
];

#[test]
fn session_memory_keeps_only_the_last_64_undone_words() {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    let cu = ctx(&us);
    let in_us = |word: &str| -> String {
        word.chars()
            .map(|ch| {
                let s = ru.stroke_for(ch).expect("буква «Русской»");
                let level = if s.shift { Level::Shift } else { Level::Base };
                us.char_for(s.key, level).expect("клавиша US")
            })
            .collect()
    };
    let retype = |e: &mut Engine, typed: &str| {
        type_text(e, &cu, &us, typed);
        space(e, &cu)
    };
    let mut undone = Vec::new();
    for word in RU_WORDS {
        let typed = in_us(word);
        let out = retype(&mut e, &typed);
        if !matches!(
            out.actions.as_slice(),
            [Action::Replace {
                kind: ReplaceKind::Auto,
                ..
            }]
        ) {
            continue;
        }
        assert!(
            is_undo(&actions(&cmd_z(&mut e, &ctx(&ru), 1_000))),
            "{word}"
        );
        undone.push(typed);
        if undone.len() == 65 {
            break;
        }
    }
    assert_eq!(undone.len(), 65, "в списке мало слов, которые исправляются");
    assert_eq!(
        retype(&mut e, &undone[64]),
        Outcome::pass(),
        "последнее помнится"
    );
    assert_eq!(
        retype(&mut e, &undone[1]),
        Outcome::pass(),
        "второе ещё помнится"
    );
    assert!(
        matches!(
            retype(&mut e, &undone[0]).actions.as_slice(),
            [Action::Replace {
                kind: ReplaceKind::Auto,
                ..
            }]
        ),
        "самое старое из 65 вытеснено и снова исправляется"
    );
}

// --- Ручная смена раскладки переводит контекст сегмента ---------------------------------

/// Короткие русские слова, совпадающие нажатиями с английскими («мы» = «vs»).
const SHORT_RU: [&str; 7] = ["мы", "вы", "из", "ли", "во", "ок", "ИИ"];

/// «Slack » в US (контекст — английский), ручное переключение `switch` на русскую,
/// затем `word` с пробелом в русской раскладке; исход пробела.
fn after_english_then_switch(word: &str, switch: fn(&mut Engine, &LayoutMap)) -> Outcome {
    let (us, ru) = (us(), ru_apple());
    let mut e = mac_engine(&us, &ru);
    let cu = ctx(&us);
    type_text(&mut e, &cu, &us, "Slack");
    assert_eq!(
        space(&mut e, &cu),
        Outcome::pass(),
        "предусловие: «Slack» остаётся"
    );
    switch(&mut e, &us);
    let cr = ctx(&ru);
    type_text(&mut e, &cr, &ru, word);
    space(&mut e, &cr)
}

#[test]
fn single_shift_switch_moves_the_segment_to_the_new_layout() {
    for word in SHORT_RU {
        let out = after_english_then_switch(word, |e, us| {
            shift_tap(e, &ctx(us), K::ShiftLeft, 1_000);
            let switched = e.tick(60_000);
            assert!(matches!(switched.as_slice(), [Action::SwitchLayout(_)]));
        });
        assert_eq!(out, Outcome::pass(), "«Slack ⇧ {word}» → «{word}»");
    }
}

#[test]
fn os_layout_switch_seen_in_context_moves_the_segment() {
    for word in SHORT_RU {
        // Alt+Shift / Ctrl+Space ОС: движок видит только, что `Context.layout` стал русским
        let out = after_english_then_switch(word, |_, _| {});
        assert_eq!(
            out,
            Outcome::pass(),
            "«Slack», смена ОС, «{word}» → «{word}»"
        );
    }
}

#[test]
fn ctrl_space_layout_switch_does_not_leave_english_context() {
    for word in SHORT_RU {
        let out = after_english_then_switch(word, |e, us| {
            // ⌃Space macOS: пробел с Ctrl уходит в ОС, раскладка меняется на его нажатии
            let (cu, cr) = (ctx(us), ctx(&ru_apple()));
            key_at(e, &cu, K::ControlLeft, KeyDir::Down, ctrl(), 1_000);
            key_at(e, &cu, K::Space, KeyDir::Down, ctrl(), 1_020);
            key_at(e, &cr, K::Space, KeyDir::Up, ctrl(), 1_060);
            key_at(
                e,
                &cr,
                K::ControlLeft,
                KeyDir::Up,
                Modifiers::default(),
                1_080,
            );
        });
        assert_eq!(out, Outcome::pass(), "«Slack», ⌃Space, «{word}» → «{word}»");
    }
}
