//! Правило решения на границе слова — точка расширения задачи 05 (вето, LM, пороги).
//!
//! Сейчас минимум: набранного нет в словаре своего языка, а те же нажатия в другой раскладке
//! дают слово из словаря другого языка. Хвостовые знаки (`ghbdtn?` → `привет,`) в проверку
//! словарём не входят, но заменяются вместе со словом.

use crate::{Lang, Lexicon};

/// Знаки, допустимые в слове-кандидате помимо букв.
fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '-' || c == '\''
}

pub(crate) fn should_convert(
    typed: &str,
    typed_lang: Lang,
    candidate: &str,
    candidate_lang: Lang,
    lexicon: &Lexicon,
) -> bool {
    let typed_core = typed.trim_end_matches(|c: char| !c.is_alphabetic());
    let candidate_core = candidate.trim_end_matches(|c: char| !c.is_alphabetic());
    if typed_core.is_empty()
        || candidate_core.is_empty()
        || !candidate_core.chars().all(is_word_char)
    {
        return false;
    }
    !lexicon.contains(typed_lang, &typed_core.to_lowercase())
        && lexicon.contains(candidate_lang, &candidate_core.to_lowercase())
}
