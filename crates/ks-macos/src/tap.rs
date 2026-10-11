//! Логика колбэка перехвата без системных вызовов: что передать движку, что пропустить,
//! что придержать на время замены, когда включить перехват снова. Колбэк `CGEventTap`
//! (`sys::tap`) переводит `CGEvent` в [`RawEvent`] и отдаёт сюда.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use ks_core::{Action, Context, Disposition, KeyDir, KeyEvent, ResetReason};
use ks_platform::{BackendError, KeyHandler};

use crate::exec::Executor;
use crate::keycode;

/// `kCGEventSourceUserData` своих событий замены: движок их не видит.
pub const MARK_OWN: i64 = 0x4B53_0001;
/// `kCGEventSourceUserData` событий `--selftest`: обрабатываются как нажатия пользователя.
pub const MARK_SELFTEST: i64 = 0x4B53_0002;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    KeyDown,
    KeyUp,
    FlagsChanged,
    MouseDown,
    DisabledByTimeout,
    DisabledByUserInput,
    Other,
}

/// Событие перехвата в том виде, в каком его видит логика. Намеренно без `Debug`:
/// код клавиши — это набранный символ, в лог он попасть не должен.
#[derive(Clone, Copy)]
pub struct RawEvent {
    pub kind: RawKind,
    pub keycode: u16,
    pub flags: u64,
    pub autorepeat: bool,
    pub user_data: i64,
    /// Процесс-получатель (`kCGEventTargetUnixProcessID`); 0 — неизвестен.
    pub pid: i32,
    pub time_ms: u64,
}

impl RawEvent {
    pub fn key(kind: RawKind, keycode: u16, flags: u64) -> RawEvent {
        RawEvent {
            kind,
            keycode,
            flags,
            autorepeat: false,
            user_data: 0,
            pid: 0,
            time_ms: 0,
        }
    }
}

/// Сколько исполнитель может держать клавиатуру. Дольше — он признан зависшим: удержанные
/// нажатия сбрасываются, клавиатура отпускается, исправления на паузе, пока он не отвиснет.
pub const BUSY_TIMEOUT_MS: u64 = 1500;

/// Единые монотонные часы бэкенда, мс от первого вызова: ими меряется `KeyEvent.time_ms`,
/// ими же следует звать `Engine::tick(now_ms)`.
pub fn clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Задание исполнителю: действия движка и нажатие, которое надо пропустить в приложение
/// уже после них (`Disposition::Pass` с действиями: раскладка сменится раньше буквы).
pub struct Job {
    pub actions: Vec<Action>,
    pub then_post: Option<RawEvent>,
    /// Поколение из [`TapShared::begin`]; задание сброшенного поколения не исполняется.
    pub generation: u64,
    /// Куда вернуть итог действий (синхронный `Backend::execute`).
    pub reply: Option<Sender<Result<(), BackendError>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Swallow,
}

/// Включение перехвата заново (`CGEventTapEnable`).
pub trait TapControl {
    fn reenable(&self);
}

/// Снимок окружения для движка; читает кэши, без вызовов TIS.
pub trait ContextSource: Send + Sync {
    fn context(&self, pid: i32) -> Context;
}

enum Queued {
    Key(RawEvent),
    Flags(RawEvent),
    Reset(ResetReason),
}

/// Пока `in_flight > 0`, нажатия пользователя придерживаются в `events`.
#[derive(Default)]
struct Pending {
    in_flight: u32,
    since_ms: u64,
    generation: u64,
    stalled: bool,
    needs_reset: bool,
    events: VecDeque<Queued>,
}

impl Pending {
    fn abort(&mut self, stalled: bool) {
        self.in_flight = 0;
        self.events.clear();
        self.generation += 1;
        self.stalled = stalled;
        self.needs_reset = true;
    }
}

/// Состояние, общее для потока перехвата, исполнителя и главного потока.
#[derive(Default)]
pub struct TapShared {
    pending: Mutex<Pending>,
    /// Есть право на эмуляцию ввода (кэш, обновляется с главного потока).
    pub post_allowed: AtomicBool,
    /// Перехват поднят и включён.
    pub capturing: AtomicBool,
    /// Сколько раз система отключала перехват и он был включён снова.
    pub reenabled: AtomicU64,
    /// Сколько раз исполнитель замены падал или не укладывался в срок.
    pub executor_failures: AtomicU64,
}

impl TapShared {
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Идёт замена: нажатия пользователя придерживаются.
    pub fn busy(&self) -> bool {
        self.pending().in_flight > 0
    }

    /// Сколько нажатий придержано.
    pub fn held(&self) -> usize {
        self.pending().events.len()
    }

    /// Исполнитель признан зависшим: новые задания не берутся.
    pub fn stalled(&self) -> bool {
        self.pending().stalled
    }

