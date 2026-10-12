//! Правило распознавания (задача 05): сценарии `autocorrect/spec.md` и «Строки, похожие на
//! пароль» из `safety/spec.md` на шве `Engine::on_key` с настоящим `Lexicon`.

mod common;

use std::time::{Duration, Instant};

use common::*;
use ks_core::{
    Action, AppId, Config, Context, Engine, KeyDir, KeyStroke, LayoutMap, Modifiers, Os, Outcome,
    PhysicalKey as K, ReplaceKind, ResetReason,
};

/// Текст замены и её вид, если движок что-то заменил.
fn fix(outcome: &Outcome) -> Option<(String, ReplaceKind)> {
    outcome.actions.iter().find_map(|a| match a {
        Action::Replace { text, kind, .. } => Some((text.clone(), *kind)),
        _ => None,
    })
}

fn fixed_text(outcome: &Outcome) -> Option<String> {
    fix(outcome).map(|(text, _)| text)
}

/// Набрать слово и пробел; текст замены, если она была.
fn word(e: &mut Engine, c: &Context, map: &LayoutMap, text: &str) -> Option<String> {
    type_text(e, c, map, text);
    fixed_text(&space(e, c))
}

fn us_pc() -> (LayoutMap, LayoutMap, Engine) {
    let (us, ru) = (us(), ru_pc());
    let e = engine(vec![us.clone(), ru.clone()]);
    (us, ru, e)
}

fn caps() -> Modifiers {
    Modifiers {
        caps_lock: true,
        ..Modifiers::default()
    }
}

// --- Сохранение регистра ------------------------------------------------------------------

#[test]
fn caps_lock_word_is_replaced_in_upper_case() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    for key in [K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN] {
        assert_eq!(press(&mut e, &c, key, caps()), Outcome::pass());
    }
    let out = press(&mut e, &c, K::Space, caps());
    assert_eq!(fixed_text(&out).as_deref(), Some("ПРИВЕТ "));
}

// --- Таблицы клавиш: обе трактовки знака ------------------------------------------------------

#[test]
fn trailing_sign_is_tried_as_a_letter_and_as_a_sign() {
    let (us, ru, mut e) = us_pc();
    let c = ctx(&us);
    // «.» — буква «ю»: «думаю», а не «дума.»
    assert_eq!(word(&mut e, &c, &us, "levf.").as_deref(), Some("думаю "));
    // «,» буквой даёт «приветб» — не слово; знаком — «привет,», запятая печатается клавишей «ПК»
    type_text(&mut e, &c, &us, "ghbdtn,");
    let out = space(&mut e, &c);
    let mut keys = plain(&[K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN]);
    // запятая в «Русская – ПК» — Shift+Slash (фикстура ru_pc)
    keys.push(KeyStroke {
        key: K::Slash,
        shift: true,
    });
    keys.push(KeyStroke {
        key: K::Space,
        shift: false,
    });
    assert_eq!(out, replaced("привет, ", 7, keys, &ru));
}

#[test]
fn shifted_digits_at_the_edges_are_russian_punctuation() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    assert_eq!(
        word(&mut e, &c, &us, "@ghbdtn@").as_deref(),
        Some("\"привет\" ")
    );
    assert_eq!(word(&mut e, &c, &us, "ltkf&").as_deref(), Some("дела? "));
}

// --- Защита настоящих слов ------------------------------------------------------------------

#[test]
fn collision_pairs_are_kept_in_both_directions() {
    let (us, ru, mut e) = us_pc();
    let (in_us, in_ru) = (ctx(&us), ctx(&ru));
    for en in ["here", "her", "herb", "bob", "keys", "tim", "hen", "iii"] {
        assert_eq!(word(&mut e, &in_us, &us, en), None, "EN");
    }
    for ru_word in ["руку", "рук", "руки", "ищи", "луны", "ешь", "рут", "шшш"]
    {
        assert_eq!(word(&mut e, &in_ru, &ru, ru_word), None, "RU");
    }
}

