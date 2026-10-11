//! Правило решения на границе слова (PRD «Решение на границе слова», `analysis/detection.md` §8):
//! жёсткие вето → кандидат в двух трактовках краевых знаков → словарное вето → короткие слова
//! по спискам и контексту → оценка LM с порогами → серая полоса (оставить, есть ⇧⇧).
//! Отдельно — случайные две заглавные («ПРивет» → «Привет»).
//!
//! На вход — нажатия слова, пара раскладок, контекст, настройки и словари ([`Input`]), на выход —
//! [`Decision`]. Между словами живёт только язык сегмента ([`Segment`]).

mod convert;
mod shape;

use std::ops::Range;

pub(crate) use convert::convert_keys;
use convert::stroke;
use shape::{is_leading_punct, is_trailing_punct};

use crate::{
    Config, Context, KeyStroke, Lang, LayoutId, LayoutMap, Level, Lexicon, PhysicalKey, ReplaceKind,
};

// Пороги подобраны на корпусе 2026-10-12 (настроечная половина `data/eval`), см. docs/eval.md.
// Δ = LM_цель(конверсия) − LM_набора(набранное), средний ln P на триграмму.
/// Конверсия — словарное слово цели: исправить, если Δ не ниже.
const THETA_DICT: f32 = -0.5;
/// Конверсия вне словаря: исправить, если Δ не ниже (и слово не короче `L_LM`).
const THETA_UNKNOWN: f32 = 2.5;
/// Конверсия вне словаря рассматривается от этой длины в буквах.
const L_LM: usize = 4;
/// Две буквы: пара из обоих списков (`vs` ↔ «мы») или словарное слово цели вне списков
/// (`lj` → «до», `шэь` → «I'm») — исправить, если Δ не ниже.
const THETA_SHORT: f32 = 0.5;
/// Сегмент уже на языке цели — пороги ниже на столько; на языке набора — выше.
const BETA_CONTEXT: f32 = 1.0;
/// Слова не длиннее решают списки `short_words` и контекст, а не словарь и LM.
const SHORT_MAX: usize = 2;
/// Случайные две заглавные исправляются в словах не короче (PRD, история 16).
const CASE_FIX_MIN: usize = 4;

/// Всё, что нужно для решения о слове на границе.
pub(crate) struct Input<'a> {
    pub(crate) keys: &'a [(PhysicalKey, Level)],
    pub(crate) from: &'a LayoutMap,
    pub(crate) to: &'a LayoutMap,
    pub(crate) ctx: &'a Context,
    pub(crate) config: &'a Config,
    pub(crate) lexicon: &'a Lexicon,
    /// Язык последних решённых слов сегмента.
    pub(crate) segment: Option<Lang>,
    /// Была правка, ручная конвертация или слово начато не с начала (неполное).
    pub(crate) edited: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Decision {
    /// Оставить; `lang` — язык, которым слово опознано (для контекста сегмента), если опознано.
    Keep {
        lang: Option<Lang>,
    },
    Fix(Fix),
}

#[derive(Debug, PartialEq)]
pub(crate) struct Fix {
    /// Замена без граничного символа.
    pub(crate) text: String,
    /// Нажатия замены в раскладке `target` (или в текущей).
    pub(crate) strokes: Vec<KeyStroke>,
    /// Куда переключить раскладку; `None` — остаться (исправлен только регистр).
    pub(crate) target: Option<LayoutId>,
    pub(crate) kind: ReplaceKind,
    /// Язык слова после замены.
    pub(crate) lang: Lang,
}

/// Контекст сегмента: язык последних решённых слов. Переживает границу слова и умирает вместе
/// с буфером при сбросе (клик, Tab, стрелки, сочетания, смена приложения).
#[derive(Debug, Default)]
pub(crate) struct Segment {
    lang: Option<Lang>,
}

impl Segment {
    pub(crate) fn lang(&self) -> Option<Lang> {
        self.lang
    }

    pub(crate) fn reset(&mut self) {
        self.lang = None;
    }

    pub(crate) fn set(&mut self, lang: Lang) {
        self.lang = Some(lang);
    }