    /// Начинает задание исполнителя: с этого момента нажатия придерживаются.
    /// `None` — исполнитель завис, задание не начато.
    pub fn begin(&self, now_ms: u64) -> Option<u64> {
        let mut p = self.pending();
        if p.stalled {
            return None;
        }
        if p.in_flight == 0 {
            p.since_ms = now_ms;
        }
        p.in_flight += 1;
        Some(p.generation)
    }

    /// Поколение задания ещё текущее (срок его не срезал).
    pub fn is_current(&self, generation: u64) -> bool {
        self.pending().generation == generation
    }

    /// Исполнитель снова жив (вернулся из задания, срезанного по сроку): пауза снимается.
    fn recovered(&self) {
        self.pending().stalled = false;
    }

    /// Отпускает клавиатуру и сбрасывает удержанное, если поколение ещё текущее.
    pub fn abort(&self, generation: u64) -> bool {
        let mut p = self.pending();
        if p.generation != generation {
            return false;
        }
        p.abort(false);
        true
    }
}

pub fn to_key_event(ev: &RawEvent) -> Option<KeyEvent> {
    let key = keycode::to_physical(ev.keycode);
    let dir = match ev.kind {
        RawKind::KeyDown => KeyDir::Down,
        RawKind::KeyUp => KeyDir::Up,
        RawKind::FlagsChanged => keycode::modifier_dir(key, ev.flags)?,
        _ => return None,
    };
    Some(KeyEvent {
        key,
        dir,
        mods: keycode::modifiers(ev.flags),
        repeat: ev.autorepeat,
        time_ms: ev.time_ms,
    })
}

fn os_actions(actions: Vec<Action>) -> Vec<Action> {
    actions
        .into_iter()
        .filter(|a| !matches!(a, Action::ToggleAutocorrect))
        .collect()
}

pub struct TapLogic {
    handler: Arc<dyn KeyHandler>,
    ctx: Arc<dyn ContextSource>,
    shared: Arc<TapShared>,
    jobs: Sender<Job>,
    last_pid: AtomicI32,
}

impl TapLogic {
    pub fn new(
        handler: Arc<dyn KeyHandler>,
        ctx: Arc<dyn ContextSource>,
        shared: Arc<TapShared>,
        jobs: Sender<Job>,
    ) -> TapLogic {
        TapLogic {
            handler,
            ctx,
            shared,
            jobs,
            last_pid: AtomicI32::new(0),
        }
    }

    /// Решение по одному событию перехвата; вызывается синхронно из колбэка.
    pub fn handle(&self, ev: &RawEvent, control: &dyn TapControl) -> Verdict {
        match ev.kind {
            RawKind::DisabledByTimeout | RawKind::DisabledByUserInput => {
                // Система сняла перехват (долгий колбэк или перегрузка): включаем сразу,
                // слово в работе сбрасываем — часть нажатий могла пройти мимо.
                control.reenable();
                self.shared.reenabled.fetch_add(1, Ordering::Relaxed);
                log::warn!("перехват был отключён системой и включён снова");
                self.reset(ResetReason::External);
                return Verdict::Pass;
            }
            RawKind::Other => return Verdict::Pass,
            _ => {}
        }
        if ev.user_data == MARK_OWN {
            return Verdict::Pass;
        }
        if ev.kind == RawKind::MouseDown {
            self.reset(ResetReason::MouseClick);
            return Verdict::Pass;
        }
        if !self.shared.post_allowed.load(Ordering::Relaxed) {
            // Без права на эмуляцию замена невозможна: ничего не глотаем, движок не кормим.
            return Verdict::Pass;
        }
        if ev.pid != 0 {
            let last = self.last_pid.swap(ev.pid, Ordering::Relaxed);
            if last != 0 && last != ev.pid {
                self.reset(ResetReason::FocusChanged);
            }
        }
        {
            let mut pending = self.shared.pending();
            if pending.in_flight > 0
                && ev.time_ms.saturating_sub(pending.since_ms) > BUSY_TIMEOUT_MS
            {
                pending.abort(true);
                self.shared
                    .executor_failures
                    .fetch_add(1, Ordering::Relaxed);
                log::error!(
                    "исполнитель замены не ответил за {BUSY_TIMEOUT_MS} мс: удержанные нажатия \
                     сброшены, исправления на паузе"
                );
            }
            if pending.in_flight > 0 {
                return if ev.kind == RawKind::FlagsChanged {
                    pending.events.push_back(Queued::Flags(*ev));
                    Verdict::Pass
                } else {
                    pending.events.push_back(Queued::Key(*ev));
                    Verdict::Swallow
                };
            }
            if pending.stalled {
                return Verdict::Pass;
            }
        }
        self.reset_if_needed();
        let Some(kev) = to_key_event(ev) else {
            return Verdict::Pass;
        };
        let outcome = self.handler.on_key(&kev, &self.ctx.context(ev.pid));
        let actions = os_actions(outcome.actions);
        let swallow = outcome.disposition == Disposition::Swallow;
        if actions.is_empty() {
            return if swallow {
                Verdict::Swallow
            } else {
                Verdict::Pass
            };
        }
        // Pass с действиями: клавишу придерживаем и пропускаем после действий. Модификаторы
        // (`FlagsChanged`) не воспроизводятся — их пропускаем сразу.
        let then_post = (!swallow && ev.kind != RawKind::FlagsChanged).then_some(*ev);
        let held = then_post.is_some();
        let Some(generation) = self.shared.begin(ev.time_ms) else {
            return Verdict::Pass;
        };
        let job = Job {
            actions,
            then_post,
            generation,
            reply: None,
        };
        if self.jobs.send(job).is_err() {
            self.shared.abort(generation);
            return Verdict::Pass;
        }
        if swallow || held {
            Verdict::Swallow
        } else {
            Verdict::Pass
        }
    }

