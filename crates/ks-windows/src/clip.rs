//! Буфер обмена для конвертации выделения поверх системного доступа ([`ClipboardOs`]) — без
//! системных вызовов, проверяется тестом на любой ОС.
//!
//! Снимок буфера снимается ещё в `usable_in`, до Ctrl+C: не открылся буфер (держит другое
//! приложение) — конвертация отменяется, и ни копирование, ни запись, ни восстановление не
//! трогают то, что скопировал пользователь. Свои записи (результат и восстановление) помечены
//! форматами, которые исключают их из истории Win+V, облачного буфера и мониторов буфера.
//! Содержимое не логируется.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ks_core::AppId;
use ks_platform::exec::{Clipboard, ClipboardSnapshot};

use crate::apps;

/// `CF_UNICODETEXT`.
pub const CF_UNICODETEXT: &str = "13";

/// Зарегистрированные форматы-пометки своих записей (имя → данные, DWORD 0).
pub const EXCLUDE_MARKS: [&str; 3] = [
    "ExcludeClipboardContentFromMonitorProcessing",
    "CanIncludeInClipboardHistory",
    "CanUploadToCloudClipboard",
];

/// Формат: число — идентификатор `CF_*`/зарегистрированный в этой сессии, иначе — имя для
/// `RegisterClipboardFormatW`.
pub type Item = (String, Vec<u8>);

/// Системный буфер обмена.
pub trait ClipboardOs: Send + Sync {
    /// Все форматы с данными; `None` — буфер не открылся.
    fn read_all(&self) -> Option<Vec<Item>>;
    /// Данные одного формата.
    fn read(&self, format: &str) -> Option<Vec<u8>>;
    /// Очистить буфер и записать форматы; `false` — не открылся.
    fn replace(&self, items: &[Item]) -> bool;
    /// `GetClipboardSequenceNumber`.
    fn sequence(&self) -> i64;
}

fn marked(mut items: Vec<Item>) -> Vec<Item> {
    items.retain(|(f, _)| !EXCLUDE_MARKS.contains(&f.as_str()));
    items.extend(EXCLUDE_MARKS.iter().map(|m| ((*m).to_owned(), vec![0; 4])));
    items
}

pub struct GuardedClipboard<O> {
    os: O,
    saved: Mutex<Option<ClipboardSnapshot>>,
    /// Снимок, отданный исполнителю, настоящий: восстанавливать можно.
    valid: AtomicBool,
}

impl<O: ClipboardOs> GuardedClipboard<O> {
    pub fn new(os: O) -> GuardedClipboard<O> {
        GuardedClipboard {
            os,
            saved: Mutex::new(None),
            valid: AtomicBool::new(false),
        }
    }

    pub fn os(&self) -> &O {
        &self.os
    }

    fn saved(&self) -> std::sync::MutexGuard<'_, Option<ClipboardSnapshot>> {
        self.saved.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<O: ClipboardOs> Clipboard for GuardedClipboard<O> {
    /// Вставка работает в приложении **и** снимок буфера снят сейчас.
    fn usable_in(&self, app: &AppId) -> bool {
        if !apps::paste_usable(app) {
            return false;
        }
        match self.os.read_all() {
            Some(items) => {
                *self.saved() = Some(ClipboardSnapshot(vec![items]));
                true
            }
            None => {
                *self.saved() = None;
                log::warn!("буфер обмена занят другим приложением: выделение не конвертируется");
                false
            }
        }
    }

    fn change_count(&self) -> i64 {
        self.os.sequence()
    }

    fn snapshot(&self) -> ClipboardSnapshot {
        let taken = self.saved().take();
        let snapshot = taken.or_else(|| {
            self.os
                .read_all()
                .map(|items| ClipboardSnapshot(vec![items]))
        });
        self.valid.store(snapshot.is_some(), Ordering::Release);
        snapshot.unwrap_or_default()
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) {
        if !self.valid.swap(false, Ordering::AcqRel) {
            log::warn!("снимка буфера обмена нет: восстановление пропущено");
            return;
        }
        let items: Vec<Item> = snapshot.0.iter().flatten().cloned().collect();
        if !self.os.replace(&marked(items)) {
            log::warn!("буфер обмена не восстановлен: занят другим приложением");
        }
    }

    fn text(&self) -> Option<String> {
        let bytes = self.os.read(CF_UNICODETEXT)?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    fn set_text(&self, text: &str) {
        let bytes: Vec<u8> = text
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        if !self
            .os
            .replace(&marked(vec![(CF_UNICODETEXT.to_owned(), bytes)]))
        {
            log::warn!("результат не записан в буфер обмена: занят другим приложением");
        }
    }
}