    pub(crate) fn note(&mut self, decision: &Decision) {
        match decision {
            Decision::Keep { lang: Some(lang) } => self.lang = Some(*lang),
            Decision::Keep { lang: None } => {}
            Decision::Fix(fix) => self.lang = Some(fix.lang),
        }
    }
}

const KEEP: Decision = Decision::Keep { lang: None };

pub(crate) fn decide(input: &Input) -> Decision {
    let (from, to, lexicon) = (input.from, input.to, input.lexicon);
    // 1. Жёсткие вето, без оценки.
    if input.ctx.secure_input
        || input.ctx.password_field
        || input.edited
        || input
            .config
            .apps
            .excludes(&input.ctx.app, lexicon.default_excluded_apps())
    {
        return KEEP;
    }
    let Some(token) = Token::read(input.keys, from, to) else {
        return KEEP;
    };
    if input
        .config
        .words
        .contains(&token.typed.iter().collect::<String>())
    {
        return KEEP;
    }
    let Some(word) = shape::letter_span(&token.typed) else {
        return KEEP;
    };
    if shape::not_prose(&token.typed, &token.conv, &token.levels())
        || shape::password_like(&token.typed, &token.conv)
    {
        return KEEP;
    }
    let word_lower = lower(&token.typed[word.clone()]);
    let letters = letters(&token.typed[word.clone()]);
    // 3. Настоящее слово своего языка не трогается (here ↔ руку); «ПРивет» — только регистр.
    if letters > SHORT_MAX && lexicon.contains(from.lang, &word_lower) {
        return match case_fix(&token, word, from) {
            Some(fix) => Decision::Fix(fix),
            None => Decision::Keep {
                lang: Some(from.lang),
            },
        };
    }
    // Набранное в трактовке «знак» — короткое слово своего языка с обычной пунктуацией по
    // краям: `it.` — это «it» с точкой, а не «шею»; `to§` — не «to».
    let plain_edges = token.typed[..word.start]
        .iter()
        .all(|&c| is_leading_punct(c))
        && token.typed[word.end..]
            .iter()
            .all(|&c| is_trailing_punct(c));
    let known_short = plain_edges
        && letters <= SHORT_MAX
        && lexicon
            .short_words(from.lang, letters)
            .contains(&word_lower);
    let sign_word = plain_edges.then_some(word_lower.as_str());
    // 2, 4–6. Кандидаты: краевые знаки буквами и краевые знаки знаками.
    let mut best: Option<(u8, Reading)> = None;
    for reading in [token.letter_reading(), token.sign_reading(to)]
        .into_iter()
        .flatten()
    {
        if let Some(rank) = judge(&token, &reading, input, sign_word, known_short)
            && best.as_ref().is_none_or(|(r, _)| rank > *r)
        {
            best = Some((rank, reading));
        }
    }
    match best {
        Some((_, reading)) => Decision::Fix(reading.into_fix(&token, to)),
        None => Decision::Keep {
            lang: known_short.then_some(from.lang),
        },
    }
}

