use ks_data::{Lang, Lexicon};

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

// В нынешних основах у каждого слова с «ё» есть двойник с «е», поэтому нормализацию ё→е
// этот тест отличить не может; проверка на развёрнутых формах — задача 03.
#[test]
fn both_yo_and_ye_spellings_are_found() {
    let lex = Lexicon::embedded();
    assert!(lex.contains(Lang::Ru, "ёлка"));
    assert!(lex.contains(Lang::Ru, "елка"));
}