/// Задача 11: «еру» есть в ru_RU (имя буквы «ер»), но это «the» в русской раскладке. Редкая
/// форма из курируемого списка не защищает слово от исправления; частые пары — по-прежнему.
#[test]
fn rare_dictionary_form_does_not_shield_a_frequent_word() {
    let (us, ru, mut e) = us_pc();
    let (in_us, in_ru) = (ctx(&us), ctx(&ru));
    type_text(&mut e, &in_ru, &ru, "еру");
    let out = space(&mut e, &in_ru);
    assert_eq!(fixed_text(&out).as_deref(), Some("the "));
    // как в сценарии «Английская фраза в русской раскладке»: дальше — английская раскладка
    let target = out.actions.iter().find_map(|a| match a {
        Action::Replace { target_layout, .. } => target_layout.clone(),
        _ => None,
    });
    assert_eq!(target, Some(us.id.clone()));
    e.on_reset(ResetReason::External);
    assert_eq!(word(&mut e, &in_ru, &ru, "Еру").as_deref(), Some("The "));
    e.on_reset(ResetReason::External);
    assert_eq!(word(&mut e, &in_us, &us, "here"), None);
    e.on_reset(ResetReason::External);
    assert_eq!(word(&mut e, &in_ru, &ru, "руку"), None);
}

/// Обычная форма частого слова в список не входит: «туче» («туча») остаётся настоящим словом и
/// в начале фразы, без контекста, хотя её образ «next» — словарное слово.
#[test]
fn ordinary_form_of_a_common_word_is_still_protected() {
    let (_, ru, mut e) = us_pc();
    let in_ru = ctx(&ru);
    assert_eq!(word(&mut e, &in_ru, &ru, "туче"), None);
}

// --- Токены, не похожие на прозу ---------------------------------------------------------------

#[test]
fn addresses_and_identifiers_are_not_touched() {
    let (us, ru, mut e) = us_pc();
    let c = ctx(&us);
    for token in [
        "git@github.com",
        "/usr/local/bin",
        "v1.2.3",
        "kubectl",
        "HTTP",
        "iPhone",
        "ckjjjjdfff",
    ] {
        assert_eq!(
            word(&mut e, &c, &us, token),
            None,
            "токен {} знаков",
            token.len()
        );
    }
    assert_eq!(word(&mut e, &ctx(&ru), &ru, "слоооовааа"), None);
}

// --- Короткие слова ------------------------------------------------------------------------

#[test]
fn one_letter_word_follows_the_segment_context() {
    // раскладка в контексте не переключилась (бэкенд не успел или не смог): «z» набрано в US
    // после русского слова
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
    assert_eq!(word(&mut e, &c, &us, "z").as_deref(), Some("я "));
    assert_eq!(word(&mut e, &c, &us, "ljvf").as_deref(), Some("дома "));
}

#[test]
fn one_letter_word_without_context_is_kept() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    assert_eq!(word(&mut e, &c, &us, "z"), None);
    // контекст умирает вместе с буфером: Tab, клик
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
    press(&mut e, &c, K::Tab, Modifiers::default());
    assert_eq!(word(&mut e, &c, &us, "z"), None);
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
    e.on_reset(ResetReason::MouseClick);
    assert_eq!(word(&mut e, &c, &us, "z"), None);
}

#[test]
fn two_letter_words_go_by_the_lists() {
    let (us, ru, mut e) = us_pc();
    let c = ctx(&us);
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
    assert_eq!(word(&mut e, &c, &us, "lf").as_deref(), Some("да "));
    e.on_reset(ResetReason::External);
    assert_eq!(word(&mut e, &c, &us, "lf").as_deref(), Some("да "));
    // «vs» ↔ «мы» — в обоих списках: без контекста решает LM: «vs» → «мы», а «мы» остаётся
    e.on_reset(ResetReason::External);
    assert_eq!(word(&mut e, &c, &us, "vs").as_deref(), Some("мы "));
    e.on_reset(ResetReason::External);
    assert_eq!(word(&mut e, &ctx(&ru), &ru, "мы"), None);
    assert_eq!(word(&mut e, &c, &us, "to"), None);
}

// --- Правленое и неполное слово ------------------------------------------------------------

