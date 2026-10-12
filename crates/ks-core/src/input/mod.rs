//! Ручное управление: жесты Shift, горячие клавиши из `Config.hotkeys`, окно отмены,
//! ручная конвертация последнего слова, список слов «никогда».
//!
//! `Engine::on_key` спрашивает этот модуль первым о каждом нажатии и отпускании и передаёт
//! состояние движка только для чтения ([`View`]). Ответ — [`Step`]: проглотить событие как
//! хоткей или пропустить дальше, какие действия выполнить до него, что стало с буфером.
//! Хоткеи распознаются по физическим клавишам, раскладка на совпадение не влияет.
//!
//! Часов в ядре нет: время — только `KeyEvent.time_ms` и `Engine::tick(now_ms)`.

use std::collections::VecDeque;

use crate::buffer::TokenBuffer;
use crate::detect::convert_keys;
use crate::{
    Action, AppId, Config, Context, Disposition, Hotkey, KeyDir, KeyEvent, KeyStroke, Lang,
    LayoutId, LayoutMap, LayoutSet, Level, Modifiers, Os, PhysicalKey, ReplaceKind, ResetReason,
    WordRules,
};

/// Одиночный Shift: от нажатия до отпускания не дольше.
pub(crate) const T_TAP_MS: u64 = 300;
/// Двойной Shift: второе нажатие не позже этого после первого отпускания.
pub(crate) const T_DOUBLE_MS: u64 = 400;
/// Слова короче в список «никогда» не берутся: иначе ломаются короткие слова.
const NEVER_MIN_CHARS: usize = 3;
/// Память сеанса держит столько последних отменённых слов; старые вытесняются.
const UNDONE_MAX: usize = 64;

/// Состояние движка, которое модуль читает.
pub(crate) struct View<'a> {
    pub(crate) config: &'a Config,
    pub(crate) layouts: &'a LayoutSet,
    pub(crate) buffer: &'a TokenBuffer,
    /// Чьё стандартное «отменить» слушать.
    pub(crate) os: Os,
}

/// Ответ модуля на событие.
#[derive(Debug, Default)]
pub(crate) struct Step {
    /// `Some` — событие было хоткеем и дальше не идёт.
    pub(crate) consumed: Option<Disposition>,
    /// Действия, которые исполняются раньше самого события.
    pub(crate) actions: Vec<Action>,
    /// Слово в буфере сконвертировано вручную и теперь стоит в этой раскладке.
    pub(crate) relayout: Option<LayoutId>,
    /// Ожидавший одиночный Shift переключил раскладку: событие печатается уже в ней.
    pub(crate) switched_to: Option<LayoutId>,
}

/// Три переназначаемых действия (PRD, история 31).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    UndoOrConvertWord,
    ConvertSelection,
    ToggleAutocorrect,
}

/// Shift нажат и ещё не отпущен.
#[derive(Debug)]
struct Tap {
    key: PhysicalKey,
    down_ms: u64,
    /// Между нажатием и отпусканием не было других клавиш, модификаторов и кликов.
    clean: bool,
    /// Второе касание двойного Shift.
    second: bool,
}

/// Одиночный Shift отпущен и ждёт `T_DOUBLE_MS`: не первая ли это половина двойного.
#[derive(Debug)]
struct Pending {
    up_ms: u64,
    /// Другая раскладка пары относительно той, что была при касании.
    target: Option<LayoutId>,
}

/// Модификаторы, нажатые вместе: для хоткеев только из модификаторов (⌥⇧).
#[derive(Debug, Default)]
struct Chord {
    peak: u8,
    dirty: bool,
}

