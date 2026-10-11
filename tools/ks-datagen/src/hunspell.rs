//! hunspell: разбор `.aff` и развёртка словоформ `.dic` (свой unmunch).
//!
//! Поддерживается то, что есть в `ru_RU` и `en_US` из LibreOffice: однобуквенные флаги, PFX/SFX
//! с условиями (`.`, `[…]`, `[^…]`), cross product, NEEDAFFIX, FORBIDDENWORD, ONLYINCOMPOUND,
//! FULLSTRIP, флаги COMPOUNDRULE и прочие спецфлаги. Непредусмотренное — ошибка, а не пропуск.

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    pub fn parse(code: &str) -> Result<Lang> {
        match code {
            "en" => Ok(Lang::En),
            "ru" => Ok(Lang::Ru),
            other => bail!("неизвестный язык {other:?}"),
        }
    }

    pub fn is_letter(self, c: char) -> bool {
        match self {
            Lang::En => c.is_ascii_lowercase(),
            Lang::Ru => ('а'..='я').contains(&c),
        }
    }

    /// Знаки, допустимые внутри слова (не первым и не последним, не два подряд).
    pub fn inner(self) -> &'static str {
        match self {
            Lang::En => "'-",
            Lang::Ru => "-",
        }
    }

    /// Алфавит LM без служебных символов: 0 — граница слова, 1 — любой другой символ.
    pub fn alphabet(self) -> Vec<char> {
        let letters = match self {
            Lang::En => "abcdefghijklmnopqrstuvwxyz",
            Lang::Ru => "абвгдежзийклмнопрстуфхцчшщъыьэюя",
        };
        letters.chars().chain(self.inner().chars()).collect()
    }
}

/// Директивы `.aff`, чей аргумент — флаг со специальным смыслом, а не класс аффиксов.
const FLAG_DIRECTIVES: &[&str] = &[
    "NOSUGGEST",
    "KEEPCASE",
    "ONLYINCOMPOUND",
    "NEEDAFFIX",
    "FORBIDDENWORD",
    "COMPOUNDFLAG",
    "COMPOUNDBEGIN",
    "COMPOUNDMIDDLE",
    "COMPOUNDEND",
    "COMPOUNDPERMITFLAG",
    "COMPOUNDFORBIDFLAG",
    "CIRCUMFIX",
    "SUBSTANDARD",
    "WARN",
    "FORCEUCASE",
];

/// Элемент условия аффикса: `.` — любой символ, `[абв]` / `[^абв]` / буква — набор.
enum Cond {
    Any,
    Set { negated: bool, chars: Vec<char> },
}

impl Cond {
    fn accepts(&self, c: char) -> bool {
        match self {
            Cond::Any => true,
            Cond::Set { negated, chars } => chars.contains(&c) != *negated,
        }
    }
}

struct Affix {
    strip: String,
    add: String,
    cond: Vec<Cond>,
}

impl Affix {
    /// hunspell без FULLSTRIP не даёт аффиксу отсечь слово целиком.
    fn strips_all(&self, word: &str, full_strip: bool) -> bool {
        !full_strip && !self.strip.is_empty() && self.strip.len() == word.len()
    }

    /// Условие сверяется с концом исходного слова (до отсечения `strip`), как в hunspell.
    fn apply_suffix(&self, word: &str, full_strip: bool) -> Option<String> {
        let n = word.chars().count();
        if !word.ends_with(&self.strip) || n < self.cond.len() || self.strips_all(word, full_strip)
        {
            return None;
        }
        let tail = word.chars().skip(n - self.cond.len());
        if !tail.zip(&self.cond).all(|(c, k)| k.accepts(c)) {
            return None;
        }
        let form = format!("{}{}", &word[..word.len() - self.strip.len()], self.add);
        (!form.is_empty()).then_some(form)
    }

    fn apply_prefix(&self, word: &str, full_strip: bool) -> Option<String> {
        if !word.starts_with(&self.strip)
            || word.chars().count() < self.cond.len()
            || self.strips_all(word, full_strip)
        {
            return None;
        }
        if !word.chars().zip(&self.cond).all(|(c, k)| k.accepts(c)) {
            return None;
        }
        let form = format!("{}{}", self.add, &word[self.strip.len()..]);
        (!form.is_empty()).then_some(form)
    }
}

struct AffixClass {
    prefix: bool,
    cross: bool,
    rules: Vec<Affix>,
}

#[derive(Default)]
pub struct Aff {
    classes: HashMap<char, AffixClass>,
    special: HashSet<char>,
    only_in_compound: Option<char>,
    need_affix: Option<char>,
    forbidden: Option<char>,
    full_strip: bool,
}

fn single_flag(s: &str) -> Result<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Ok(c),
        _ => bail!("флаг {s:?}: поддерживаются только однобуквенные флаги"),
    }
}

