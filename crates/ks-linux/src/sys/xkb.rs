//! Таблицы клавиш по именам раскладок XKB через libxkbcommon, загружаемую при запуске
//! (`xkbcommon-dl`): библиотека при сборке не нужна. Код XKB = код evdev + 8.
#![allow(unsafe_code)] // FFI libxkbcommon: единственное место unsafe в крейте

use std::ffi::CString;

use ks_core::{LayoutMap, Level, PhysicalKey};
use xkbcommon_dl::{xkb_context_flags, xkb_keymap_compile_flags, xkb_rule_names, xkbcommon_option};

use crate::keymap::KEYS;
use crate::layouts::LayoutSpec;

/// Таблица раскладки; `None` — libxkbcommon не загрузилась или раскладка не собирается.
pub fn layout_map(spec: &LayoutSpec) -> Option<LayoutMap> {
    let xkb = xkbcommon_option()?;
    if spec.layout.is_empty() {
        return None;
    }
    let rules = CString::new("evdev").ok()?;
    let model = CString::new("pc105").ok()?;
    let layout = CString::new(spec.layout.as_str()).ok()?;
    let variant = CString::new(spec.variant.as_str()).ok()?;
    let names = xkb_rule_names {
        rules: rules.as_ptr(),
        model: model.as_ptr(),
        layout: layout.as_ptr(),
        variant: variant.as_ptr(),
        options: std::ptr::null(),
    };
    let mut map = LayoutMap::new(spec.id(), spec.lang(), spec.label());
    // SAFETY: строки `names` живут до конца функции; каждый объект xkbcommon проверен на null
    // и освобождается своим `*_unref` ровно один раз.
    unsafe {
        let ctx = (xkb.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
        if ctx.is_null() {
            return None;
        }
        let km = (xkb.xkb_keymap_new_from_names)(
            ctx,
            &names,
            xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
        );
        if km.is_null() {
            (xkb.xkb_context_unref)(ctx);
            return None;
        }
        let state = (xkb.xkb_state_new)(km);
        if state.is_null() {
            (xkb.xkb_keymap_unref)(km);
            (xkb.xkb_context_unref)(ctx);
            return None;
        }
        let bit = |i: u32| if i < 32 { 1u32 << i } else { 0 };
        let shift = bit((xkb.xkb_keymap_mod_get_index)(km, c"Shift".as_ptr()));
        let lock = bit((xkb.xkb_keymap_mod_get_index)(km, c"Lock".as_ptr()));
        let levels = [
            (Level::Base, 0, 0),
            (Level::Shift, shift, 0),
            (Level::Caps, 0, lock),
            (Level::ShiftCaps, shift, lock),
        ];
        for &(code, key) in KEYS
            .iter()
            .filter(|(_, k)| k.is_printable() || *k == PhysicalKey::Space)
        {
            for (level, depressed, locked) in levels {
                (xkb.xkb_state_update_mask)(state, depressed, 0, locked, 0, 0, 0);
                let cp = (xkb.xkb_state_key_get_utf32)(state, u32::from(code) + 8);
                if let Some(c) = char::from_u32(cp).filter(|c| cp != 0 && !c.is_control()) {
                    map.insert(key, level, c);
                }
            }
        }
        (xkb.xkb_state_unref)(state);
        (xkb.xkb_keymap_unref)(km);
        (xkb.xkb_context_unref)(ctx);
    }
    Some(map)
}