/// Ранг кандидата: 2 — словарь или список коротких слов, 1 — только LM; `None` — не исправлять.
/// `sign_word` — набранное без краевых знаков, если они обычная пунктуация на своём месте
/// (`levf.` → «levf»): с ним сравнивает LM; иначе — набранное целиком (`,sk` → «был»).
/// `known_short` — оно же короткое слово своего языка.
fn judge(
    token: &Token,
    reading: &Reading,
    input: &Input,
    sign_word: Option<&str>,
    known_short: bool,
) -> Option<u8> {
    let (from, to, lexicon) = (input.from, input.to, input.lexicon);
    let core = reading.core.clone();
    let candidate = lower(&reading.text[core.clone()]);
    let typed = lower(&token.typed[core.clone()]);
    let word = sign_word.unwrap_or(&typed);
    let in_target = input.segment == Some(to.lang);
    let in_source = input.segment == Some(from.lang);
    let short = |lang: Lang, len: usize, w: &str| lexicon.short_words(lang, len).contains(w);
    let delta = |typed: &str| lm(lexicon, to.lang, &candidate) - lm(lexicon, from.lang, typed);
    match letters(&reading.text[core]) {
        0 => None,
        // 4. Одна буква — только в сегменте на языке цели и по списку.
        1 => (in_target && short(to.lang, 1, &candidate) && !short(from.lang, 1, &typed))
            .then_some(2),
        // Две буквы — по спискам; пара из обоих списков — по контексту или по LM (vs → «мы»,
        // но не «мы» → vs); вне списков — словарь цели и LM.
        2 if in_source => None,
        2 => {
            let (in_b, in_a) = (short(to.lang, 2, &candidate), short(from.lang, 2, &typed));
            let ok = if in_b && (in_target || !in_a) {
                true
            } else if in_b {
                delta(word) >= THETA_SHORT
            } else {
                // в en_US пары букв — слова: двухбуквенный словарный кандидат в EN — только
                // сокращение с апострофом (`I'm`), иначе «ИИ» под CapsLock стало бы «BB»
                let dict_ok = to.lang == Lang::Ru || candidate.chars().count() > 2;
                dict_ok
                    && !in_a
                    && lexicon.contains(to.lang, &candidate)
                    && !lexicon.contains(from.lang, word)
                    && delta(word) >= THETA_SHORT
            };
            ok.then_some(2)
        }
        // 5. Оценка LM; контекст сегмента сдвигает пороги.
        _ if known_short => None,
        n => {
            let shift = if in_target {
                -BETA_CONTEXT
            } else if in_source {
                BETA_CONTEXT
            } else {
                0.0
            };
            let delta = delta(word);
            if lexicon.contains(to.lang, &candidate) {
                (delta >= THETA_DICT + shift).then_some(2)
            } else {
                // вне словаря — только чисто буквенное набранное: `example.com` не слово
                (n >= L_LM
                    && word.chars().all(char::is_alphabetic)
                    && delta >= THETA_UNKNOWN + shift)
                    .then_some(1)
            }
        }
    }
}

/// «ПРивет» → «Привет» в той же раскладке.
fn case_fix(token: &Token, word: Range<usize>, from: &LayoutMap) -> Option<Fix> {
    let levels = token.levels();
    if !shape::two_capitals(
        &token.typed[word.clone()],
        &levels[word.clone()],
        CASE_FIX_MIN,
    ) {
        return None;
    }
    let mut text = token.typed.clone();
    let mut strokes: Vec<KeyStroke> = token.keys.iter().map(|&(k, l)| stroke(k, l)).collect();
    lower_second(&mut text, &mut strokes, token.keys, word.start, from)?;
    Some(Fix {
        text: text.into_iter().collect(),
        strokes,
        target: None,
        kind: ReplaceKind::CaseFix,
        lang: from.lang,
    })
}

/// Вторая заглавная слова, начатого в `start`, — строчной: та же клавиша без Shift.
fn lower_second(
    text: &mut [char],
    strokes: &mut [KeyStroke],
    keys: &[(PhysicalKey, Level)],
    start: usize,
    map: &LayoutMap,
) -> Option<()> {
    let i = start + 1;
    text[i] = map.char_for(keys[i].0, Level::Base)?;
    strokes[i].shift = false;
    Some(())
}

/// Слово в двух раскладках, по символу на нажатие.
struct Token<'a> {
    keys: &'a [(PhysicalKey, Level)],
    typed: Vec<char>,
    conv: Vec<char>,
}

/// Одна трактовка конверсии: текст замены, нажатия и позиции ядра слова в нём.
struct Reading {
    text: Vec<char>,
    strokes: Vec<KeyStroke>,
    core: Range<usize>,
}

impl<'a> Token<'a> {
    fn read(keys: &'a [(PhysicalKey, Level)], from: &LayoutMap, to: &LayoutMap) -> Option<Self> {
        let mut token = Token {
            keys,
            typed: Vec::with_capacity(keys.len()),
            conv: Vec::with_capacity(keys.len()),
        };
        for &(key, level) in keys {
            token.typed.push(from.char_for(key, level)?);
            token.conv.push(to.char_for(key, level)?);
        }
        Some(token)
    }

