//! Объявления функций ОС, которых нет в `core-graphics`/`core-foundation`.

use std::ffi::c_void;

use core_foundation::array::CFArrayRef;
use core_foundation::base::CFTypeRef;
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::CFStringRef;

pub type TISInputSourceRef = *const c_void;
pub type AXUIElementRef = *const c_void;
pub type NotificationCallback =
    extern "C" fn(*mut c_void, *mut c_void, CFStringRef, *const c_void, CFDictionaryRef);

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    pub static kTISPropertyInputSourceID: CFStringRef;
    pub static kTISPropertyLocalizedName: CFStringRef;
    pub static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    pub static kTISPropertyInputSourceLanguages: CFStringRef;
    pub static kTISPropertyInputSourceType: CFStringRef;
    pub static kTISTypeKeyboardLayout: CFStringRef;
    pub static kTISNotifySelectedKeyboardInputSourceChanged: CFStringRef;
    pub fn TISCreateInputSourceList(properties: CFDictionaryRef, include_all: u8) -> CFArrayRef;
    pub fn TISCopyCurrentKeyboardInputSource() -> TISInputSourceRef;
    pub fn TISGetInputSourceProperty(source: TISInputSourceRef, key: CFStringRef) -> *const c_void;
    pub fn TISSelectInputSource(source: TISInputSourceRef) -> i32;
    pub fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: u32,
        dead_key_state: *mut u32,
        max_string_length: usize,
        actual_string_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
    pub fn LMGetKbdType() -> u8;
    pub fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    pub fn AXIsProcessTrusted() -> u8;
    pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    pub fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    /// `kAXValueTypeCFRange` = 4.
    pub fn AXValueCreate(the_type: u32, value_ptr: *const c_void) -> CFTypeRef;
    pub fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub fn CGPreflightListenEventAccess() -> bool;
    pub fn CGPreflightPostEventAccess() -> bool;
    pub fn CGRequestListenEventAccess() -> bool;
    pub fn CGRequestPostEventAccess() -> bool;
    pub fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    pub fn CGEventTapIsEnabled(tap: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
    pub fn CFNotificationCenterAddObserver(
        center: *mut c_void,
        observer: *const c_void,
        callback: NotificationCallback,
        name: CFStringRef,
        object: *const c_void,
        suspension_behavior: isize,
    );
}