/// Слово, закрытое пробелом. Из него делаются отмена и ручная конвертация; живёт до первого
/// нажатия, отличного от Shift. После Enter не запоминается: строка могла уйти (чат),
/// и что стоит перед кареткой, уже неизвестно.
#[derive(Debug)]
struct Finished {
    keys: Vec<(PhysicalKey, Level)>,
    /// В какой раскладке слово стоит набранным (для отмены — куда вернуть).
    layout: LayoutId,
    app: AppId,
    boundary: (KeyStroke, char),
    /// Автоисправление напечатало столько символов вместе с границей — окно отмены открыто.
    corrected: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct InputState {
    /// Нажатые модификаторы, по биту на клавишу (`mod_bit`).
    held: u8,
    chord: Option<Chord>,
    tap: Option<Tap>,
    /// Сработали оба Shift; касаний нет, пока оба не отпущены.
    both: bool,
    pending: Option<Pending>,
    finished: Option<Finished>,
    last_undone: Option<String>,
    /// Набранные формы (без хвостовых знаков) последних `UNDONE_MAX` токенов, исправление
    /// которых отменено в этом сеансе: автоматика их больше не трогает. Только память
    /// процесса — не `Config`, не диск, не логи; сбросы и `set_config` её не чистят, новый
    /// `Engine` начинает с пустой.
    undone: VecDeque<String>,
    /// ⌘Z/Ctrl+Z нажат в окне отмены: отмена ждёт отпускания Cmd/Ctrl, чтобы стирание не
    /// стало Ctrl+Backspace / Cmd+Backspace. Пока ждёт, автоповтор Z глотается.
    queued_undo: Option<Finished>,
}

impl InputState {
    pub(crate) fn on_key(&mut self, ev: &KeyEvent, ctx: &Context, view: &View) -> Step {
        let mut step = Step::default();
        if self.deadline().is_some_and(|d| ev.time_ms > d) {
            self.flush(view.config, &mut step);
        }
        if self.finished.as_ref().is_some_and(|f| f.app != ctx.app) {
            self.finished = None;
        }
        if self.queued_undo.as_ref().is_some_and(|f| f.app != ctx.app) {
            self.queued_undo = None;
        }
        let is_shift = matches!(ev.key, PhysicalKey::ShiftLeft | PhysicalKey::ShiftRight);
        if !is_shift && ev.dir == KeyDir::Down && !ev.key.is_modifier() {
            self.reconcile_shift(ev.mods.shift);
        }
        match (ev.dir, is_shift) {
            (KeyDir::Down, true) => self.shift_down(ev, view, &mut step),
            (KeyDir::Up, true) => self.shift_up(ev, ctx, view, &mut step),
            (KeyDir::Down, false) => self.other_down(ev, view, &mut step),
            (KeyDir::Up, false) => self.other_up(ev, view, &mut step),
        }
        step
    }

    /// Когда бэкенду позвать `tick`: конец ожидания второго касания.
    pub(crate) fn deadline(&self) -> Option<u64> {
        let pending = self.pending.as_ref()?;
        match &self.tap {
            Some(tap) if tap.second => Some(tap.down_ms.saturating_add(T_TAP_MS)),
            _ => Some(pending.up_ms.saturating_add(T_DOUBLE_MS)),
        }
    }

    pub(crate) fn tick(&mut self, now_ms: u64, config: &Config) -> Vec<Action> {
        let mut step = Step::default();
        if self.deadline().is_some_and(|d| now_ms > d) {
            self.flush(config, &mut step);
        }
        step.actions
    }

    /// Клик, смена фокуса, безопасный ввод: слово забыто, Shift с кликом — не касание.
    /// Отпускания модификаторов могли уйти мимо движка (поле пароля, отключённый перехват),
    /// поэтому нажатые модификаторы забываются. Ожидающий одиночный Shift снимается вместе
    /// со сроком: Shift с кликом раскладку не переключает. Ждущая отмена тоже снимается:
    /// каретка могла уйти.
    pub(crate) fn on_reset(&mut self, _why: ResetReason) {
        self.finished = None;
        self.queued_undo = None;
        self.held = 0;
        self.chord = None;
        self.tap = None;
        self.both = false;
        self.pending = None;
    }

    /// Раскладки сменились: записанные нажатия больше не читаются.
    pub(crate) fn forget_word(&mut self) {
        self.finished = None;
    }

    pub(crate) fn last_undone(&self) -> Option<&str> {
        self.last_undone.as_deref()
    }

    /// Токен отменён в этом сеансе: без учёта регистра и хвостовых знаков с обеих сторон
    /// (отменённое «ИИ,» защищает и «ИИ»).
    pub(crate) fn was_undone(&self, keys: &[(PhysicalKey, Level)], from: &LayoutMap) -> bool {
        !self.undone.is_empty()
            && typed_form(keys, from).is_some_and(|typed| {
                let core = word_core(&typed);
                self.undone.iter().any(|w| same_word(w, core))
            })
    }

