//! Модель исполнения ОС-слоя без системных вызовов — общая для macOS, Windows и Linux:
//! что передать движку, что пропустить, что придержать на время замены, когда включить
//! перехват снова, как исполнитель прогоняет придержанное. Колбэк перехвата бэкенда переводит
//! событие ОС в [`InputEvent`] и отдаёт сюда; замены исполняет поток [`spawn_executor`].
//!
//! Обязательства бэкенда: модификаторы приходят отдельными событиями `Modifier` со своей
//! `PhysicalKey` с учётом стороны; свои синтетические события помечены (`own`); `Pass` с
//! действиями держит нажатие и отправляет его после действий (`Job::then_post`), кроме
//! модификаторов — они пропускаются сразу.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ks_core::{Action, Context, Disposition, KeyEvent, ResetReason};

use crate::exec::{Executor, Lease};
use crate::{BackendError, KeyHandler, clock_ms};

/// Сколько исполнитель может держать клавиатуру без продления. Дольше — он признан зависшим:
/// удержанные нажатия сбрасываются, клавиатура отпускается, исправления на паузе, пока он не
/// отвиснет.
pub const BUSY_TIMEOUT_MS: u64 = 1500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// Обычная клавиша (нажатие или отпускание).
    Key,
    /// Клавиша-модификатор: пропускается сразу, не воспроизводится.
    Modifier,
    MouseDown,
    /// Система отключила перехват (долгий колбэк, перегрузка): включить снова.
    Disabled,
    Other,
}

/// Событие перехвата в нейтральном виде. Намеренно без `Debug`: внутри набранная клавиша.
#[derive(Clone, Copy)]
pub struct InputEvent<N> {
    pub kind: InputKind,
    /// Нажатие для движка; `None` — бэкенд его не перевёл (неизвестный код, модификатор без
    /// направления). Такое событие пропускается в приложение как есть.
    pub key: Option<KeyEvent>,
    /// Своё синтетическое событие: движку не отдаётся.
    pub own: bool,
    /// По часам [`clock_ms`].
    pub time_ms: u64,
    /// Событие ОС как есть — для воспроизведения придержанного.
    pub native: N,
}

impl<N> InputEvent<N> {
    pub fn new(kind: InputKind, key: Option<KeyEvent>, time_ms: u64, native: N) -> InputEvent<N> {
        InputEvent {
            kind,
            key,
            own: false,
            time_ms,
            native,
        }
    }
}

/// Задание исполнителю: действия движка и нажатие, которое надо пропустить в приложение
/// уже после них (`Disposition::Pass` с действиями: раскладка сменится раньше буквы).
pub struct Job<N> {
    pub actions: Vec<Action>,
    pub then_post: Option<InputEvent<N>>,
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

/// Включение перехвата заново.
pub trait TapControl {
    fn reenable(&self);
}

/// Снимок окружения для движка: приложение в фокусе, раскладка, безопасный ввод. Читает
/// кэши бэкенда, без системных вызовов в горячем пути.
pub trait ContextSource: Send + Sync {
    fn context(&self) -> Context;
}

enum Queued<N> {
    Key(InputEvent<N>),
    Modifier(InputEvent<N>),
    Reset(ResetReason),
}

/// Пока `in_flight > 0`, нажатия пользователя придерживаются в `events`.
struct Pending<N> {
    in_flight: u32,
    since_ms: u64,
    generation: u64,
    stalled: bool,
    needs_reset: bool,
    events: VecDeque<Queued<N>>,
}

impl<N> Default for Pending<N> {
    fn default() -> Self {
        Pending {
            in_flight: 0,
            since_ms: 0,
            generation: 0,
            stalled: false,
            needs_reset: false,
            events: VecDeque::new(),
        }
    }
}

impl<N> Pending<N> {
    fn abort(&mut self, stalled: bool) {
        self.in_flight = 0;
        self.events.clear();
        self.generation += 1;
        self.stalled = stalled;
        self.needs_reset = true;
    }
}

/// Состояние, общее для потока перехвата, исполнителя и главного потока.
pub struct TapShared<N> {
    pending: Mutex<Pending<N>>,
    /// Есть право на эмуляцию ввода (кэш, обновляется бэкендом).
    pub post_allowed: AtomicBool,
    /// Перехват поднят и включён.
    pub capturing: AtomicBool,
    /// Сколько раз система отключала перехват и он был включён снова.
    pub reenabled: AtomicU64,
    /// Сколько раз исполнитель замены падал или не укладывался в срок.
    pub executor_failures: AtomicU64,
}

impl<N> Default for TapShared<N> {
    fn default() -> Self {
        TapShared {
            pending: Mutex::new(Pending::default()),
            post_allowed: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            reenabled: AtomicU64::new(0),
            executor_failures: AtomicU64::new(0),
        }
    }
}

impl<N> TapShared<N> {
    fn pending(&self) -> MutexGuard<'_, Pending<N>> {
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

