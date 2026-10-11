use std::collections::HashMap;
use std::path::Path;

use ks_core::{Config, LayoutMap, Lexicon, Os};
use ks_eval::metrics::{Counts, RecallOutcome, Tally};
use ks_eval::{
    EngineSubject, Layouts, Part, RecallWord, Subject, Variant, Word, load_corpus, mistype,
    run_live, run_recall, smoke_check, type_correct,
};

fn screen(intended: &str, from: &LayoutMap, typed: &LayoutMap, caps: bool) -> String {
    mistype(intended, from, typed, caps)
        .unwrap_or_else(|| panic!("нет синтетической ошибки для {intended}"))
        .screen
}

/// analysis/detection.md §7: «привет,» в US даёт `ghbdtn^` (Apple) и `ghbdtn?` («ПК», KBDRU).
#[test]
fn synthetic_error_follows_the_russian_layout_variant() {
    let l = Layouts::fixtures();
    assert_eq!(screen("привет,", &l.ru_apple, &l.us, false), "ghbdtn^");
    assert_eq!(screen("привет,", &l.ru_pc, &l.us, false), "ghbdtn?");
    assert_eq!(screen("привет,", &l.ru_win, &l.us, false), "ghbdtn?");
    assert_eq!(screen("good", &l.us, &l.ru_pc, false), "пщщв");
    assert_eq!(screen("Привет", &l.ru_apple, &l.us, false), "Ghbdtn");
}

/// CapsLock включён на всё слово: и буквы, и хвостовой знак набраны под ним.
#[test]
fn caps_lock_is_held_for_the_whole_word() {
    let l = Layouts::fixtures();
    let word = mistype("ПРИВЕТ,", &l.ru_pc, &l.us, true).expect("ПРИВЕТ,");
    assert_eq!(word.screen, "GHBDTN?");
    assert!(
        word.presses.iter().all(|p| p.caps),
        "CapsLock отпущен посреди слова"
    );
}

/// Подставной движок: исправляет то, что видно на экране, по таблице.
struct Fake(HashMap<&'static str, &'static str>);

impl Subject for Fake {
    fn reset(&mut self) {}
    fn type_word(&mut self, word: &Word) -> Option<String> {
        self.0.get(word.screen.as_str()).map(|s| (*s).to_owned())
    }
}

#[test]
fn false_positives_are_counted_per_10k_word_tokens_and_by_length() {
    let l = Layouts::fixtures();
    let mut fake = Fake(HashMap::from([("мир!", "vbh!")]));
    let mut tally = Tally::default();
    let corpus = ["Привет, мир!".to_owned(), "Я дома 5 минут.".to_owned()];
    run_live(&mut fake, &corpus, &l.ru_pc, &l.us, "RU", &mut tally);
    let row = tally.live_row("RU").expect("срез RU");
    // 5 токенов с буквами («5» без букв не считается), одно ложное исправление.
    assert_eq!(row.all, Counts { n: 5, hit: 1 });
    assert_eq!(row.all.per_10k(), 2000.0);
    assert_eq!(row.by_len[0], Counts { n: 1, hit: 0 }); // я
    assert_eq!(row.by_len[1], Counts { n: 0, hit: 0 });
    assert_eq!(row.by_len[2], Counts { n: 1, hit: 1 }); // мир
    assert_eq!(row.by_len[3], Counts { n: 3, hit: 0 }); // привет, дома, минут
}