    /// Запомнить отменённый токен; повтор поднимается в самые свежие, сверх `UNDONE_MAX`
    /// вытесняется самый старый.
    fn remember_undone(&mut self, typed: &str) {
        let core = word_core(typed);
        self.undone.retain(|w| !same_word(w, core));
        self.undone.push_back(core.to_owned());
        if self.undone.len() > UNDONE_MAX {
            self.undone.pop_front();
        }
    }

    /// Граница слова пройдена: запомнить слово (и исправление, если было) для ⇧⇧ и Pause.
    pub(crate) fn word_done(
        &mut self,
        buffer: &TokenBuffer,
        ev: &KeyEvent,
        app: &AppId,
        actions: &[Action],
    ) {
        self.finished = None;
        let Some(layout) = buffer.layout() else {
            return;
        };
        if ev.key != PhysicalKey::Space || buffer.is_empty() {
            return;
        }
        let corrected = actions.iter().find_map(|action| match action {
            Action::Replace {
                text,
                kind: ReplaceKind::Auto | ReplaceKind::CaseFix,
                ..
            } => Some(text.chars().count()),
            _ => None,
        });
        let boundary = KeyStroke {
            key: ev.key,
            shift: ev.mods.shift,
        };
        self.finished = Some(Finished {
            keys: buffer.keys().to_vec(),
            layout: layout.clone(),
            app: app.clone(),
            boundary: (boundary, ' '),
            corrected,
        });
    }

    fn shift_down(&mut self, ev: &KeyEvent, view: &View, step: &mut Step) {
        let bit = mod_bit(ev.key).unwrap_or(0);
        if ev.repeat || self.held & bit != 0 {
            return;
        }
        let others = self.held;
        self.press_modifier(bit);
        if others & SHIFTS != 0 {
            // второй Shift, пока держится первый
            self.tap = None;
            self.pending = None;
            self.both = true;
            if let Some(chord) = &mut self.chord {
                chord.dirty = true;
            }
            self.fire(Hotkey::BothShifts, view, step);
            return;
        }
        if others != 0 {
            // Shift к нажатому Ctrl/Alt/Cmd — часть сочетания, не касание
            self.flush(view.config, step);
            self.tap = None;
            return;
        }
        let second = self.pending.is_some();
        self.tap = Some(Tap {
            key: ev.key,
            down_ms: ev.time_ms,
            clean: true,
            second,
        });
    }

    fn shift_up(&mut self, ev: &KeyEvent, ctx: &Context, view: &View, step: &mut Step) {
        if let Some(peak) = mod_bit(ev.key).and_then(|bit| self.release_modifier(bit)) {
            self.fire_chord(peak, view, step);
        }
        if self.both {
            self.both = self.held & SHIFTS != 0;
            return;
        }
        let Some(tap) = self.tap.take_if(|tap| tap.key == ev.key) else {
            return;
        };
        let quick = ev.time_ms.saturating_sub(tap.down_ms) <= T_TAP_MS;
        if !(tap.clean && quick) {
            if tap.second {
                self.flush(view.config, step);
            }
            return;
        }
        if tap.second {
            self.pending = None;
            self.fire(Hotkey::DoubleShift, view, step);
            return;
        }
        self.pending = Some(Pending {
            up_ms: ev.time_ms,
            target: switch_target(view.layouts, &ctx.layout).cloned(),
        });
        if find_command(view.config, |h| *h == Hotkey::DoubleShift).is_none() {
            // двойной Shift ни на что не назначен — ждать нечего
            self.flush(view.config, step);
        }
    }

    fn other_down(&mut self, ev: &KeyEvent, view: &View, step: &mut Step) {
        if let Some(tap) = &mut self.tap {
            tap.clean = false;
        }
        self.flush(view.config, step);
        match mod_bit(ev.key) {
            Some(bit) => self.press_modifier(bit),
            None => {
                if let Some(chord) = &mut self.chord {
                    chord.dirty = true;
                }
            }
        }
        if self.standard_undo(ev, view, step) {
            return;
        }
        let command = if ev.key.is_modifier() {
            None
        } else {
            find_command(view.config, |h| {
                matches!(h, Hotkey::Combo { mods, key: Some(key) }
                    if *key == ev.key && same_mods(*mods, ev.mods))
            })
        };
        if command != Some(Command::UndoOrConvertWord) && !is_undo_modifier(ev.key, view.os) {
            // первое нажатие, отличное от Shift, закрывает окно отмены и забывает слово;
            // Cmd/Ctrl стандартного «отменить» — нет, иначе ⌘Z/Ctrl+Z до окна не доживёт
            self.finished = None;
        }
        if let Some(command) = command {
            step.consumed = Some(Disposition::Swallow);
            if !ev.repeat {
                self.run(command, view, step);
            }
        }
    }

