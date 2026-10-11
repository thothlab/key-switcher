//! Таймер ручного управления: одиночный Shift ждёт `T_double`, не начало ли это двойного.
//! Поток ждёт срок `next_deadline` (сравнение строгое: зовёт `tick` на `deadline + 1`),
//! действия исполняет бэкенд через `Backend::execute` — раскладка переключается без
//! следующего нажатия.

use std::sync::Arc;
use std::time::Duration;

use ks_platform::{Backend, KeyHandler, clock_ms};

use crate::handler::EngineHandler;

/// Зовёт `tick`, если срок прошёл к `now_ms`, и отдаёт действия бэкенду. Блокировка движка
/// отпущена до `execute`: исполнитель снова зовёт `on_key` для удержанных нажатий.
/// Возвращает число исполненных действий.
pub fn run_due(handler: &dyn KeyHandler, backend: &dyn Backend, now_ms: u64) -> usize {
    if !handler.next_deadline().is_some_and(|d| now_ms > d) {
        return 0;
    }
    let actions = handler.tick(now_ms);
    for action in &actions {
        if let Err(e) = backend.execute(action) {
            log::warn!("действие таймера не исполнено: {e}");
        }
    }
    actions.len()
}

/// Поднимает поток таймера на всё время жизни агента.
pub fn spawn(handler: Arc<EngineHandler>, backend: Arc<dyn Backend>) {
    let spawned = std::thread::Builder::new()
        .name("ks-timer".to_owned())
        .spawn(move || {
            loop {
                if handler.wait_deadline(&clock_ms, Duration::from_secs(60)) {
                    run_due(&*handler, &*backend, clock_ms());
                }
            }
        });
    if let Err(e) = spawned {
        log::error!("таймер ручного управления не поднят: {e}");
    }
}
