//! Замер качества распознавания: живой текст набирается через `Engine` и считаются ложные
//! исправления; синтетические ошибки раскладки строятся по таблицам клавиш и считается полнота.
//!
//! Раскладки — фикстуры задачи 01 (`crates/ks-core/tests/fixtures/`): US, macOS «Русская»,
//! «Русская – ПК» и Windows KBDRU. Корпус — `data/eval/` (Tatoeba, отложенный), поделённый по
//! хэшу id предложения на настроечную и контрольную части ([`Part`]).
//!
//! Для подбора порогов: `load_corpus(dir, Part::Tune, ..)` и [`evaluate`] с вариантом `Config`;
//! итоговые цифры — на `Part::Control`.

pub mod metrics;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use ks_core::{
    Action, AppId, Config, Context, Engine, KeyDir, KeyEvent, Lang, LayoutId, LayoutMap, LayoutSet,
    Level, Lexicon, Modifiers, Os, PhysicalKey, ResetReason,
};
use metrics::{RecallOutcome, Tally};

// ---------------------------------------------------------------------------------------------
// Раскладки

const US: &str = include_str!("../../ks-core/tests/fixtures/us.tsv");
const RU_APPLE: &str = include_str!("../../ks-core/tests/fixtures/ru_apple.tsv");
const RU_PC: &str = include_str!("../../ks-core/tests/fixtures/ru_pc.tsv");
const RU_WIN: &str = include_str!("../../ks-core/tests/fixtures/ru_win.tsv");

pub struct Layouts {
    pub us: LayoutMap,
    pub ru_apple: LayoutMap,
    pub ru_pc: LayoutMap,
    pub ru_win: LayoutMap,
}

impl Layouts {
    pub fn fixtures() -> Layouts {
        let parse = |text| parse_layout(text).expect("фикстура раскладки задачи 01");
        Layouts {
            us: parse(US),
            ru_apple: parse(RU_APPLE),
            ru_pc: parse(RU_PC),
            ru_win: parse(RU_WIN),
        }
    }