fn parse_cond(s: &str) -> Result<Vec<Cond>> {
    let mut out = Vec::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '.' => out.push(Cond::Any),
            '[' => {
                let (mut negated, mut chars, mut closed) = (false, Vec::new(), false);
                for d in it.by_ref() {
                    match d {
                        ']' => {
                            closed = true;
                            break;
                        }
                        '^' if chars.is_empty() && !negated => negated = true,
                        d => chars.push(d),
                    }
                }
                ensure!(closed, "незакрытая [ в условии {s:?}");
                out.push(Cond::Set { negated, chars });
            }
            c => out.push(Cond::Set {
                negated: false,
                chars: vec![c],
            }),
        }
    }
    Ok(out)
}

pub fn parse_aff(text: &str) -> Result<Aff> {
    let mut aff = Aff::default();
    let mut lines = text.lines();
    let mut continuation_rules = 0usize;
    while let Some(line) = lines.next() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["SET", enc, ..] => ensure!(*enc == "UTF-8", "кодировка {enc} не поддерживается"),
            ["FLAG", kind, ..] => bail!("FLAG {kind} не поддерживается"),
            ["AF", ..] => bail!("AF не поддерживается"),
            ["FULLSTRIP", ..] => aff.full_strip = true,
            ["COMPOUNDRULE", rule, ..] => {
                if rule.parse::<usize>().is_err() {
                    aff.special
                        .extend(rule.chars().filter(|c| !"*?()".contains(*c)));
                }
            }
            [directive, f, ..] if FLAG_DIRECTIVES.contains(directive) => {
                let flag = single_flag(f)?;
                aff.special.insert(flag);
                match *directive {
                    "ONLYINCOMPOUND" => aff.only_in_compound = Some(flag),
                    "NEEDAFFIX" => aff.need_affix = Some(flag),
                    "FORBIDDENWORD" => aff.forbidden = Some(flag),
                    _ => {}
                }
            }
            [kind @ ("PFX" | "SFX"), flag, cross @ ("Y" | "N"), count]
                if count.parse::<usize>().is_ok() =>
            {
                let count: usize = count.parse()?;
                let mut rules = Vec::with_capacity(count);
                for _ in 0..count {
                    let rule = lines.next().context("правила аффикса оборвались")?;
                    let g: Vec<&str> = rule.split_whitespace().collect();
                    ensure!(
                        g.len() >= 4 && g[0] == *kind && g[1] == *flag,
                        "плохое правило {rule:?}"
                    );
                    let (add, cont) = g[3].split_once('/').unwrap_or((g[3], ""));
                    if !cont.is_empty() {
                        continuation_rules += 1;
                    }
                    let zero = |s: &str| {
                        if s == "0" {
                            String::new()
                        } else {
                            s.to_owned()
                        }
                    };
                    rules.push(Affix {
                        strip: zero(g[2]),
                        add: zero(add),
                        cond: parse_cond(g.get(4).copied().unwrap_or("."))?,
                    });
                }
                let class = AffixClass {
                    prefix: *kind == "PFX",
                    cross: *cross == "Y",
                    rules,
                };
                ensure!(
                    aff.classes.insert(single_flag(flag)?, class).is_none(),
                    "класс аффиксов {flag} объявлен дважды"
                );
            }
            [kind @ ("PFX" | "SFX"), ..] => {
                bail!("{kind}: заголовок или правило вне блока: {line:?}");
            }
            _ => {}
        }
    }
    ensure!(
        continuation_rules == 0,
        "аффиксы с флагами продолжения не поддерживаются ({continuation_rules})"
    );
    Ok(aff)
}

/// Флаг статьи `.dic` должен быть классом аффиксов или объявленным спецфлагом.
fn check_flags(aff: &Aff, word: &str, flags: &[char]) -> Result<()> {
    match flags
        .iter()
        .find(|f| !aff.classes.contains_key(f) && !aff.special.contains(f))
    {
        Some(f) => bail!("{word}: флаг {f:?} не объявлен в .aff"),
        None => Ok(()),
    }
}

/// Все формы слова: само слово, суффиксы, префиксы и их сочетания (cross product).
fn expand(aff: &Aff, word: &str, flags: &[char], out: &mut dyn FnMut(String)) {
    let has = |flag: Option<char>| flag.is_some_and(|f| flags.contains(&f));
    if has(aff.forbidden) || has(aff.only_in_compound) {
        return;
    }
    if !has(aff.need_affix) {
        out(word.to_owned());
    }
    let full = aff.full_strip;
    let mut crossable = Vec::new();
    for class in flags.iter().filter_map(|f| aff.classes.get(f)) {
        if class.prefix {
            continue;
        }
        for form in class
            .rules
            .iter()
            .filter_map(|r| r.apply_suffix(word, full))
        {
            if class.cross {
                crossable.push(form.clone());
            }
            out(form);
        }
    }
    for class in flags.iter().filter_map(|f| aff.classes.get(f)) {
        if !class.prefix {
            continue;
        }
        for rule in &class.rules {
            if let Some(form) = rule.apply_prefix(word, full) {
                out(form);
            }
            if class.cross {
                for form in crossable.iter().filter_map(|s| rule.apply_prefix(s, full)) {
                    out(form);
                }
            }
        }
    }
}