    /// Стандартное «отменить» ОС в открытом окне отмены — та же отмена, что ⇧⇧: нажатие и
    /// его автоповтор в приложение не уходят, а сама отмена ждёт отпускания Cmd/Ctrl
    /// (`other_up`). Вне окна — `false`: сочетание идёт в приложение и сбрасывает слово как
    /// обычно. Не переназначается и от `Config.hotkeys` не зависит.
    fn standard_undo(&mut self, ev: &KeyEvent, view: &View, step: &mut Step) -> bool {
        let combo = ev.key == PhysicalKey::KeyZ && same_mods(ev.mods, undo_mods(view.os));
        if self.queued_undo.is_some() && !ev.key.is_modifier() {
            if combo && ev.repeat {
                step.consumed = Some(Disposition::Swallow);
                return true;
            }
            // другая клавиша или новое нажатие Z: текст мог измениться — отмена не уходит
            self.queued_undo = None;
        }
        if !combo || ev.repeat {
            return false;
        }
        let Some(word) = self.finished.take_if(|f| f.corrected.is_some()) else {
            return false;
        };
        step.consumed = Some(Disposition::Swallow);
        self.queued_undo = Some(word);
        true
    }

    fn other_up(&mut self, ev: &KeyEvent, view: &View, step: &mut Step) {
        let Some(bit) = mod_bit(ev.key) else {
            return;
        };
        if let Some(peak) = self.release_modifier(bit) {
            self.fire_chord(peak, view, step);
        }
        if is_undo_modifier(ev.key, view.os) && self.held & undo_bits(view.os) == 0 {
            // Cmd/Ctrl отпущен: ждущая отмена уходит, без Z окно отмены закрывается
            match self.queued_undo.take() {
                Some(word) => self.undo(word, view.layouts, step),
                None => self.finished = None,
            }
        }
    }

    /// Отпускание Shift могло уйти мимо движка (безопасный ввод, отключённый перехват).
    /// Обычная клавиша без флага Shift — доказательство, что Shift отпущен: иначе следующее
    /// касание другого Shift засчиталось бы как «оба Shift» и выключило автоматику.
    fn reconcile_shift(&mut self, shift_flag: bool) {
        if shift_flag || self.held & SHIFTS == 0 {
            return;
        }
        self.held &= !SHIFTS;
        self.both = false;
        self.tap = None;
        if self.held == 0 {
            self.chord = None;
        }
    }

    fn press_modifier(&mut self, bit: u8) {
        if self.held == 0 {
            self.chord = Some(Chord::default());
        }
        self.held |= bit;
        if let Some(chord) = &mut self.chord {
            chord.peak |= bit;
        }
    }

    /// Отпущен модификатор; если отпущены все и между ними ничего не нажималось — аккорд
    /// (только Shift не в счёт: это касания).
    fn release_modifier(&mut self, bit: u8) -> Option<u8> {
        if self.held & bit == 0 {
            return None;
        }
        self.held &= !bit;
        if self.held != 0 {
            return None;
        }
        let chord = self.chord.take()?;
        (!chord.dirty && chord.peak & !SHIFTS != 0).then_some(chord.peak)
    }

    /// Ожидание кончилось без второго касания: это был одиночный Shift.
    fn flush(&mut self, config: &Config, step: &mut Step) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if let Some(tap) = &mut self.tap {
            tap.second = false;
        }
        if config.single_shift_switch
            && let Some(target) = pending.target
        {
            step.actions.push(Action::SwitchLayout(target.clone()));
            step.switched_to = Some(target);
        }
    }

    fn fire(&mut self, gesture: Hotkey, view: &View, step: &mut Step) {
        if let Some(command) = find_command(view.config, |h| *h == gesture) {
            self.run(command, view, step);
        }
    }