    /// Три русские раскладки с короткими именами для срезов.
    pub fn russian(&self) -> [(&'static str, &LayoutMap); 3] {
        [
            ("Apple", &self.ru_apple),
            ("ПК", &self.ru_pc),
            ("KBDRU", &self.ru_win),
        ]
    }
}

/// Формат фикстуры задачи 01: `#` — комментарий; `@id`, `@lang`, `@name` — заголовок;
/// остальное — `код W3C\tBase\tShift\tCaps\tShift+Caps`, пусто — символа нет.
pub fn parse_layout(text: &str) -> Result<LayoutMap, String> {
    let (mut id, mut lang, mut name) = (None, None, None);
    let mut rows = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut cols = line.split('\t');
        match cols.next().unwrap_or_default() {
            "@id" => id = cols.next().map(str::to_owned),
            "@lang" => {
                lang = Some(match cols.next() {
                    Some("en") => Lang::En,
                    Some("ru") => Lang::Ru,
                    _ => Lang::Other,
                })
            }
            "@name" => name = cols.next().map(str::to_owned),
            code => {
                let key: PhysicalKey = code.parse().map_err(|e| format!("{code}: {e:?}"))?;
                rows.push((key, cols.map(|c| c.chars().next()).collect::<Vec<_>>()));
            }
        }
    }
    let mut map = LayoutMap::new(
        LayoutId(id.ok_or("нет @id")?),
        lang.ok_or("нет @lang")?,
        name.ok_or("нет @name")?,
    );
    let levels = [Level::Base, Level::Shift, Level::Caps, Level::ShiftCaps];
    for (key, chars) in rows {
        for (level, ch) in levels.into_iter().zip(chars) {
            if let Some(ch) = ch {
                map.insert(key, level, ch);
            }
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------------------------
// Набор

/// Нажатие клавиши; `Other(0)` — символ, которого в раскладке нет (буфер слова сбрасывается).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub key: PhysicalKey,
    pub shift: bool,
    pub caps: bool,
}

/// Слово к набору: нажатия, раскладка, активная при наборе, и что при этом видно на экране.
#[derive(Clone, Debug)]
pub struct Word {
    pub presses: Vec<Press>,
    pub layout: LayoutId,
    pub screen: String,
}

/// Правильный набор текста в раскладке `map`.
pub fn type_correct(text: &str, map: &LayoutMap) -> Word {
    let presses = text
        .chars()
        .map(|c| match map.stroke_for(c) {
            Some(s) => Press {
                key: s.key,
                shift: s.shift,
                caps: false,
            },
            None => Press {
                key: PhysicalKey::Other(0),
                shift: false,
                caps: false,
            },
        })
        .collect();
    Word {
        presses,
        layout: map.id.clone(),
        screen: text.to_owned(),
    }
}

/// Синтетическая ошибка раскладки: нажатия, которыми `intended` набирается в `intended_map`,
/// сделаны при активной `typed_map`. `caps_lock` — слово набрано с включённым CapsLock.
/// `None`, если символа нет в одной из раскладок.
pub fn mistype(
    intended: &str,
    intended_map: &LayoutMap,
    typed_map: &LayoutMap,
    caps_lock: bool,
) -> Option<Word> {
    let mut presses = Vec::new();
    let mut screen = String::new();
    for c in intended.chars() {
        let (key, shift, caps) = press_for(c, intended_map, caps_lock)?;
        let level = match (shift, caps) {
            (false, false) => Level::Base,
            (true, false) => Level::Shift,
            (false, true) => Level::Caps,
            (true, true) => Level::ShiftCaps,
        };
        screen.push(typed_map.char_for(key, level)?);
        presses.push(Press { key, shift, caps });
    }
    Some(Word {
        presses,
        layout: typed_map.id.clone(),
        screen,
    })
}

/// Клавиша, Shift и CapsLock для символа задуманного текста.
fn press_for(c: char, map: &LayoutMap, caps_lock: bool) -> Option<(PhysicalKey, bool, bool)> {
    // CapsLock держится на всё слово: заглавная буква — без Shift, строчная — с Shift,
    // знаки — своими нажатиями, но тоже под CapsLock.
    if caps_lock && c.is_alphabetic() {
        let lower = c.to_lowercase().next()?;
        return Some((map.stroke_for(lower)?.key, !c.is_uppercase(), true));
    }
    let s = map.stroke_for(c)?;
    Some((s.key, s.shift, caps_lock))
}

/// Варианты синтетической ошибки; слово получает вариант по кругу от своего номера.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Lower,
    Capital,
    CapsLock,
    Comma,
    Period,
}

impl Variant {
    pub const ALL: [Variant; 5] = [
        Variant::Lower,
        Variant::Capital,
        Variant::CapsLock,
        Variant::Comma,
        Variant::Period,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Variant::Lower => "обычный",
            Variant::Capital => "Первая заглавная",
            Variant::CapsLock => "CAPSLOCK",
            Variant::Comma => "«,» в конце",
            Variant::Period => "«.» в конце",
        }
    }