/// Нижний регистр, `ё→е`; только буквы языка и внутренние `-`/`'`. Иначе `None`.
pub fn normalize_form(lang: Lang, form: &str) -> Option<String> {
    let lower: String = form
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'ё' { 'е' } else { c })
        .collect();
    let first = lower.chars().next()?;
    let last = lower.chars().next_back()?;
    if !lang.is_letter(first) || !lang.is_letter(last) {
        return None;
    }
    let mut prev_inner = false;
    for c in lower.chars() {
        if lang.is_letter(c) {
            prev_inner = false;
        } else if lang.inner().contains(c) && !prev_inner {
            prev_inner = true;
        } else {
            return None;
        }
    }
    Some(lower)
}

pub struct Expansion {
    pub entries: usize,
    pub raw: usize,
    /// Сырая развёртка: все формы после нормализации.
    pub forms: BTreeSet<String>,
}

pub fn unmunch(aff: &Aff, dic: &str, lang: Lang) -> Result<Expansion> {
    let mut lines = dic.lines();
    let declared: usize = lines.next().context("пустой .dic")?.trim().parse()?;
    let mut exp = Expansion {
        entries: 0,
        raw: 0,
        forms: BTreeSet::new(),
    };
    for line in lines {
        let Some(entry) = line.split_whitespace().next() else {
            continue;
        };
        if entry.contains("\\/") {
            continue; // слово со знаком «/» всё равно отбросила бы нормализация
        }
        exp.entries += 1;
        let (word, flags) = entry.split_once('/').unwrap_or((entry, ""));
        let flags: Vec<char> = flags.chars().collect();
        check_flags(aff, word, &flags)?;
        let (raw, forms) = (&mut exp.raw, &mut exp.forms);
        expand(aff, word, &flags, &mut |form| {
            *raw += 1;
            if let Some(f) = normalize_form(lang, &form) {
                forms.insert(f);
            }
        });
    }
    ensure!(
        exp.entries.abs_diff(declared) * 100 <= declared,
        "в .dic {} статей, заявлено {declared}",
        exp.entries
    );
    Ok(exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFF: &str = "SET UTF-8\nNEEDAFFIX X\nNOSUGGEST !\n\
                       PFX P Y 1\nPFX P 0 re .\n\
                       SFX S Y 2\nSFX S y ies [^aeiou]y\nSFX S 0 s [^y]\n\
                       SFX D N 1\nSFX D 0 ed [^e]\n\
                       SFX W Y 1\nSFX W cat dog cat\n";

    fn forms(aff: &str, dic: &str) -> Result<Vec<String>> {
        let aff = parse_aff(aff)?;
        Ok(unmunch(&aff, dic, Lang::En)?.forms.into_iter().collect())
    }

    #[test]
    fn conditions_and_negated_sets_pick_rules() {
        let got = forms(AFF, "3\ncity/S\nday/S\ncat/S").unwrap();
        assert_eq!(got, ["cat", "cats", "cities", "city", "day"]);
    }

    #[test]
    fn cross_product_follows_the_y_flag() {
        assert_eq!(
            forms(AFF, "1\ncat/PS").unwrap(),
            ["cat", "cats", "recat", "recats"]
        );
        assert_eq!(
            forms(AFF, "1\nload/PD").unwrap(),
            ["load", "loaded", "reload"]
        );
    }

    #[test]
    fn needaffix_root_is_not_a_word() {
        assert_eq!(forms(AFF, "1\ncat/XS").unwrap(), ["cats"]);
    }

    #[test]
    fn strip_of_the_whole_word_needs_fullstrip() {
        assert_eq!(forms(AFF, "1\ncat/W").unwrap(), ["cat"]);
        let full = format!("FULLSTRIP\n{AFF}");
        assert_eq!(forms(&full, "1\ncat/W").unwrap(), ["cat", "dog"]);
    }

    #[test]
    fn every_dic_flag_must_be_declared() {
        assert!(forms(AFF, "1\ncat/Q").is_err());
        assert_eq!(forms(AFF, "1\ncat/!").unwrap(), ["cat"]);
    }

    #[test]
    fn affix_header_of_unexpected_shape_is_an_error() {
        assert!(parse_aff("SFX S Y\nSFX S 0 s .\n").is_err());
        assert!(parse_aff("SFX S maybe 1\nSFX S 0 s .\n").is_err());
    }
}
