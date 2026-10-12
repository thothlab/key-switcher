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

/// Ревью 07: встроенный список — только приложения текущей ОС, чужие id в него не попадают.
#[test]
fn builtin_excluded_apps_belong_to_the_current_os() {
    let lex = Lexicon::embedded();
    let apps = lex.default_excluded_apps();
    let terminals = [
        ("macos", "com.apple.Terminal"),
        ("windows", "windowsterminal.exe"),
        ("linux", "org.gnome.Terminal"),
    ];
    let current = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    for (os, id) in terminals {
        let listed = apps.contains(&AppId(id.to_owned()));
        assert_eq!(listed, os == current, "{os}: {id}");
    }
}

/// Окно настроек показывает названия, а не идентификаторы: у каждой строки встроенного списка
/// есть название, JetBrains и Android Studio — одно семейство, список совпадает с вето движка.
#[test]
fn builtin_apps_have_names_and_jetbrains_is_one_family() {
    let lex = Lexicon::embedded();
    let apps = lex.builtin_apps();
    let ids: Vec<AppId> = apps.iter().map(|a| a.id.clone()).collect();
    assert_eq!(ids, lex.default_excluded_apps());
    assert!(apps.iter().all(|a| !a.name.is_empty() && a.name != a.id.0));
    let studio = if cfg!(target_os = "macos") {
        "com.google.android.studio"
    } else if cfg!(windows) {
        "studio64.exe"
    } else {
        "jetbrains-studio"
    };
    let family = |id: &str| apps.iter().find(|a| a.id.0 == id).and_then(|a| a.family);
    assert_eq!(family(studio), Some("JetBrains IDE и Android Studio"));
    let jetbrains = apps.iter().filter(|a| a.family == family(studio)).count();
    assert!(jetbrains >= 5, "семейство JetBrains: {jetbrains}");
}

/// Слабые формы (задача 11): редкая форма ослаблена, частые коллизии истории 12 — нет.
#[test]
fn weak_forms_cover_rare_forms_but_not_protected_collisions() {
    let lex = Lexicon::embedded();
    assert!(lex.is_weak(Lang::Ru, "еру"));
    assert!(lex.is_weak(Lang::En, "dbl"));
    for word in ["руку", "рук", "руки", "луны", "ешь", "еще", "ещё", "муки"]
    {
        assert!(lex.contains(Lang::Ru, word), "{word}");
        assert!(!lex.is_weak(Lang::Ru, word), "{word}");
    }
    for word in ["here", "her", "herb", "keys", "tim", "the"] {
        assert!(!lex.is_weak(Lang::En, word), "{word}");
    }
    assert!(!lex.is_weak(Lang::En, "еру"));
    assert!(!lex.is_weak(Lang::Other, "еру"));
}