    /// Задуманный текст варианта для слова в нижнем регистре.
    pub fn apply(self, word: &str) -> String {
        match self {
            Variant::Lower => word.to_owned(),
            Variant::Capital => {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
            Variant::CapsLock => word.to_uppercase(),
            Variant::Comma => format!("{word},"),
            Variant::Period => format!("{word}."),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Испытуемый

/// То, что замеряется: движок или подставной двойник в тестах.
pub trait Subject {
    /// Новое поле: всё набранное раньше забыто.
    fn reset(&mut self);
    /// Набирает слово и пробел за ним; текст исправления без граничного символа, если оно было.
    fn type_word(&mut self, word: &Word) -> Option<String>;
}

/// `Engine` в связке «US + русская раскладка», набор — в приложении TextEdit.
pub struct EngineSubject {
    engine: Engine,
    app: AppId,
    time_ms: u64,
}

fn replace_text(actions: Vec<Action>) -> Option<String> {
    actions
        .into_iter()
        .filter_map(|a| match a {
            Action::Replace { text, .. } => Some(text),
            _ => None,
        })
        .next_back()
}

impl EngineSubject {
    /// Настройки по умолчанию для macOS.
    pub fn new(lexicon: Arc<Lexicon>, layouts: Vec<LayoutMap>) -> EngineSubject {
        EngineSubject::with_config(Config::default_for(Os::MacOs), lexicon, layouts)
    }

    /// Заданные настройки — например, вариант порогов при подборе.
    pub fn with_config(
        config: Config,
        lexicon: Arc<Lexicon>,
        layouts: Vec<LayoutMap>,
    ) -> EngineSubject {
        EngineSubject::from_engine(Engine::new(config, lexicon, LayoutSet(layouts)))
    }

    /// Готовый движок.
    pub fn from_engine(engine: Engine) -> EngineSubject {
        EngineSubject {
            engine,
            app: AppId("com.apple.TextEdit".to_owned()),
            time_ms: 0,
        }
    }

    fn send(
        &mut self,
        ctx: &Context,
        key: PhysicalKey,
        dir: KeyDir,
        mods: Modifiers,
    ) -> Option<String> {
        self.time_ms += 40;
        let ev = KeyEvent {
            key,
            dir,
            mods,
            repeat: false,
            time_ms: self.time_ms,
        };
        replace_text(self.engine.on_key(&ev, ctx).actions)
    }
}

impl Subject for EngineSubject {
    fn reset(&mut self) {
        self.engine.on_reset(ResetReason::External);
    }

    /// Заглавные — с настоящим ShiftLeft вокруг клавиши; CapsLock — флагом модификатора.
    /// После пробела срабатывают отложенные таймеры движка, как при паузе в наборе.
    fn type_word(&mut self, word: &Word) -> Option<String> {
        let ctx = Context {
            app: self.app.clone(),
            layout: word.layout.clone(),
            secure_input: false,
            password_field: false,
        };
        let mut replaced = None;
        let mut caps = false;
        for p in &word.presses {
            caps = p.caps;
            let plain = Modifiers {
                caps_lock: p.caps,
                ..Modifiers::default()
            };
            let mods = Modifiers {
                shift: p.shift,
                ..plain
            };
            let mut outs = Vec::with_capacity(4);
            if p.shift {
                outs.push(self.send(&ctx, PhysicalKey::ShiftLeft, KeyDir::Down, mods));
            }
            outs.push(self.send(&ctx, p.key, KeyDir::Down, mods));
            outs.push(self.send(&ctx, p.key, KeyDir::Up, mods));
            if p.shift {
                outs.push(self.send(&ctx, PhysicalKey::ShiftLeft, KeyDir::Up, plain));
            }
            if let Some(text) = outs.into_iter().flatten().next_back() {
                replaced = Some(text);
            }
        }
        let plain = Modifiers {
            caps_lock: caps,
            ..Modifiers::default()
        };
        for dir in [KeyDir::Down, KeyDir::Up] {
            if let Some(text) = self.send(&ctx, PhysicalKey::Space, dir, plain) {
                replaced = Some(text);
            }
        }
        if let Some(deadline) = self.engine.next_deadline() {
            self.time_ms = self.time_ms.max(deadline);
            if let Some(text) = replace_text(self.engine.tick(deadline)) {
                replaced = Some(text);
            }
        }
        replaced.map(|t| t.strip_suffix([' ', '\n']).map(str::to_owned).unwrap_or(t))
    }
}

/// Связка с движком до любых цифр (пара US + «Русская – ПК»): «ghbdtn» → «привет»,
/// «Ghbdtn» → «Привет», правильно набранное «привет» не тронуто.
pub fn smoke_check(
    lexicon: &Arc<Lexicon>,
    layouts: &Layouts,
    config: &Config,
) -> Result<(), String> {
    let mut subject = EngineSubject::with_config(
        config.clone(),
        Arc::clone(lexicon),
        vec![layouts.us.clone(), layouts.ru_pc.clone()],
    );
    for intended in ["привет", "Привет"] {
        let typo = mistype(intended, &layouts.ru_pc, &layouts.us, false)
            .ok_or("в таблицах раскладок нет «привет»")?;
        subject.reset();
        let got = subject.type_word(&typo);
        if got.as_deref() != Some(intended) {
            return Err(format!(
                "«{}» → {got:?}, ожидалось «{intended}»",
                typo.screen
            ));
        }
    }
    subject.reset();
    match subject.type_word(&type_correct("привет", &layouts.ru_pc)) {
        Some(got) => Err(format!("правильно набранное «привет» исправлено в «{got}»")),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------
// Корпус

/// Часть корпуса оценки: настроечная — для подбора порогов, контрольная — для итоговых цифр.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Tune,
    Control,
    All,
}

const SPLIT_SALT: u64 = 0x6b73_7370_6c69_7401;

pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Part {
    pub fn parse(name: &str) -> Option<Part> {
        match name {
            "tune" => Some(Part::Tune),
            "control" => Some(Part::Control),
            "all" => Some(Part::All),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::Tune => "настроечная",
            Part::Control => "контрольная",
            Part::All => "весь корпус",
        }
    }

    /// Половины корпуса по хэшу id предложения: детерминированно, не зависит ни от порядка
    /// строк, ни от размера выборки.
    pub fn contains(self, id: u64) -> bool {
        let tune = splitmix64(id ^ SPLIT_SALT).is_multiple_of(2);
        match self {
            Part::All => true,
            Part::Tune => tune,
            Part::Control => !tune,
        }
    }
}

/// Токен состязательного среза из `adversarial.tsv`.
#[derive(Clone, Debug)]
pub struct AdversarialToken {
    pub slice: String,
    pub lang: Lang,
    pub token: String,
}

/// Часть корпуса оценки: предложения RU и EN и ручные состязательные токены.
#[derive(Clone, Debug)]
pub struct Corpus {
    pub part: Part,
    pub ru: Vec<String>,
    pub en: Vec<String>,
    pub adversarial: Vec<AdversarialToken>,
}

/// Читает `tatoeba_ru.tsv`, `tatoeba_en.tsv` (`id\tтекст`) и `adversarial.tsv` из `dir`;
/// берёт предложения части `part`, не больше `limit` на язык.
pub fn load_corpus(dir: &Path, part: Part, limit: usize) -> Result<Corpus> {
    let read = |file: &str| {
        let path = dir.join(file);
        std::fs::read_to_string(&path).with_context(|| path.display().to_string())
    };
    let sentences = |file: &str| -> Result<Vec<String>> {
        let mut out = Vec::new();
        for line in read(file)?
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (id, text) = line
                .split_once('\t')
                .with_context(|| format!("{file}: строка без id"))?;
            let id: u64 = id.parse().with_context(|| format!("{file}: id {id:?}"))?;
            if part.contains(id) && out.len() < limit {
                out.push(text.to_owned());
            }
        }
        Ok(out)
    };
    let mut adversarial = Vec::new();
    for line in read("adversarial.tsv")?
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut cols = line.split('\t');
        let (Some(slice), Some(lang), Some(token)) = (cols.next(), cols.next(), cols.next()) else {
            anyhow::bail!("adversarial.tsv: ожидалось 3 колонки: {line:?}");
        };
        adversarial.push(AdversarialToken {
            slice: slice.to_owned(),
            lang: if lang == "ru" { Lang::Ru } else { Lang::En },
            token: token.to_owned(),
        });
    }
    Ok(Corpus {
        part,
        ru: sentences("tatoeba_ru.tsv")?,
        en: sentences("tatoeba_en.tsv")?,
        adversarial,
    })
}

/// Типографские знаки, которых нет на клавиатуре, — в то, что пользователь набрал бы руками.
pub fn normalize_typography(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '«' | '»' | '„' | '“' | '”' => '"',
            '‘' | '’' => '\'',
            '—' | '–' => '-',
            '…' => '.',
            '\u{a0}' => ' ',
            c => c,
        })
        .collect()
}

pub fn letters(token: &str) -> usize {
    token.chars().filter(|c| c.is_alphabetic()).count()
}

fn typable(map: &LayoutMap, text: &str) -> bool {
    text.chars().all(|c| map.stroke_for(c).is_some())
}

/// Слово корпуса для замера полноты: нижний регистр, только буквы и внутренние `-`/`'`.
#[derive(Clone, Debug)]
pub struct RecallWord {
    pub text: String,
    pub in_dict: bool,
}

/// Слова для полноты из предложений: ядро токена без краевых знаков, только буквы языка.
pub fn recall_words(sentences: &[String], lang: Lang, lexicon: &Lexicon) -> Vec<RecallWord> {
    let is_letter = |c: char| match lang {
        Lang::Ru => matches!(c, 'а'..='я' | 'ё'),
        _ => c.is_ascii_lowercase(),
    };
    sentences
        .iter()
        .flat_map(|s| {
            normalize_typography(s)
                .split_whitespace()
                .map(|t| t.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
                .collect::<Vec<_>>()
        })
        .filter(|w| {
            w.chars().next().is_some_and(is_letter)
                && w.chars().next_back().is_some_and(is_letter)
                && w.chars().all(|c| is_letter(c) || c == '-' || c == '\'')
        })
        .map(|w| RecallWord {
            in_dict: lexicon.contains(lang, &w),
            text: w,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Прогоны

/// Живой текст: предложение набирается подряд (токен — в раскладке, где он набирается целиком,
/// сначала своей), пробел после каждого токена, между предложениями — сброс. FP — любое
/// исправление токена; токены без букв набираются, но не считаются.
pub fn run_live(
    subject: &mut impl Subject,
    sentences: &[String],
    own: &LayoutMap,
    other: &LayoutMap,
    slice: &str,
    tally: &mut Tally,
) {
    for sentence in sentences {
        subject.reset();
        for token in normalize_typography(sentence).split_whitespace() {
            let map = if typable(own, token) || !typable(other, token) {
                own
            } else {
                other
            };
            let corrected = subject.type_word(&type_correct(token, map)).is_some();
            let n = letters(token);
            if n > 0 {
                tally.live(slice, n, corrected);
            }
        }
    }
}

/// Полнота: каждое слово набирается отдельно (сброс перед ним) в `typed` вместо `intended`,
/// в варианте по кругу ([`Variant::ALL`]). Засчитывается только точный задуманный текст;
/// слова, которые не набрать, считаются отдельно.
pub fn run_recall(
    subject: &mut impl Subject,
    words: &[RecallWord],
    intended: &LayoutMap,
    typed: &LayoutMap,
    slice: &str,
    tally: &mut Tally,
) {
    for (i, word) in words.iter().enumerate() {
        let variant = Variant::ALL[i % Variant::ALL.len()];
        let text = variant.apply(&word.text);
        let Some(typo) = mistype(&text, intended, typed, variant == Variant::CapsLock) else {
            tally.recall_skip(slice);
            continue;
        };
        subject.reset();
        let outcome = match subject.type_word(&typo) {
            Some(fixed) if fixed == text => RecallOutcome::Fixed,
            Some(_) => RecallOutcome::Wrong,
            None => RecallOutcome::Missed,
        };
        tally.recall(slice, word.in_dict, letters(&word.text), variant, outcome);
    }
}

/// Состязательный срез: каждый токен набран правильно и отдельно; любое исправление — ложное.
pub fn run_adversarial(
    subject: &mut impl Subject,
    tokens: &[String],
    own: &LayoutMap,
    slice: &str,
    tally: &mut Tally,
) {
    for token in tokens {
        subject.reset();
        let corrected = subject.type_word(&type_correct(token, own)).is_some();
        tally.adversarial(slice, corrected);
    }
}

/// Состязательный срез: имя (`срез · язык`), язык, в раскладке которого токены набраны, токены.
#[derive(Clone, Debug)]
pub struct AdversarialSlice {
    pub name: String,
    pub lang: Lang,
    pub tokens: Vec<String>,
}

/// Срезы из `adversarial.tsv` и производные от корпуса: опечатки, растянутые слова и словарные
/// слова, чья конверсия — тоже словарное слово другого языка (коллизии из живого текста).
pub fn adversarial_slices(
    corpus: &Corpus,
    ru_words: &[RecallWord],
    en_words: &[RecallWord],
    lexicon: &Lexicon,
    us: &LayoutMap,
    ru_map: &LayoutMap,
) -> Vec<AdversarialSlice> {
    let code = |lang: Lang| if lang == Lang::Ru { "ru" } else { "en" };
    let mut slices: Vec<AdversarialSlice> = Vec::new();
    for t in &corpus.adversarial {
        let name = format!("{} · {}", t.slice, code(t.lang));
        match slices.iter_mut().find(|s| s.name == name) {
            Some(s) => s.tokens.push(t.token.clone()),
            None => slices.push(AdversarialSlice {
                name,
                lang: t.lang,
                tokens: vec![t.token.clone()],
            }),
        }
    }
    for (lang, words, own, other, other_lang) in [
        (Lang::Ru, ru_words, ru_map, us, Lang::En),
        (Lang::En, en_words, us, ru_map, Lang::Ru),
    ] {
        let dict: std::collections::BTreeSet<&str> = words
            .iter()
            .filter(|w| w.in_dict)
            .map(|w| w.text.as_str())
            .collect();
        let typos: Vec<String> = dict
            .iter()
            .filter(|w| w.chars().count() >= 4)
            .enumerate()
            .filter_map(|(i, w)| typo(w, own, i))
            .filter(|t| !lexicon.contains(lang, t))
            .take(3000)
            .collect();
        let stretched: Vec<String> = dict
            .iter()
            .filter(|w| w.chars().count() >= 3)
            .filter_map(|w| stretch(w))
            .take(2000)
            .collect();
        let collisions: Vec<String> = dict
            .iter()
            .filter(|w| w.chars().count() >= 2)
            .filter(|w| convert(w, own, other).is_some_and(|c| lexicon.contains(other_lang, &c)))
            .map(|w| (*w).to_owned())
            .collect();
        for (name, tokens) in [
            ("опечатки из корпуса", typos),
            ("растянутые из корпуса", stretched),
            ("коллизии из корпуса", collisions),
        ] {
            slices.push(AdversarialSlice {
                name: format!("{name} · {}", code(lang)),
                lang,
                tokens,
            });
        }
    }
    slices
}

/// Полный замер одного варианта настроек на части корпуса: живой текст, полнота и
/// состязательные срезы для каждой из трёх русских раскладок; раскладка — часть ключа среза.
pub fn evaluate(
    lexicon: &Arc<Lexicon>,
    layouts: &Layouts,
    corpus: &Corpus,
    config: &Config,
) -> Tally {
    let ru_words = recall_words(&corpus.ru, Lang::Ru, lexicon);
    let en_words = recall_words(&corpus.en, Lang::En, lexicon);
    let us = &layouts.us;
    let mut tally = Tally::default();
    for (name, ru_map) in layouts.russian() {
        let mut subject = EngineSubject::with_config(
            config.clone(),
            Arc::clone(lexicon),
            vec![us.clone(), ru_map.clone()],
        );
        let s = &mut subject;
        let t = &mut tally;
        run_live(s, &corpus.ru, ru_map, us, &format!("RU · {name}"), t);
        run_live(s, &corpus.en, us, ru_map, &format!("EN · {name}"), t);
        run_recall(s, &ru_words, ru_map, us, &format!("RU в US · {name}"), t);
        run_recall(s, &en_words, us, ru_map, &format!("EN в RU · {name}"), t);
        for slice in adversarial_slices(corpus, &ru_words, &en_words, lexicon, us, ru_map) {
            let own = if slice.lang == Lang::Ru { ru_map } else { us };
            let key = format!("{} · {name}", slice.name);
            run_adversarial(s, &slice.tokens, own, &key, t);
        }
    }
    tally
}

// ---------------------------------------------------------------------------------------------
// Производные слова

/// Те же нажатия в другой раскладке (базовый уровень); `None`, если символа нет.
pub fn convert(word: &str, from: &LayoutMap, to: &LayoutMap) -> Option<String> {
    word.chars()
        .map(|c| {
            let s = from.stroke_for(c)?;
            to.char_for(s.key, if s.shift { Level::Shift } else { Level::Base })
        })
        .collect()
}

const ROWS: [&[PhysicalKey]; 3] = {
    use PhysicalKey::*;
    [
        &[
            KeyQ,
            KeyW,
            KeyE,
            KeyR,
            KeyT,
            KeyY,
            KeyU,
            KeyI,
            KeyO,
            KeyP,
            BracketLeft,
            BracketRight,
        ],
        &[
            KeyA, KeyS, KeyD, KeyF, KeyG, KeyH, KeyJ, KeyK, KeyL, Semicolon, Quote,
        ],
        &[KeyZ, KeyX, KeyC, KeyV, KeyB, KeyN, KeyM, Comma, Period],
    ]
};

/// Опечатка: замена буквы соседней по ряду клавишей (чётные `seed`) или перестановка соседних
/// букв (нечётные). `None`, если опечатка не получилась.
pub fn typo(word: &str, map: &LayoutMap, seed: usize) -> Option<String> {
    let mut chars: Vec<char> = word.chars().collect();
    if chars.len() < 2 {
        return None;
    }
    let pos = seed / 2 % (chars.len() - 1);
    if seed % 2 == 1 {
        chars.swap(pos, pos + 1);
    } else {
        let key = map.stroke_for(chars[pos])?.key;
        let row = ROWS.iter().find(|r| r.contains(&key))?;
        let col = row.iter().position(|&k| k == key)?;
        let neighbour = [col + 1, col.wrapping_sub(1)]
            .into_iter()
            .filter_map(|i| row.get(i))
            .filter_map(|&k| map.char_for(k, Level::Base))
            .find(|c| c.is_alphabetic())?;
        chars[pos] = neighbour;
    }
    let out: String = chars.into_iter().collect();
    (out != word).then_some(out)
}

/// Растянутое слово: первая гласная повторена ещё трижды («слоооово»).
pub fn stretch(word: &str) -> Option<String> {
    let pos = word.find(|c: char| "аеёиоуыэюяaeiouy".contains(c))?;
    let vowel = word[pos..].chars().next()?;
    let mut out = word.to_owned();
    for _ in 0..3 {
        out.insert(pos, vowel);
    }
    Some(out)
}