    fn levels(&self) -> Vec<Level> {
        self.keys.iter().map(|&(_, l)| l).collect()
    }

    /// Все нажатия — в другой раскладке: `levf.` → «думаю», `ghbdtn?` → «привет,».
    fn letter_reading(&self) -> Option<Reading> {
        let core = candidate_core(&self.conv, 0..self.conv.len())?;
        Some(Reading {
            text: self.conv.clone(),
            strokes: self.keys.iter().map(|&(k, l)| stroke(k, l)).collect(),
            core,
        })
    }

    /// Краевые знаки, которые в другой раскладке — буквы, остаются знаками и печатаются своей
    /// клавишей цели: `ghbdtn,` → «привет,». `None`, если таких знаков нет или их не напечатать.
    fn sign_reading(&self, to: &LayoutMap) -> Option<Reading> {
        let n = self.typed.len();
        let sign = |i: usize| !self.typed[i].is_alphabetic() && self.conv[i].is_alphabetic();
        let lead = (0..n).take_while(|&i| sign(i)).count();
        let trail = (lead..n).rev().take_while(|&i| sign(i)).count();
        if lead + trail == 0 || lead + trail == n {
            return None;
        }
        let middle = lead..n - trail;
        let core = candidate_core(&self.conv, middle.clone())?;
        let mut text = self.conv.clone();
        let mut strokes = Vec::with_capacity(n);
        for (i, &(key, level)) in self.keys.iter().enumerate() {
            if middle.contains(&i) {
                strokes.push(stroke(key, level));
            } else {
                let c = self.typed[i];
                let fits = if i < lead {
                    is_leading_punct(c)
                } else {
                    is_trailing_punct(c)
                };
                if !fits {
                    return None;
                }
                text[i] = c;
                strokes.push(to.stroke_for(c)?);
            }
        }
        Some(Reading {
            text,
            strokes,
            core,
        })
    }
}

impl Reading {
    fn into_fix(self, token: &Token, to: &LayoutMap) -> Fix {
        let Reading {
            mut text,
            mut strokes,
            core,
        } = self;
        let levels = token.levels();
        if shape::two_capitals(&text[core.clone()], &levels[core.clone()], CASE_FIX_MIN) {
            // «GHbdtn» → «Привет», а не «ПРивет»
            let _ = lower_second(&mut text, &mut strokes, token.keys, core.start, to);
        }
        Fix {
            text: text.into_iter().collect(),
            strokes,
            target: Some(to.id.clone()),
            kind: ReplaceKind::Auto,
            lang: to.lang,
        }
    }
}

/// Ядро кандидата внутри `span`: буквы и внутренние `-`/`'`, по краям — только обычная
/// пунктуация. `None` — кандидат не похож на слово.
fn candidate_core(conv: &[char], span: Range<usize>) -> Option<Range<usize>> {
    let part = &conv[span.clone()];
    let first = part.iter().position(|c| c.is_alphabetic())?;
    let last = part.iter().rposition(|c| c.is_alphabetic())?;
    let edges_ok = part[..first].iter().all(|&c| is_leading_punct(c))
        && part[last + 1..].iter().all(|&c| is_trailing_punct(c));
    let core_ok = part[first..=last].iter().all(|&c| ks_data::is_word_char(c));
    (edges_ok && core_ok).then_some(span.start + first..span.start + last + 1)
}

fn lower(chars: &[char]) -> String {
    chars.iter().flat_map(|c| c.to_lowercase()).collect()
}

fn letters(chars: &[char]) -> usize {
    chars.iter().filter(|c| c.is_alphabetic()).count()
}

/// LM обучена на формах с `ё → е`, как и словарь.
fn lm(lexicon: &Lexicon, lang: Lang, word: &str) -> f32 {
    if word.contains('ё') {
        lexicon.lm_score(lang, &word.replace('ё', "е"))
    } else {
        lexicon.lm_score(lang, word)
    }
}
