//! Виртуальная клавиатура как набор нажатых ею кодов: всё, что устройство нажало, оно же и
//! отпускает — после каждого нажатия-отпускания, при ошибке записи и по окончании задания
//! (`release_all` зовёт сторож бэкенда, когда исполнитель свободен).

use std::io;

use crate::keymap::{KEY_LEFTCTRL, KEY_LEFTSHIFT};

/// Запись одного события клавиши (с синхронизацией) в устройство.
pub trait Emit: Send {
    fn emit(&mut self, code: u16, value: i32) -> io::Result<()>;
}

pub struct Keys<E> {
    dev: Option<E>,
    pressed: Vec<u16>,
}

impl<E: Emit> Default for Keys<E> {
    fn default() -> Self {
        Keys {
            dev: None,
            pressed: Vec::new(),
        }
    }
}

impl<E: Emit> Keys<E> {
    pub fn attach(&mut self, dev: E) {
        self.dev = Some(dev);
    }

    pub fn ready(&self) -> bool {
        self.dev.is_some()
    }

    /// Коды, которые устройство нажало и ещё не отпустило.
    pub fn pressed(&self) -> &[u16] {
        &self.pressed
    }

    fn send(&mut self, code: u16, value: i32) -> bool {
        let Some(dev) = self.dev.as_mut() else {
            return false;
        };
        match dev.emit(code, value) {
            Ok(()) => {
                if value == 0 {
                    self.pressed.retain(|c| *c != code);
                } else if !self.pressed.contains(&code) {
                    self.pressed.push(code);
                }
                true
            }
            Err(e) => {
                log::warn!("uinput: событие не отправлено: {e}");
                self.release_all();
                false
            }
        }
    }

    /// Нажатие или отпускание клавиши, с Shift или без. Перед нажатием и после отпускания на
    /// устройстве не остаётся ничего зажатого.
    pub fn key(&mut self, code: u16, down: bool, shift: bool) {
        if down {
            self.release_all();
            if shift && !self.send(KEY_LEFTSHIFT, 1) {
                return;
            }
            self.send(code, 1);
        } else {
            if self.pressed.contains(&code) {
                self.send(code, 0);
            }
            self.release_all();
        }
    }

    /// Ctrl+клавиша.
    pub fn chord(&mut self, code: u16) {
        self.release_all();
        if self.send(KEY_LEFTCTRL, 1) && self.send(code, 1) {
            self.send(code, 0);
        }
        self.release_all();
    }

    /// Отпускает всё, что устройство нажало; неотпущенное (ошибка записи) остаётся в списке до
    /// следующей попытки.
    pub fn release_all(&mut self) {
        let Some(dev) = self.dev.as_mut() else {
            return;
        };
        self.pressed.retain(|&code| dev.emit(code, 0).is_err());
    }
}