    /// Живой исполнитель перед необратимым шагом: если до среза меньше `need_ms`, срок
    /// отсчитывается заново от `now_ms`. `false` — поколение уже срезано.
    pub fn reserve(&self, generation: u64, now_ms: u64, need_ms: u64) -> bool {
        let mut p = self.pending();
        if p.generation != generation {
            return false;
        }
        if (p.since_ms + BUSY_TIMEOUT_MS).saturating_sub(now_ms) < need_ms {
            p.since_ms = now_ms;
        }
        true
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

fn os_actions(actions: Vec<Action>) -> Vec<Action> {
    actions
        .into_iter()
        .filter(|a| !matches!(a, Action::ToggleAutocorrect))
        .collect()
}

pub struct TapLogic<N> {
    handler: Arc<dyn KeyHandler>,
    ctx: Arc<dyn ContextSource>,
    shared: Arc<TapShared<N>>,
    jobs: Sender<Job<N>>,
}

impl<N: Copy + Send> TapLogic<N> {
    pub fn new(
        handler: Arc<dyn KeyHandler>,
        ctx: Arc<dyn ContextSource>,
        shared: Arc<TapShared<N>>,
        jobs: Sender<Job<N>>,
    ) -> TapLogic<N> {
        TapLogic {
            handler,
            ctx,
            shared,
            jobs,
        }
    }

    /// Решение по одному событию перехвата; вызывается синхронно из колбэка.
    pub fn handle(&self, ev: &InputEvent<N>, control: &dyn TapControl) -> Verdict {
        match ev.kind {
            InputKind::Disabled => {
                // Слово в работе сбрасываем: часть нажатий могла пройти мимо.
                control.reenable();
                self.shared.reenabled.fetch_add(1, Ordering::Relaxed);
                log::warn!("перехват был отключён системой и включён снова");
                self.reset(ResetReason::External);
                return Verdict::Pass;
            }
            InputKind::Other => return Verdict::Pass,
            _ => {}
        }
        if ev.own {
            return Verdict::Pass;
        }
        if ev.kind == InputKind::MouseDown {
            self.reset(ResetReason::MouseClick);
            return Verdict::Pass;
        }
        if !self.shared.post_allowed.load(Ordering::Relaxed) {
            // Без права на эмуляцию замена невозможна: ничего не глотаем, движок не кормим.
            return Verdict::Pass;
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
                return if ev.kind == InputKind::Modifier {
                    pending.events.push_back(Queued::Modifier(*ev));
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
        let Some(kev) = ev.key else {
            return Verdict::Pass;
        };
        let outcome = self.handler.on_key(&kev, &self.ctx.context());
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
        // не воспроизводятся — их пропускаем сразу.
        let then_post = (!swallow && ev.kind != InputKind::Modifier).then_some(*ev);
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

    /// Сброс слова в работе (клик, смена приложения в фокусе, безопасный ввод). Во время
    /// замены встаёт в очередь удержанного, чтобы не обогнать придержанные нажатия.
    pub fn reset(&self, why: ResetReason) {
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
struct UnwindGuard<'a, N> {
    shared: &'a TapShared<N>,
    generation: u64,
    armed: bool,
}

impl<N> Drop for UnwindGuard<'_, N> {
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

/// Срок одного задания по общим часам.
struct JobLease<'a, N> {
    shared: &'a TapShared<N>,
    generation: u64,
    clock: &'a (dyn Fn() -> u64 + Send + Sync),
}

impl<N> Lease for JobLease<'_, N> {
    fn alive(&self) -> bool {
        self.shared.is_current(self.generation)
    }
    fn reserve(&self, ms: u64) -> bool {
        self.shared.reserve(self.generation, (self.clock)(), ms)
    }
}

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Исполнитель заданий: выполняет действия, затем прогоняет через движок придержанные
/// нажатия по порядку и воспроизводит пропущенные движком. Единственный путь исполнения
/// действий — и из колбэка, и из `Backend::execute`.
pub struct Pipeline<N> {
    handler: Arc<dyn KeyHandler>,
    ctx: Arc<dyn ContextSource>,
    shared: Arc<TapShared<N>>,
    exec: Executor<N>,
    clock: Clock,
}

impl<N: Copy + Send> Pipeline<N> {
    pub fn new(
        handler: Arc<dyn KeyHandler>,
        ctx: Arc<dyn ContextSource>,
        shared: Arc<TapShared<N>>,
        exec: Executor<N>,
    ) -> Pipeline<N> {
        Pipeline {
            handler,
            ctx,
            shared,
            exec,
            clock: Arc::new(clock_ms),
        }
    }

    /// Свои часы вместо [`clock_ms`] (тесты срока).
    pub fn with_clock(mut self, clock: Clock) -> Pipeline<N> {
        self.clock = clock;
        self
    }

    pub fn run(&self, job: Job<N>) {
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
                Queued::Modifier(ev) => {
                    if let Some(kev) = ev.key {
                        let out = self.handler.on_key(&kev, &self.ctx.context());
                        let _ = self.execute_all(&os_actions(out.actions), generation);
                    }
                }
                Queued::Key(ev) => {
                    let pass = match ev.key {
                        Some(kev) => {
                            let out = self.handler.on_key(&kev, &self.ctx.context());
                            let _ = self.execute_all(&os_actions(out.actions), generation);
                            out.disposition == Disposition::Pass
                        }
                        None => true,
                    };
                    if pass && self.shared.is_current(generation) {
                        self.exec.replay(&ev);
                    }
                }
            }
        }
    }

    fn execute_all(&self, actions: &[Action], generation: u64) -> Result<(), BackendError> {
        let lease = JobLease {
            shared: &self.shared,
            generation,
            clock: &*self.clock,
        };
        let mut first = Ok(());
        for action in actions {
            let done = match action {
                Action::ConvertSelection => self.exec.convert_selection(
                    &self.ctx.context().app,
                    &|text| self.handler.convert_text(text),
                    &lease,
                ),
                _ => self.exec.execute_with(action, &lease),
            };
            if let Err(e) = done {
                log::warn!("действие не исполнено: {e}");
                if first.is_ok() {
                    first = Err(e);
                }
            }
        }
        first
    }
}

/// Поднимает поток исполнителя `ks-exec`: паника в задании не убивает поток, удержание
/// снимает `Pipeline`.
pub fn spawn_executor<N: Copy + Send + 'static>(
    pipeline: Pipeline<N>,
) -> Result<Sender<Job<N>>, BackendError> {
    let (tx, rx) = mpsc::channel::<Job<N>>();
    std::thread::Builder::new()
        .name("ks-exec".to_owned())
        .spawn(move || {
            for job in rx {
                if catch_unwind(AssertUnwindSafe(|| pipeline.run(job))).is_err() {
                    log::error!("задание исполнителя завершилось паникой");
                }
            }
        })
        .map_err(|e| BackendError::Os(e.to_string()))?;
    Ok(tx)
}

/// `Backend::execute` через ту же очередь и то же удержание, что и замены из колбэка.
/// `wait` — ждать итога (не с главного потока, если исполнителю нужен главный поток);
/// без ожидания задание ставится в очередь и ответ — `Ok(())`, ошибка уходит в лог.
pub fn submit<N>(
    shared: &TapShared<N>,
    jobs: &Sender<Job<N>>,
    action: &Action,
    wait: bool,
) -> Result<(), BackendError> {
    let Some(generation) = shared.begin(clock_ms()) else {
        return Err(BackendError::Os(
            "исполнитель замены не отвечает".to_owned(),
        ));
    };
    let (reply_tx, reply_rx) = mpsc::channel();
    let job = Job {
        actions: vec![action.clone()],
        then_post: None,
        generation,
        reply: wait.then_some(reply_tx),
    };
    if jobs.send(job).is_err() {
        shared.abort(generation);
        return Err(BackendError::Os("исполнитель не запущен".to_owned()));
    }
    if !wait {
        return Ok(());
    }
    reply_rx
        .recv_timeout(Duration::from_millis(2 * BUSY_TIMEOUT_MS))
        .unwrap_or_else(|_| Err(BackendError::Os("исполнитель не ответил".to_owned())))
}
