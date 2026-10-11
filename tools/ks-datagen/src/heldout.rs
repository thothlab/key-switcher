//! Проверка отложенности корпуса оценки: слово корпуса, которого нет в сырой развёртке
//! hunspell, не должно оказаться ни в словаре (fst), ни в обучающих данных LM.

use std::collections::BTreeSet;

use anyhow::Result;

/// Слова корпуса вне сырой развёртки, попавшие во fst словаря или в обучение LM.
pub fn violations(
    eval_tokens: &BTreeSet<String>,
    raw_expansion: &BTreeSet<String>,
    fst_bytes: &[u8],
    lm_train: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let dictionary = fst::Set::new(fst_bytes)?;
    Ok(eval_tokens
        .iter()
        .filter(|t| !raw_expansion.contains(*t))
        .filter(|t| dictionary.contains(t.as_bytes()) || lm_train.contains(*t))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(words: &[&str]) -> BTreeSet<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn fst_of(words: &[&str]) -> Vec<u8> {
        let mut builder = fst::SetBuilder::memory();
        for word in set(words) {
            builder.insert(word).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn corpus_word_planted_into_the_dictionary_is_caught() {
        let raw = set(&["дом", "привет"]);
        let eval = set(&["норм", "привет"]);
        let fst = fst_of(&["дом", "норм", "привет"]);
        assert_eq!(violations(&eval, &raw, &fst, &raw).unwrap(), ["норм"]);
    }

    #[test]
    fn corpus_word_planted_into_lm_training_is_caught() {
        let raw = set(&["дом", "привет"]);
        let eval = set(&["норм", "привет"]);
        let lm = set(&["дом", "норм", "привет"]);
        let fst = fst_of(&["дом", "привет"]);
        assert_eq!(violations(&eval, &raw, &fst, &lm).unwrap(), ["норм"]);
    }

    #[test]
    fn corpus_words_from_hunspell_are_not_leaks() {
        let raw = set(&["дом", "привет"]);
        let eval = set(&["норм", "привет"]);
        let fst = fst_of(&["дом", "привет"]);
        assert!(violations(&eval, &raw, &fst, &raw).unwrap().is_empty());
    }
}
