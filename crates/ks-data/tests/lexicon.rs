use ks_data::{AppId, Lang, Lexicon};

#[test]
fn embedded_dictionaries_know_words_of_both_languages() {
    let lex = Lexicon::embedded();
    assert!(lex.contains(Lang::En, "good"));
    assert!(lex.contains(Lang::En, "morning"));
    assert!(lex.contains(Lang::Ru, "привет"));
    assert!(lex.contains(Lang::Ru, "спасибо"));
    assert!(!lex.contains(Lang::En, "ghbdtn"));
    assert!(!lex.contains(Lang::Ru, "пщщв"));
    assert!(!lex.contains(Lang::Other, "good"));
}

/// Формы, которых в .dic нет: их даёт только развёртка аффиксов.
#[test]
fn dictionaries_hold_inflected_forms() {
    let lex = Lexicon::embedded();
    for word in ["дома", "приветами", "делаешь", "сказала"] {
        assert!(lex.contains(Lang::Ru, word), "ru: {word}");
    }
    for word in ["here", "mornings", "dog's", "walked"] {
        assert!(lex.contains(Lang::En, word), "en: {word}");
    }
    assert!(!lex.contains(Lang::En, "привет"));
    assert!(!lex.contains(Lang::Ru, "good"));
}

/// «ещё» и «идём» в hunspell записаны только через «ё»: находятся и по «е», и по «ё».
#[test]
fn yo_and_ye_spellings_are_the_same_word() {
    let lex = Lexicon::embedded();
    for (yo, ye) in [("ещё", "еще"), ("идём", "идем"), ("ёлка", "елка")] {
        assert!(lex.contains(Lang::Ru, yo), "{yo}");
        assert!(lex.contains(Lang::Ru, ye), "{ye}");
    }
}

/// Настоящее слово оценивается выше, чем те же нажатия в другой раскладке. Обе строки — из
/// алфавита модели: сравниваются сочетания букв, а не штраф за чужой символ.
#[test]
fn lm_prefers_real_words_over_wrong_layout_strings() {
    let lex = Lexicon::embedded();
    let pairs = [
        (Lang::Ru, "привет", "руддщ"),       // hello в русской раскладке
        (Lang::Ru, "спасибо", "пщщв"),       // good в русской раскладке
        (Lang::Ru, "дома", "рщьу"),          // home в русской раскладке
        (Lang::Ru, "остроконечный", "ыъьй"), // редкая форма против невиданных сочетаний
        (Lang::En, "hello", "ghbdtn"),       // привет в английской раскладке
        (Lang::En, "morning", "rfrltkf"),    // какдела в английской раскладке
        (Lang::En, "good", "ltkf"),          // дела в английской раскладке
    ];
    for (lang, real, wrong) in pairs {
        let (good, bad) = (lex.lm_score(lang, real), lex.lm_score(lang, wrong));
        assert!(good > bad, "{lang:?}: {real} {good} <= {wrong} {bad}");
    }
}

#[test]
fn short_word_lists_hold_frequent_words_only() {
    let lex = Lexicon::embedded();
    assert!(lex.short_words(Lang::Ru, 1).contains("я"));
    assert!(lex.short_words(Lang::Ru, 2).contains("не"));
    assert!(lex.short_words(Lang::En, 1).contains("i"));
    assert!(lex.short_words(Lang::En, 2).contains("of"));
    assert!(!lex.short_words(Lang::En, 2).contains("lf"));
    assert!(!lex.short_words(Lang::Ru, 2).contains("яя"));
    assert!(lex.short_words(Lang::En, 3).is_empty());
    assert!(lex.short_words(Lang::Other, 1).is_empty());
}

#[test]
fn terminals_of_every_os_are_excluded_by_default() {
    let lex = Lexicon::embedded();
    let apps = lex.default_excluded_apps();
    for id in [
        "com.apple.Terminal",
        "windowsterminal.exe",
        "org.gnome.Terminal",
    ] {
        assert!(apps.contains(&AppId(id.to_owned())), "{id}");
    }
}
