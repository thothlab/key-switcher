//! Сценарии `autocorrect/spec.md` на шве `Engine::on_key` с фикстурами раскладок.

mod common;

use common::*;
use ks_core::{EngineStatus, KeyStroke, Level, Modifiers, Outcome, PhysicalKey as K, ResetReason};

const GHBDTN: [K; 6] = [K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN];

fn ghbdtn_then(boundary: K) -> Vec<KeyStroke> {
    let mut keys = plain(&GHBDTN);
    keys.push(KeyStroke {
        key: boundary,
        shift: false,
    });
    keys
}

#[test]
fn russian_word_in_us_layout_is_replaced_on_space_for_every_russian_layout() {
    for ru in [ru_apple(), ru_pc(), ru_win()] {
        let us = us();
        let mut e = engine(vec![us.clone(), ru.clone()]);
        let c = ctx(&us);
        assert!(all_pass(&type_text(&mut e, &c, &us, "ghbdtn")));
        assert_eq!(
            space(&mut e, &c),
            replaced("привет ", 6, ghbdtn_then(K::Space), &ru),
            "{}",
            ru.name
        );
    }
}

#[test]
fn english_phrase_in_russian_layout_is_replaced_and_next_english_word_is_kept() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let in_ru = ctx(&ru);
    assert!(all_pass(&type_text(&mut e, &in_ru, &ru, "пщщв")));
    let good = plain(&[K::KeyG, K::KeyO, K::KeyO, K::KeyD, K::Space]);
    assert_eq!(space(&mut e, &in_ru), replaced("good ", 4, good, &us));

    // бэкенд переключил раскладку: следующее слово набирается уже в US
    let in_us = ctx(&us);
    assert!(all_pass(&type_text(&mut e, &in_us, &us, "morning")));
    assert_eq!(space(&mut e, &in_us), Outcome::pass());
}

#[test]
fn capital_first_letter_is_kept() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "Ghbdtn");
    let mut keys = ghbdtn_then(K::Space);
    keys[0].shift = true;
    assert_eq!(space(&mut e, &c), replaced("Привет ", 6, keys, &ru));
}

#[test]
fn enter_is_a_boundary_like_space() {
    let (us, ru) = (us(), ru_apple());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        press(&mut e, &c, K::Enter, Modifiers::default()),
        replaced("привет\n", 6, ghbdtn_then(K::Enter), &ru)
    );
}

#[test]
fn tab_resets_the_word_without_decision() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        press(&mut e, &c, K::Tab, Modifiers::default()),
        Outcome::pass()
    );
    // после Tab слово пустое: следующее «ghbdtn» исправляется само по себе, шесть букв
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}

#[test]
fn mouse_click_resets_the_word() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    e.on_reset(ResetReason::MouseClick);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}

#[test]
fn command_shortcut_resets_the_word() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    let meta = Modifiers {
        meta: true,
        ..Modifiers::default()
    };
    assert_eq!(press(&mut e, &c, K::KeyA, meta), Outcome::pass());
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}

#[test]
fn backspace_inside_the_word_blocks_correction() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdt");
    press(&mut e, &c, K::Backspace, Modifiers::default());
    type_text(&mut e, &c, &us, "tn");
    assert_eq!(space(&mut e, &c), Outcome::pass());
    // флаг правки живёт до границы слова: следующее слово снова исправляется
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}

#[test]
fn letter_on_punctuation_key_and_trailing_punctuation_follow_the_russian_layout() {
    let us = us();
    let c = ctx(&us);

    let pc = ru_pc();
    let mut e = engine(vec![us.clone(), pc.clone()]);
    type_text(&mut e, &c, &us, "cgfcb,j");
    let spasibo = plain(&[
        K::KeyC,
        K::KeyG,
        K::KeyF,
        K::KeyC,
        K::KeyB,
        K::Comma,
        K::KeyJ,
        K::Space,
    ]);
    assert_eq!(space(&mut e, &c), replaced("спасибо ", 7, spasibo, &pc));

    type_text(&mut e, &c, &us, "ghbdtn?");
    let mut keys = plain(&GHBDTN);
    keys.push(KeyStroke {
        key: K::Slash,
        shift: true,
    });
    keys.push(KeyStroke {
        key: K::Space,
        shift: false,
    });
    assert_eq!(space(&mut e, &c), replaced("привет, ", 7, keys, &pc));

    let apple = ru_apple();
    let mut e = engine(vec![us.clone(), apple.clone()]);
    type_text(&mut e, &c, &us, "ghbdtn^");
    let mut keys = plain(&GHBDTN);
    keys.push(KeyStroke {
        key: K::Digit6,
        shift: true,
    });
    keys.push(KeyStroke {
        key: K::Space,
        shift: false,
    });
    assert_eq!(space(&mut e, &c), replaced("привет, ", 7, keys, &apple));
}

#[test]
fn layout_table_from_uckeytranslate_dump_reads_both_ways_and_fixes_latin_yo() {
    let pc = ru_pc();
    // в дампе «Русская – ПК» Shift+` — латинская Ë U+00CB
    assert_eq!(pc.char_for(K::Backquote, Level::Shift), Some('Ё'));
    assert_eq!(pc.char_for(K::Comma, Level::Base), Some('б'));
    assert_eq!(
        pc.stroke_for('б'),
        Some(KeyStroke {
            key: K::Comma,
            shift: false
        })
    );
    assert_eq!(us().char_for(K::KeyQ, Level::Shift), Some('Q'));
}

#[test]
fn without_russian_layout_nothing_is_replaced_and_status_says_so() {
    let us = us();
    let mut e = engine(vec![us.clone()]);
    let c = ctx(&us);
    assert_eq!(e.status(), EngineStatus::NeedsLayouts);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(space(&mut e, &c), Outcome::pass());

    // пара появилась — автоматика заработала
    let ru = ru_pc();
    e.set_layouts(ks_core::LayoutSet(vec![us.clone(), ru.clone()]));
    assert_eq!(e.status(), EngineStatus::Working);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}

#[test]
fn delete_inside_the_word_blocks_correction() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    press(&mut e, &c, K::Delete, Modifiers::default());
    assert_eq!(space(&mut e, &c), Outcome::pass());
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}

#[test]
fn arrow_key_forgets_the_word() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    for arrow in [K::ArrowLeft, K::ArrowRight, K::ArrowUp, K::ArrowDown] {
        type_text(&mut e, &c, &us, "ghbdtn");
        assert_eq!(
            press(&mut e, &c, arrow, Modifiers::default()),
            Outcome::pass()
        );
        assert_eq!(space(&mut e, &c), Outcome::pass(), "{arrow}");
    }
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        space(&mut e, &c),
        replaced("привет ", 6, ghbdtn_then(K::Space), &ru)
    );
}
