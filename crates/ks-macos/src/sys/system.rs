//! TIS/TSM и права. Каждая функция TIS требует [`MainThreadMarker`]: вызов не с главного
//! потока не скомпилируется, а [`main_thread`] в debug-сборке паникует, если маркер просят
//! не там. С других потоков — через [`on_main`] (главная очередь GCD).

use std::ffi::c_void;
use std::ptr;

use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::data::{CFDataGetBytePtr, CFDataRef};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use ks_core::{LayoutId, LayoutMap, LayoutSet, Level};
pub use objc2::MainThreadMarker;

use super::ffi::*;
use crate::layouts::{build_layout, lang_of, printable_char, uc_modifier_state};

/// Маркер главного потока. Не на главном: в debug — паника, в release — ошибка в лог и `None`.
pub fn main_thread(what: &str) -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        if cfg!(debug_assertions) {
            panic!("TIS/TSM не с главного потока: {what}");
        }
        log::error!("TIS/TSM не с главного потока: {what}; вызов пропущен");
    }
    mtm
}

/// Выполняет `f` на главном потоке: сразу, если мы на нём, иначе синхронно через главную очередь.
pub fn on_main<R: Send>(f: impl FnOnce(MainThreadMarker) -> R + Send) -> R {
    dispatch2::run_on_main(f)
}

/// `LMGetKbdType()` этой машины.
pub fn machine_kbd_type(_mtm: MainThreadMarker) -> u32 {
    u32::from(unsafe { LMGetKbdType() })
}

pub fn secure_input(_mtm: MainThreadMarker) -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
}

fn cf_string(r: CFStringRef) -> CFString {
    unsafe { CFString::wrap_under_get_rule(r) }
}

fn string_prop(src: TISInputSourceRef, key: CFStringRef) -> Option<String> {
    let value = unsafe { TISGetInputSourceProperty(src, key) } as CFStringRef;
    (!value.is_null()).then(|| cf_string(value).to_string())
}

fn languages(src: TISInputSourceRef) -> Vec<String> {
    let arr = unsafe { TISGetInputSourceProperty(src, kTISPropertyInputSourceLanguages) };
    if arr.is_null() {
        return Vec::new();
    }
    let arr = arr as CFArrayRef;
    (0..unsafe { CFArrayGetCount(arr) })
        .map(|i| cf_string(unsafe { CFArrayGetValueAtIndex(arr, i) } as CFStringRef).to_string())
        .collect()
}

/// Источники ввода типа «раскладка»: включённые или (`all`) все установленные.
fn with_sources<R>(all: bool, mut f: impl FnMut(TISInputSourceRef) -> Option<R>) -> Vec<R> {
    let key = cf_string(unsafe { kTISPropertyInputSourceType });
    let value = cf_string(unsafe { kTISTypeKeyboardLayout });
    let filter = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let list = unsafe { TISCreateInputSourceList(filter.as_concrete_TypeRef(), u8::from(all)) };
    if list.is_null() {
        return Vec::new();
    }
    let out = (0..unsafe { CFArrayGetCount(list) })
        .filter_map(|i| f(unsafe { CFArrayGetValueAtIndex(list, i) }))
        .collect();
    unsafe { CFRelease(list as *const c_void) };
    out
}

fn layout_of(src: TISInputSourceRef, kbd_type: u32) -> Option<LayoutMap> {
    let id = string_prop(src, unsafe { kTISPropertyInputSourceID })?;
    let name = string_prop(src, unsafe { kTISPropertyLocalizedName }).unwrap_or_else(|| id.clone());
    let data = unsafe { TISGetInputSourceProperty(src, kTISPropertyUnicodeKeyLayoutData) };
    if data.is_null() {
        return None;
    }
    let bytes = unsafe { CFDataGetBytePtr(data as CFDataRef) } as *const c_void;
    let translate = |code: u16, level: Level| {
        let (mut dead, mut len, mut buf) = (0u32, 0usize, [0u16; 8]);
        // kUCKeyActionDown = 0, kUCKeyTranslateNoDeadKeysMask = 1
        let status = unsafe {
            UCKeyTranslate(
                bytes,
                code,
                0,
                uc_modifier_state(level),
                kbd_type,
                1,
                &mut dead,
                buf.len(),
                &mut len,
                buf.as_mut_ptr(),
            )
        };
        (status == 0 && len > 0)
            .then(|| printable_char(&buf[..len]))
            .flatten()
    };
    Some(build_layout(
        LayoutId(id),
        lang_of(&languages(src)),
        &name,
        translate,
    ))
}

/// Таблицы раскладок, включённых в системе.
pub fn enabled_layouts(_mtm: MainThreadMarker, kbd_type: u32) -> LayoutSet {
    LayoutSet(with_sources(false, |src| layout_of(src, kbd_type)))
}

/// Таблица установленной раскладки по id, даже если она не включена (для теста фикстур).
pub fn layout_by_id(_mtm: MainThreadMarker, id: &str, kbd_type: u32) -> Option<LayoutMap> {
    with_sources(true, |src| {
        (string_prop(src, unsafe { kTISPropertyInputSourceID }).as_deref() == Some(id))
            .then(|| layout_of(src, kbd_type))
            .flatten()
    })
    .pop()
}

pub fn current_layout(_mtm: MainThreadMarker) -> Option<LayoutId> {
    let src = unsafe { TISCopyCurrentKeyboardInputSource() };
    if src.is_null() {
        return None;
    }
    let id = string_prop(src, unsafe { kTISPropertyInputSourceID });
    unsafe { CFRelease(src) };
    id.map(LayoutId)
}

/// Включает раскладку и проверяет, что она стала текущей.
pub fn select_layout(mtm: MainThreadMarker, id: &LayoutId) -> bool {
    let selected = with_sources(false, |src| {
        (string_prop(src, unsafe { kTISPropertyInputSourceID }).as_deref() == Some(&id.0))
            .then(|| unsafe { TISSelectInputSource(src) } == 0)
    });
    selected.first().copied().unwrap_or(false) && current_layout(mtm).as_ref() == Some(id)
}

/// Подписка на смену текущей раскладки (`enabled` — на смену набора включённых раскладок);
/// колбэк приходит на главный поток.
///
/// # Safety
/// `observer` передаётся в `callback` как есть и должен жить, пока жив процесс.
pub unsafe fn observe_layout_changes(
    _mtm: MainThreadMarker,
    observer: *const c_void,
    callback: NotificationCallback,
    enabled: bool,
) {
    unsafe {
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDistributedCenter(),
            observer,
            callback,
            if enabled {
                kTISNotifyEnabledKeyboardInputSourcesChanged
            } else {
                kTISNotifySelectedKeyboardInputSourceChanged
            },
            ptr::null(),
            4, // CFNotificationSuspensionBehaviorDeliverImmediately
        );
    }
}

/// Право Input Monitoring (`ListenEvent`).
pub fn listen_allowed() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

/// Право на эмуляцию ввода: Accessibility и `PostEvent` вместе.
pub fn post_allowed() -> bool {
    unsafe { AXIsProcessTrusted() != 0 && CGPreflightPostEventAccess() }
}

pub fn request_listen() {
    unsafe { CGRequestListenEventAccess() };
}

pub fn request_post() {
    unsafe { CGRequestPostEventAccess() };
}