#[test]
fn backspace_on_empty_buffer_has_no_effect() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
    assert_eq!(
        press(&mut e, &c, K::Backspace, Modifiers::default()),
        Outcome::pass()
    );
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
}

#[test]
fn rest_of_the_word_after_layout_change_is_not_corrected() {
    let (us, ru, mut e) = us_pc();
    type_text(&mut e, &ctx(&us), &us, "g");
    // раскладка сменилась посреди слова: «руддщ» само по себе стало бы «hello»
    let in_ru = ctx(&ru);
    assert_eq!(word(&mut e, &in_ru, &ru, "руддщ"), None);
    // флаг живёт до границы
    assert_eq!(
        word(&mut e, &in_ru, &ru, "руддщ").as_deref(),
        Some("hello ")
    );
}

#[test]
fn rest_of_the_word_after_a_key_without_symbol_is_not_corrected() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    type_text(&mut e, &c, &us, "x");
    press(&mut e, &c, K::Other(0x1_0042), Modifiers::default());
    assert_eq!(word(&mut e, &c, &us, "ghbdtn"), None);
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
}

// --- Случайные две заглавные ---------------------------------------------------------------

#[test]
fn two_accidental_capitals_are_fixed_in_place() {
    let (us, ru, mut e) = us_pc();
    type_text(&mut e, &ctx(&ru), &ru, "ПРивет");
    let out = space(&mut e, &ctx(&ru));
    assert_eq!(
        fix(&out),
        Some(("Привет ".to_owned(), ReplaceKind::CaseFix))
    );
    match &out.actions[0] {
        Action::Replace {
            erase,
            target_layout,
            ..
        } => assert_eq!((*erase, target_layout.clone()), (6, None)),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        fix(&{
            type_text(&mut e, &ctx(&us), &us, "HEllo");
            space(&mut e, &ctx(&us))
        }),
        Some(("Hello ".to_owned(), ReplaceKind::CaseFix))
    );
}

#[test]
fn abbreviations_are_not_case_fixed() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    for token in ["IDs", "MHz"] {
        assert_eq!(word(&mut e, &c, &us, token), None);
    }
}

// --- Пароли и исключения ------------------------------------------------------------------

#[test]
fn password_like_strings_are_never_corrected() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    // «PAssword%» ловит только правило пароля: без него это «Password%» (две заглавные)
    for token in ["hunter2X", "P@ssw0rd", "PAssword1", "PAssword%"] {
        assert_eq!(word(&mut e, &c, &us, token), None);
    }
}

#[test]
fn excluded_apps_follow_the_builtin_list_and_user_rules() {
    let (us, ru) = (us(), ru_pc());
    // встроенный список — только для текущей ОС (ревью 07)
    let terminal_id = if cfg!(target_os = "macos") {
        "com.apple.Terminal"
    } else if cfg!(windows) {
        "windowsterminal.exe"
    } else {
        "org.gnome.Terminal"
    };
    let terminal = Context {
        app: AppId(terminal_id.to_owned()),
        ..ctx(&us)
    };
    let mut e = engine(vec![us.clone(), ru.clone()]);
    assert_eq!(word(&mut e, &terminal, &us, "ghbdtn"), None);

    let mut config = Config::default_for(Os::MacOs);
    config.apps.enabled.push(terminal.app.clone());
    let mut e = engine_with(config, vec![us.clone(), ru.clone()]);
    assert_eq!(
        word(&mut e, &terminal, &us, "ghbdtn").as_deref(),
        Some("привет ")
    );

    let mut config = Config::default_for(Os::MacOs);
    config.apps.disabled.push(ctx(&us).app);
    let mut e = engine_with(config, vec![us.clone(), ru]);
    assert_eq!(word(&mut e, &ctx(&us), &us, "ghbdtn"), None);
}

// --- Время решения ------------------------------------------------------------------------