    fn reset_if_needed(&self) {
        let needed = std::mem::take(&mut self.shared.pending().needs_reset);
        if needed {
            self.handler.on_reset(ResetReason::External);
        }
    }

    fn reset(&self, why: ResetReason) {
        let mut pending = self.shared.pending();
        if pending.in_flight > 0 {
            pending.events.push_back(Queued::Reset(why));
        } else {
            drop(pending);
            self.handler.on_reset(why);
        }
    }
}

/// Снимает удержание, если исполнитель раскручивает стек после паники.
struct UnwindGuard<'a> {
    shared: &'a TapShared,
    generation: u64,
    armed: bool,
}

impl Drop for UnwindGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.shared.abort(self.generation) {
            self.shared
                .executor_failures
                .fetch_add(1, Ordering::Relaxed);
            log::error!(
                "исполнитель замены упал: удержанные нажатия сброшены, клавиатура отпущена"
            );
        } else {
            // Задание уже срезано по сроку; исполнитель пережил панику — пауза не вечна.
            self.shared.recovered();
        }
    }
}

/// Исполнитель заданий: выполняет действия, затем прогоняет через движок придержанные
/// нажатия по порядку и воспроизводит пропущенные движком. Единственный путь исполнения
/// действий — и из колбэка, и из `Backend::execute`.
pub struct Pipeline {
    pub handler: Arc<dyn KeyHandler>,
    pub ctx: Arc<dyn ContextSource>,
    pub shared: Arc<TapShared>,
    pub exec: Executor,
}

impl Pipeline {
    pub fn run(&self, job: Job) {
        let Job {
            actions,
            then_post,
            generation,
            reply,
        } = job;
        if !self.current(generation) {
            if let Some(reply) = reply {
                let _ = reply.send(Err(BackendError::Os(
                    "задание отменено: исполнитель был сброшен по сроку".to_owned(),
                )));
            }
            return;
        }
        let mut guard = UnwindGuard {
            shared: &self.shared,
            generation,
            armed: true,
        };
        let result = self.execute_all(&actions, generation);
        if !self.current(generation) {
            // Срезано по сроку посреди задания: ни остатка замены, ни then_post.
            guard.armed = false;
            if let Some(reply) = reply {
                let _ = reply.send(Err(BackendError::Os(
                    "задание отменено по сроку".to_owned(),
                )));
            }
            return;
        }
        if let Some(ev) = &then_post {
            self.exec.replay(ev);
        }
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
        self.drain(generation);
        guard.armed = false;
    }

    /// Поколение ещё текущее; иначе исполнитель отвис после сброса — снимаем паузу.
    fn current(&self, generation: u64) -> bool {
        let mut p = self.shared.pending();
        if p.generation != generation {
            p.stalled = false;
            return false;
        }
        true
    }

    fn drain(&self, generation: u64) {
        loop {
            let next = {
                let mut p = self.shared.pending();
                if p.generation != generation {
                    p.stalled = false;
                    return;
                }
                match p.events.pop_front() {
                    Some(next) => next,
                    None => {
                        p.in_flight = p.in_flight.saturating_sub(1);
                        return;
                    }
                }
            };
            match next {
                Queued::Reset(why) => self.handler.on_reset(why),
                Queued::Flags(ev) => {
                    if let Some(kev) = to_key_event(&ev) {
                        let out = self.handler.on_key(&kev, &self.ctx.context(ev.pid));
                        let _ = self.execute_all(&os_actions(out.actions), generation);
                    }
                }
                Queued::Key(ev) => {
                    let Some(kev) = to_key_event(&ev) else {
                        continue;
                    };
                    let out = self.handler.on_key(&kev, &self.ctx.context(ev.pid));
                    let _ = self.execute_all(&os_actions(out.actions), generation);
                    if out.disposition == Disposition::Pass && self.shared.is_current(generation) {
                        self.exec.replay(&ev);
                    }
                }
            }
        }
    }

    fn execute_all(&self, actions: &[Action], generation: u64) -> Result<(), BackendError> {
        let alive = || self.shared.is_current(generation);
        let mut first = Ok(());
        for action in actions {
            if let Err(e) = self.exec.execute_while(action, &alive) {
                log::warn!("действие не исполнено: {e}");
                if first.is_ok() {
                    first = Err(e);
                }
            }
        }
        first
    }
}
