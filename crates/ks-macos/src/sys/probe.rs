//! Примитивы проверочного режима `--selftest`: нажатия «как пользователь», поле собственного
//! документа проверки через Accessibility, приложение в фокусе. Сам сценарий ведёт `ks-app`.
//! Чужие поля не читаются и не меняются: [`own_field`] отдаёт поле, только если окно в фокусе —
//! документ с заданным именем файла, а поле в фокусе принадлежит именно этому окну.

use std::ptr;

use core_foundation::base::{CFEqual, CFRange, CFType, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use objc2_app_kit::NSWorkspace;

use super::ffi::*;
use super::post::CgPoster;
use crate::exec::Poster;
use crate::tap::MARK_SELFTEST;

pub use super::system::{current_layout, on_main, select_layout};

/// Нажатие, которое перехват обрабатывает как нажатие пользователя.
pub fn user_key(keycode: u16, down: bool, flags: u64) {
    CgPoster {
        mark: MARK_SELFTEST,
    }
    .key(keycode, down, flags);
}

/// Bundle id и pid приложения в фокусе.
pub fn frontmost_app() -> Option<(String, i32)> {
    on_main(|_mtm| {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let id = app.bundleIdentifier()?.to_string();
        Some((id, app.processIdentifier()))
    })
}

fn attribute(element: &CFType, name: &'static str) -> Option<CFType> {
    let attr = CFString::from_static_string(name);
    let mut value: CFTypeRef = ptr::null();
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            attr.as_concrete_TypeRef(),
            &mut value,
        )
    };
    (status == 0 && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
}

fn string_attribute(element: &CFType, name: &'static str) -> Option<String> {
    attribute(element, name)?
        .downcast_into::<CFString>()
        .map(|s| s.to_string())
}

/// Текстовое поле собственного документа проверки.
pub struct Field(CFType);

/// Поле в фокусе, если оно из окна документа `file_name` (по `AXDocument` или заголовку).
/// Иначе — отказ с объяснением; чужое поле не возвращается.
pub fn own_field(pid: i32, file_name: &str) -> Result<Field, String> {
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return Err("нет доступа к приложению через Accessibility".to_owned());
    }
    let app = unsafe { CFType::wrap_under_create_rule(app) };
    let refuse = || {
        format!("в фокусе не документ {file_name} — чужой текст не трогаю, проверка остановлена")
    };
    let window = attribute(&app, "AXFocusedWindow").ok_or_else(refuse)?;
    let stem = file_name.split('.').next().unwrap_or(file_name);
    let by_document =
        string_attribute(&window, "AXDocument").is_some_and(|d| d.ends_with(file_name));
    let by_title = string_attribute(&window, "AXTitle").is_some_and(|t| t.starts_with(stem));
    if !(by_document || by_title) {
        return Err(refuse());
    }
    let field = attribute(&app, "AXFocusedUIElement").ok_or_else(refuse)?;
    let same_window = attribute(&field, "AXWindow")
        .is_some_and(|w| unsafe { CFEqual(w.as_CFTypeRef(), window.as_CFTypeRef()) } != 0);
    if !same_window {
        return Err(refuse());
    }
    Ok(Field(field))
}

impl Field {
    /// Текст поля (`AXValue`).
    pub fn text(&self) -> Option<String> {
        string_attribute(&self.0, "AXValue")
    }

    /// Заменяет текст поля и ставит каретку в конец; `false` — поле не дало себя изменить.
    pub fn set_text(&self, text: &str) -> bool {
        let value = CFString::new(text);
        if !self.set("AXValue", value.as_CFTypeRef()) {
            return false;
        }
        let end = CFRange::init(text.encode_utf16().count() as isize, 0);
        let range = unsafe { AXValueCreate(4, (&end as *const CFRange).cast()) };
        if range.is_null() {
            return false;
        }
        let range = unsafe { CFType::wrap_under_create_rule(range) };
        self.set("AXSelectedTextRange", range.as_CFTypeRef())
    }

    fn set(&self, name: &'static str, value: CFTypeRef) -> bool {
        let attr = CFString::from_static_string(name);
        let status = unsafe {
            AXUIElementSetAttributeValue(self.0.as_CFTypeRef(), attr.as_concrete_TypeRef(), value)
        };
        status == 0
    }
}
