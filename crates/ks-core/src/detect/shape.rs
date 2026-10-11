//! Вид токена: что не похоже на прозу (PRD, история 11) и что похоже на пароль (история 17).
//! Знак набранного, который в другой раскладке — буква (`cgfcb,j` → «спасибо»), знаком не
//! считается; краевой знак, который в другой раскладке — обычная пунктуация (`@ghbdtn@` →
//! «"привет"», `ltkf&` → «дела?»), структурным не считается (`analysis/detection.md` §8, §9).

use std::ops::Range;

use crate::Level;

/// Пунктуация, которой слово может начинаться.
pub(super) fn is_leading_punct(c: char) -> bool {
    matches!(c, '"' | '(' | '«' | '-')
}

/// Пунктуация, которой слово может кончаться.
pub(super) fn is_trailing_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | ')' | '»' | '-'
    )
}

fn is_edge_punct(c: char) -> bool {
    is_leading_punct(c) || is_trailing_punct(c)
}

/// Знаки кода, адресов и путей; `/` — если их в токене два и больше.
fn is_structural(c: char, slashes: usize) -> bool {
    "@:\\|\"<>{}=#&_".contains(c) || (c == '/' && slashes >= 2)
}

fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

fn has_caps(level: Level) -> bool {
    matches!(level, Level::Caps | Level::ShiftCaps)
}

/// Позиции от первой до последней буквы.
pub(super) fn letter_span(chars: &[char]) -> Option<Range<usize>> {
    let first = chars.iter().position(|c| c.is_alphabetic())?;
    let last = chars.iter().rposition(|c| c.is_alphabetic())?;
    Some(first..last + 1)
}

/// Заглавная сразу после строчной: camelCase, `iPhone`.
fn inner_capital(typed: &[char]) -> bool {
    typed
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase())
}

/// Цифры, структурные знаки, смешение алфавитов, camelCase, ВСЕ ЗАГЛАВНЫЕ через Shift,
/// четыре и больше одинаковых букв подряд, слово из одной повторённой буквы. `typed`, `conv` и `levels` — по символу на нажатие.
pub(super) fn not_prose(typed: &[char], conv: &[char], levels: &[Level]) -> bool {
    let Some(span) = letter_span(typed) else {
        return true;
    };
    let slashes = typed.iter().filter(|&&c| c == '/').count();
    let structural = (0..typed.len()).any(|i| {
        is_structural(typed[i], slashes)
            && !conv[i].is_alphabetic()
            && !(!span.contains(&i) && is_edge_punct(conv[i]))
    });
    let letters = || typed.iter().zip(levels).filter(|(c, _)| c.is_alphabetic());
    let mixed = letters().any(|(c, _)| is_cyrillic(*c)) && letters().any(|(c, _)| !is_cyrillic(*c));
    let shouted =
        letters().nth(1).is_some() && letters().all(|(c, l)| c.is_uppercase() && !has_caps(*l));
    let stretched = typed.windows(4).any(|w| {
        w[0].is_alphabetic() && w.iter().all(|c| c.to_lowercase().eq(w[0].to_lowercase()))
    });
    // слово из одной повторённой буквы — междометие или число: «шшш», «ммм», «iii»
    let one_letter = letters().nth(2).is_some() && {
        let first = letters()
            .next()
            .map(|(c, _)| c.to_lowercase().collect::<String>());
        letters().all(|(c, _)| Some(c.to_lowercase().collect::<String>()) == first)
    };
    typed.iter().any(|c| c.is_numeric())
        || structural
        || mixed
        || inner_capital(typed)
        || shouted
        || stretched
        || one_letter
}

/// Больше шести знаков и знак, который не буква ни в одной раскладке и не обычная краевая
/// пунктуация (`PAssword%`). Цифры и заглавные внутри отсекает [`not_prose`] раньше.
pub(super) fn password_like(typed: &[char], conv: &[char]) -> bool {
    if typed.len() <= 6 {
        return false;
    }
    let Some(span) = letter_span(typed) else {
        return false;
    };
    (0..typed.len()).any(|i| {
        let c = typed[i];
        !c.is_alphanumeric()
            && !ks_data::is_word_char(c)
            && !conv[i].is_alphabetic()
            && !(!span.contains(&i) && (is_edge_punct(c) || is_edge_punct(conv[i])))
    })
}

/// Две первые заглавные, остальные строчные, всё — буквы, без CapsLock.
pub(super) fn two_capitals(chars: &[char], levels: &[Level], min: usize) -> bool {
    chars.len() >= min
        && chars.iter().all(|c| c.is_alphabetic())
        && chars[0].is_uppercase()
        && chars[1].is_uppercase()
        && chars[2..].iter().all(|c| c.is_lowercase())
        && !levels.iter().any(|l| has_caps(*l))
}