    fn fire_chord(&mut self, peak: u8, view: &View, step: &mut Step) {
        let chord = mods_of(peak);
        let command = find_command(
            view.config,
            |h| matches!(h, Hotkey::Combo { mods, key: None } if same_mods(*mods, chord)),
        );
        if let Some(command) = command {
            self.run(command, view, step);
        }
    }

    fn run(&mut self, command: Command, view: &View, step: &mut Step) {
        match command {
            Command::UndoOrConvertWord => self.undo_or_convert(view, step),
            Command::ConvertSelection => step.actions.push(Action::ConvertSelection),
            Command::ToggleAutocorrect => step.actions.push(Action::ToggleAutocorrect),
        }
    }

    /// Окно отмены открыто — отмена; иначе ручная конвертация набираемого слова,
    /// иначе — слова, только что закрытого пробелом; иначе ничего.
    fn undo_or_convert(&mut self, view: &View, step: &mut Step) {
        if let Some(word) = self.finished.take_if(|f| f.corrected.is_some()) {
            self.undo(word, view.layouts, step);
        } else if let Some(layout) = view.buffer.layout().filter(|_| !view.buffer.is_empty()) {
            if let Some((action, target)) = manual(view.buffer.keys(), layout, None, view.layouts) {
                step.actions.push(action);
                step.relayout = Some(target);
            }
        } else if let Some(word) = &mut self.finished
            && let Some((action, target)) =
                manual(&word.keys, &word.layout, Some(word.boundary), view.layouts)
        {
            step.actions.push(action);
            // повторный ⇧⇧ вернёт слово обратно
            word.layout = target;
        }
    }

    /// Стереть исправленное вместе с границей, напечатать исходные нажатия, раскладку обратно.
    fn undo(&mut self, word: Finished, layouts: &LayoutSet, step: &mut Step) {
        let (Some(erase), Some(from)) = (word.corrected, layouts.get(&word.layout)) else {
            return;
        };
        let Some(typed) = typed_form(&word.keys, from) else {
            return;
        };
        let mut keys: Vec<KeyStroke> = word
            .keys
            .iter()
            .map(|&(key, level)| KeyStroke {
                key,
                shift: level.is_shift(),
            })
            .collect();
        keys.push(word.boundary.0);
        let mut text = typed.clone();
        text.push(word.boundary.1);
        self.remember_undone(&typed);
        self.last_undone = Some(typed);
        step.actions.push(Action::Replace {
            erase,
            keys,
            text,
            target_layout: Some(word.layout),
            kind: ReplaceKind::Undo,
        });
    }
}

/// Ручная конвертация в другую раскладку пары: стереть слово (и границу, если она уже
/// напечатана) и напечатать те же нажатия в другой раскладке.
fn manual(
    keys: &[(PhysicalKey, Level)],
    layout: &LayoutId,
    boundary: Option<(KeyStroke, char)>,
    layouts: &LayoutSet,
) -> Option<(Action, LayoutId)> {
    let from = layouts.get(layout)?;
    let to = other_of(layouts, from.lang)?;
    let word = convert_keys(keys, from, to)?;
    let (mut strokes, mut text, mut erase) = (word.strokes, word.converted, keys.len());
    if let Some((stroke, ch)) = boundary {
        strokes.push(stroke);
        text.push(ch);
        erase += 1;
    }
    let action = Action::Replace {
        erase,
        keys: strokes,
        text,
        target_layout: Some(to.id.clone()),
        kind: ReplaceKind::Manual,
    };
    Some((action, to.id.clone()))
}

/// Первое по порядку действие, у которого есть подходящее назначение. Пустой список — действие
/// выключено; дубли между действиями отсекает `validate_hotkeys`.
fn find_command(config: &Config, matches: impl Fn(&Hotkey) -> bool) -> Option<Command> {
    let h = &config.hotkeys;
    [
        (Command::UndoOrConvertWord, &h.undo_or_convert_word),
        (Command::ConvertSelection, &h.convert_selection),
        (Command::ToggleAutocorrect, &h.toggle_autocorrect),
    ]
    .into_iter()
    .find(|(_, list)| list.iter().any(&matches))
    .map(|(command, _)| command)
}

/// Сравнение без CapsLock: парсер его не ставит, а у пользователя он может гореть.
fn same_mods(a: Modifiers, b: Modifiers) -> bool {
    a.shift == b.shift && a.alt == b.alt && a.ctrl == b.ctrl && a.meta == b.meta
}

