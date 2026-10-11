//! Жесты Shift и горячие клавиши — точка расширения задачи 04.
//!
//! `Engine::on_key` спрашивает этот модуль первым о каждом нажатии и отпускании;
//! `Some(outcome)` означает, что нажатие — часть жеста или хоткея и дальше не идёт.

use crate::{Config, Context, KeyEvent, Outcome, ResetReason};

#[derive(Debug, Default)]
pub(crate) struct InputState {}

impl InputState {
    pub(crate) fn on_key(
        &mut self,
        _ev: &KeyEvent,
        _ctx: &Context,
        _config: &Config,
    ) -> Option<Outcome> {
        None
    }

    pub(crate) fn on_reset(&mut self, _why: ResetReason) {}
}