#[test]
fn recall_needs_the_exact_intended_text_and_counts_untypable_words() {
    let l = Layouts::fixtures();
    let mut fake = Fake(HashMap::from([
        ("ghbdtn", "привет"),
        ("Ghbdtn", "Привет"),
        ("GHBDTN", "привет"), // регистр потерян — исправлено неверно
        ("ghbdtn?", "привет,"),
        ("rtr", "кек"),
    ]));
    let mut words: Vec<RecallWord> = (0..5)
        .map(|_| RecallWord {
            text: "привет".to_owned(),
            in_dict: true,
        })
        .collect();
    words.push(RecallWord {
        text: "кек".to_owned(),
        in_dict: false,
    });
    words.push(RecallWord {
        text: "ёжz".to_owned(), // «z» в русской раскладке не набрать
        in_dict: false,
    });
    let mut tally = Tally::default();
    // Варианты по кругу: обычный, Первая заглавная, CapsLock, «,» в конце, «.» в конце.
    run_recall(&mut fake, &words, &l.ru_pc, &l.us, "RU→US", &mut tally);
    let row = tally.recall_row("RU→US").expect("срез RU→US");
    assert_eq!(row.dict, Counts { n: 5, hit: 3 });
    assert_eq!(row.oov, Counts { n: 1, hit: 1 });
    assert_eq!(row.wrong, 1);
    assert_eq!(row.skipped, 1);
    assert_eq!(row.by_variant[2], Counts { n: 1, hit: 0 });
    assert_eq!(row.by_variant[4], Counts { n: 1, hit: 0 }); // «привет.» → ghbdtn/ не исправлено
}

/// Настоящий Lexicon и движок: исправление в обе стороны регистра и правильное слово не тронуто.
#[test]
fn engine_linkage_on_the_real_lexicon() {
    let l = Layouts::fixtures();
    let mut engine = EngineSubject::new(Lexicon::embedded(), vec![l.us.clone(), l.ru_pc.clone()]);
    for intended in ["привет", "Привет"] {
        engine.reset();
        let typo = mistype(intended, &l.ru_pc, &l.us, false).expect(intended);
        assert_eq!(
            engine.type_word(&typo).as_deref(),
            Some(intended),
            "{}",
            typo.screen
        );
    }
    engine.reset();
    assert_eq!(engine.type_word(&type_correct("привет", &l.ru_pc)), None);
}

#[test]
fn smoke_check_fails_when_the_engine_does_not_correct() {
    let l = Layouts::fixtures();
    let lex = Lexicon::embedded();
    assert_eq!(
        smoke_check(&lex, &l, &Config::default_for(Os::MacOs)),
        Ok(())
    );
    let mut off = Config::default_for(Os::MacOs);
    off.autocorrect = false;
    assert!(smoke_check(&lex, &l, &off).is_err());
}

/// Настроечная и контрольная части не пересекаются по id и вместе дают весь корпус.
#[test]
fn corpus_split_by_id_hash_is_disjoint_and_complete() {
    for id in 0..10_000u64 {
        assert!(Part::All.contains(id));
        assert_ne!(
            Part::Tune.contains(id),
            Part::Control.contains(id),
            "id {id}"
        );
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/eval");
    let all = load_corpus(&dir, Part::All, usize::MAX).unwrap();
    let tune = load_corpus(&dir, Part::Tune, usize::MAX).unwrap();
    let control = load_corpus(&dir, Part::Control, usize::MAX).unwrap();
    assert_eq!(tune.ru.len() + control.ru.len(), all.ru.len());
    assert_eq!(tune.en.len() + control.en.len(), all.en.len());
    assert!(tune.ru.len() * 3 > all.ru.len() && control.ru.len() * 3 > all.ru.len());
    assert_eq!(control.part, Part::Control);
}

#[test]
fn headline_skips_slices_without_dictionary_words() {
    let mut tally = Tally::default();
    tally.recall(
        "внесловарные",
        false,
        4,
        Variant::Lower,
        RecallOutcome::Missed,
    );
    tally.recall("словарные", true, 4, Variant::Lower, RecallOutcome::Fixed);
    tally.live("RU", 4, false);
    tally.adversarial("код · en · ПК", true);
    assert_eq!(tally.headline(), (0.0, 1.0));
    assert_eq!(
        tally.adversarial_row("код · en · ПК"),
        Some(&Counts { n: 1, hit: 1 })
    );
    assert_eq!(tally.adversarial_rows().count(), 1);
}
