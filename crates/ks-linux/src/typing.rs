//! Ввод текста физическими клавишами: у uinput нет «строки Unicode», поэтому любой текст
//! печатается клавишами раскладки, в которой он набирается.

use ks_core::{Action, KeyStroke, LayoutMap, LayoutSet, ReplaceKind};

fn whole(map: &LayoutMap, text: &str) -> Option<Vec<KeyStroke>> {
    text.chars().map(|c| map.stroke_for(c)).collect()
}

/// Нажатия для `text`: в текущей раскладке, если она набирает весь текст, иначе — в первой
/// раскладке, которая набирает его целиком. Запасной путь замены: раскладка не переключилась —
/// клавиши раскладки цели в прежней раскладке печатают исходное слово, и текст не теряется.
pub fn strokes_for(
    text: &str,
    current: Option<&LayoutMap>,
    set: &LayoutSet,
) -> Option<Vec<KeyStroke>> {
    current
        .and_then(|m| whole(m, text))
        .or_else(|| set.0.iter().find_map(|m| whole(m, text)))
}

/// Конвертированное выделение → ввод поверх выделения (`erase` = 0: первая же клавиша
/// заменяет выделенное). Раскладка цели — та из пары, в которой набирается больше букв.
/// `None` — нет пары раскладок или символ не набирается.
pub fn selection_action(converted: &str, set: &LayoutSet) -> Option<Action> {
    let (en, ru) = set.pair()?;
    let letters = |m: &LayoutMap| {
        converted
            .chars()
            .filter(|c| c.is_alphabetic() && m.stroke_for(*c).is_some())
            .count()
    };
    let target = if letters(ru) > letters(en) { ru } else { en };
    let keys = whole(target, converted)?;
    Some(Action::Replace {
        erase: 0,
        keys,
        text: converted.to_owned(),
        target_layout: Some(target.id.clone()),
        kind: ReplaceKind::Manual,
    })
}
