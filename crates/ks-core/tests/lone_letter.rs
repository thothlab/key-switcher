//! Однобуквенный союз или предлог перед исправляемым словом исправляется вместе с ним
//! («B gjkexftncz» → «И получается»); сам по себе, без контекста, он не трогается.

mod common;

use common::*;
use ks_core::{Action, Modifiers, Outcome, PhysicalKey as K, ReplaceKind};

fn replace_text(out: &Outcome) -> Option<(usize, String)> {
    out.actions.iter().find_map(|a| match a {
        Action::Replace { erase, text, .. } => Some((*erase, text.clone())),
        _ => None,
    })
}

#[test]
fn lone_letter_goes_with_the_next_corrected_word() {
    for ru in [ru_pc(), ru_apple()] {
        let us = us();
        let mut e = engine(vec![us.clone(), ru.clone()]);
        let c = ctx(&us);
        type_text(&mut e, &c, &us, "B");
        assert_eq!(
            space(&mut e, &c),
            Outcome::pass(),
            "одна буква сама не трогается"
        );
        type_text(&mut e, &c, &us, "gjkexftncz");
        let out = space(&mut e, &c);
        assert_eq!(
            replace_text(&out),
            Some((12, "И получается ".to_owned())),
            "{}",
            ru.name
        );
        assert!(
            out.actions.iter().any(|a| matches!(a,
                Action::Replace { target_layout: Some(id), .. } if *id == ru.id)),
            "раскладка — русская"
        );
    }
}

#[test]
fn english_word_letter_is_not_joined() {
    // «a» конвертируется в «ф» — это не однобуквенное русское слово
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "a");
    space(&mut e, &c);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        replace_text(&space(&mut e, &c)),
        Some((6, "привет ".to_owned()))
    );
}

#[test]
fn letter_typed_in_the_other_layout_is_not_joined() {
    // «и» набрано в русской раскладке правильно, а слово после — в английской
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    type_text(&mut e, &ctx(&ru), &ru, "и");
    space(&mut e, &ctx(&ru));
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "ghbdtn");
    assert_eq!(
        replace_text(&space(&mut e, &c)),
        Some((6, "привет ".to_owned()))
    );
}

#[test]
fn erased_space_breaks_the_join() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "B");
    space(&mut e, &c);
    press(&mut e, &c, K::Backspace, Modifiers::default());
    press(&mut e, &c, K::Backspace, Modifiers::default());
    type_text(&mut e, &c, &us, "gjkexftncz");
    assert_eq!(
        replace_text(&space(&mut e, &c)),
        Some((10, "получается ".to_owned()))
    );
}

#[test]
fn undo_returns_both_the_letter_and_the_word() {
    let (us, ru) = (us(), ru_pc());
    let mut e = engine(vec![us.clone(), ru.clone()]);
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "B");
    space(&mut e, &c);
    type_text(&mut e, &c, &us, "gjkexftncz");
    space(&mut e, &c);
    let undo = actions(&double_shift(&mut e, &ctx(&ru), 10_000));
    let back = undo.iter().find_map(|a| match a {
        Action::Replace {
            kind: ReplaceKind::Undo,
            erase,
            text,
            target_layout,
            ..
        } => Some((*erase, text.clone(), target_layout.clone())),
        _ => None,
    });
    assert_eq!(
        back,
        Some((13, "B gjkexftncz ".to_owned(), Some(us.id.clone())))
    );
}
