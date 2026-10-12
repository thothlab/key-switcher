//! Смена раскладки в пределах одного бюджета (`SWITCH_TIMEOUT_MS`): каждый вызов окружения
//! получает только остаток; ответ, пришедший после срока, — неудача.

use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Окружение: команда смены раскладки и чтение активной; оба вызова укладываются в `budget`.
pub trait LayoutControl {
    fn switch_to(&self, index: usize, budget: Duration) -> bool;
    fn current(&self, budget: Duration) -> Option<usize>;
}

/// Шаг опроса активной раскладки после команды.
pub const POLL: Duration = Duration::from_millis(15);

/// Переключает на `index` и ждёт подтверждения, всё вместе — не дольше `total`.
pub fn select(ctrl: &dyn LayoutControl, index: usize, total: Duration) -> bool {
    let start = Instant::now();
    let left = || total.saturating_sub(start.elapsed());
    let budget = left();
    if budget.is_zero() || !ctrl.switch_to(index, budget) || start.elapsed() > total {
        return false;
    }
    loop {
        let budget = left();
        if budget.is_zero() {
            return false;
        }
        let now = ctrl.current(budget);
        if start.elapsed() > total {
            return false;
        }
        if now == Some(index) {
            return true;
        }
        let budget = left();
        if budget.is_zero() {
            return false;
        }
        std::thread::sleep(POLL.min(budget));
    }
}

/// Вызов, который сам срок не соблюдает (D-Bus, сокет, X11), — на своём потоке; `None` — не
/// уложился в `budget` (поток доработает сам, результат выброшен).
pub fn with_deadline<T: Send + 'static>(
    budget: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    if budget.is_zero() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("ks-call".to_owned())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .ok()?;
    rx.recv_timeout(budget).ok()
}