#[test]
fn boundary_decision_fits_in_a_millisecond() {
    let (us, ru, mut e) = us_pc();
    let (in_us, in_ru) = (ctx(&us), ctx(&ru));
    let tokens = [
        "ghbdtn",
        "kubectl",
        "here",
        "cgfcb,j",
        "ckjdjcjxtnfybt",
        "z",
        "ПРивет",
    ];
    let (mut total, mut n) = (Duration::ZERO, 0u32);
    for _ in 0..150 {
        for token in tokens {
            let (map, c) = if token.is_ascii() {
                (&us, &in_us)
            } else {
                (&ru, &in_ru)
            };
            type_text(&mut e, c, map, token);
            let started = Instant::now();
            let _ = key_at(&mut e, c, K::Space, KeyDir::Down, Modifiers::default(), 0);
            total += started.elapsed();
            key_at(&mut e, c, K::Space, KeyDir::Up, Modifiers::default(), 0);
            n += 1;
        }
    }
    let avg = total / n;
    eprintln!(
        "on_key на границе слова: {} мкс в среднем за {n} решений",
        avg.as_micros()
    );
    assert!(avg < Duration::from_millis(1), "{avg:?}");
}

#[test]
fn key_without_symbol_on_empty_buffer_leaves_the_next_word_whole() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    press(&mut e, &c, K::Other(0x1_0042), Modifiers::default());
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
}

#[test]
fn undo_moves_the_segment_back_to_the_typed_language() {
    let (us, _, mut e) = us_pc();
    let c = ctx(&us);
    assert_eq!(word(&mut e, &c, &us, "ghbdtn").as_deref(), Some("привет "));
    let undo = actions(&double_shift(&mut e, &c, 1_000));
    assert!(undo.iter().any(|a| matches!(
        a,
        Action::Replace {
            kind: ReplaceKind::Undo,
            ..
        }
    )));
    // пользователь отказался от русского: «z» в US остаётся «z»
    assert_eq!(word(&mut e, &c, &us, "z"), None);
}

#[test]
fn undo_after_case_fix_returns_the_word_without_layout_change() {
    let (_, ru, mut e) = us_pc();
    let c = ctx(&ru);
    type_text(&mut e, &c, &ru, "ПРивет");
    space(&mut e, &c);
    let undo = actions(&double_shift(&mut e, &c, 1_000));
    match undo.as_slice() {
        [
            Action::Replace {
                erase,
                text,
                target_layout,
                kind: ReplaceKind::Undo,
                ..
            },
        ] => {
            assert_eq!((*erase, text.as_str()), (7, "ПРивет "));
            assert!(target_layout.as_ref().is_none_or(|id| *id == ru.id));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn letter_keys_and_trailing_punctuation_on_apple_and_windows_layouts() {
    let us = us();
    let c = ctx(&us);
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
    // «привет,»: в «Русская» (Apple) запятая — Shift+Digit6, в KBDRU — Shift+Slash
    for (ru, tail, comma) in [
        (ru_apple(), "ghbdtn^", K::Digit6),
        (ru_win(), "ghbdtn?", K::Slash),
    ] {
        let mut e = engine(vec![us.clone(), ru.clone()]);
        type_text(&mut e, &c, &us, "cgfcb,j");
        assert_eq!(
            space(&mut e, &c),
            replaced("спасибо ", 7, spasibo.clone(), &ru)
        );
        type_text(&mut e, &c, &us, tail);
        let mut keys = plain(&[K::KeyG, K::KeyH, K::KeyB, K::KeyD, K::KeyT, K::KeyN]);
        keys.push(KeyStroke {
            key: comma,
            shift: true,
        });
        keys.push(KeyStroke {
            key: K::Space,
            shift: false,
        });
        assert_eq!(
            space(&mut e, &c),
            replaced("привет, ", 7, keys, &ru),
            "{}",
            ru.name
        );
    }
}

#[test]
fn russian_abbreviation_in_capitals_is_kept_with_shift_and_caps_lock() {
    let (_, ru, mut e) = us_pc();
    let c = ctx(&ru);
    // «ИИ» через Shift — аббревиатура
    assert_eq!(word(&mut e, &c, &ru, "ИИ"), None);
    // под CapsLock: «bb» — не частое короткое слово, двухбуквенное правило его не даёт
    for _ in 0..2 {
        assert_eq!(press(&mut e, &c, K::KeyB, caps()), Outcome::pass());
    }
    assert_eq!(press(&mut e, &c, K::Space, caps()), Outcome::pass());
}