/// Стандартное «отменить»: ⌘Z на macOS, Ctrl+Z на Windows и Linux.
fn undo_mods(os: Os) -> Modifiers {
    match os {
        Os::MacOs => Modifiers {
            meta: true,
            ..Modifiers::default()
        },
        Os::Windows | Os::Linux => Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    }
}

/// Биты (`mod_bit`) модификатора стандартного «отменить» этой ОС, обеих сторон.
fn undo_bits(os: Os) -> u8 {
    match os {
        Os::MacOs => 0b1100_0000,
        Os::Windows | Os::Linux => 0b0000_1100,
    }
}

/// Модификатор стандартного «отменить» этой ОС: его нажатие окно отмены не закрывает.
fn is_undo_modifier(key: PhysicalKey, os: Os) -> bool {
    match os {
        Os::MacOs => matches!(key, PhysicalKey::MetaLeft | PhysicalKey::MetaRight),
        Os::Windows | Os::Linux => {
            matches!(key, PhysicalKey::ControlLeft | PhysicalKey::ControlRight)
        }
    }
}

const SHIFTS: u8 = 0b0000_0011;

fn mod_bit(key: PhysicalKey) -> Option<u8> {
    Some(match key {
        PhysicalKey::ShiftLeft => 1,
        PhysicalKey::ShiftRight => 1 << 1,
        PhysicalKey::ControlLeft => 1 << 2,
        PhysicalKey::ControlRight => 1 << 3,
        PhysicalKey::AltLeft => 1 << 4,
        PhysicalKey::AltRight => 1 << 5,
        PhysicalKey::MetaLeft => 1 << 6,
        PhysicalKey::MetaRight => 1 << 7,
        _ => return None,
    })
}

fn mods_of(bits: u8) -> Modifiers {
    Modifiers {
        shift: bits & SHIFTS != 0,
        ctrl: bits & 0b0000_1100 != 0,
        alt: bits & 0b0011_0000 != 0,
        meta: bits & 0b1100_0000 != 0,
        caps_lock: false,
    }
}

/// Другая раскладка пары: из английской — в русскую, из русской — в английскую.
fn other_of(layouts: &LayoutSet, lang: Lang) -> Option<&LayoutMap> {
    let (en, ru) = layouts.pair()?;
    match lang {
        Lang::En => Some(ru),
        Lang::Ru => Some(en),
        Lang::Other => None,
    }
}

fn switch_target<'a>(layouts: &'a LayoutSet, current: &LayoutId) -> Option<&'a LayoutId> {
    other_of(layouts, layouts.get(current)?.lang).map(|map| &map.id)
}

impl WordRules {
    /// Можно ли взять токен в список «никогда»: из 1–2 символов — нельзя.
    pub fn accepts(word: &str) -> bool {
        word.trim().chars().count() >= NEVER_MIN_CHARS
    }

    /// Набранный токен есть в списке: без учёта регистра, хвостовые знаки можно не писать.
    pub fn contains(&self, typed: &str) -> bool {
        listed(&self.never, typed)
    }

    /// Добавить токен явным действием пользователя; `false` — не принят или уже есть.
    pub fn add(&mut self, word: &str) -> bool {
        let word = word.trim();
        if !Self::accepts(word) || self.contains(word) {
            return false;
        }
        self.never.push(word.to_owned());
        true
    }
}

/// Слово без хвостовых знаков («ИИ,» → «ИИ»); токен из одних знаков — целиком.
fn word_core(typed: &str) -> &str {
    let core = typed.trim_end_matches(|c: char| !ks_data::is_word_char(c));
    if core.is_empty() { typed } else { core }
}

/// Набранная форма: символы нажатий в раскладке, где их набрали.
fn typed_form(keys: &[(PhysicalKey, Level)], from: &LayoutMap) -> Option<String> {
    keys.iter()
        .map(|&(key, level)| from.char_for(key, level))
        .collect()
}

/// Токен есть в списке слов: без учёта регистра, хвостовые знаки можно не писать.
fn listed(words: &[String], typed: &str) -> bool {
    let core = typed.trim_end_matches(|c: char| !ks_data::is_word_char(c));
    words
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .any(|w| same_word(w, typed) || same_word(w, core))
}

fn same_word(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}
