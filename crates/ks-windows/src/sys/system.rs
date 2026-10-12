//! Вызовы Win32 для бэкенда: окно на переднем плане и его раскладка, смена раскладки,
//! таблицы из `ToUnicodeEx`, имя exe, `SendInput`, буфер обмена, реестр.
//! Что именно отправлять и как строить таблицы, решают модули без системных вызовов.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use ks_core::{LayoutSet, Level};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, GetKeyboardLayoutList, HKL, INPUT, INPUT_0,
    INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, SendInput, ToUnicodeEx, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW,
    WM_INPUTLANGCHANGEREQUEST,
};
use windows::core::{PCWSTR, PWSTR};

use crate::apps;
use crate::clip::{ClipboardOs, Item};
use crate::keycode::{self, EXT};
use crate::layouts::{self, TOUNICODE_NO_STATE_CHANGE};
use crate::send::{Sink, Stroke};
use crate::tap::{MARK_OWN, ModState};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Окно на переднем плане: (окно, его поток, его процесс).
pub fn foreground() -> Option<(isize, u32, u32)> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (tid != 0).then_some((hwnd.0 as isize, tid, pid))
}

/// Раскладка окна на переднем плане: на Windows она своя у каждого потока
/// (`analysis/platforms.md` §2.4). Для консолей conhost ответ может быть неверным.
pub fn current_hkl() -> Option<isize> {
    let (_, tid, _) = foreground()?;
    let hkl = unsafe { GetKeyboardLayout(tid) };
    (!hkl.0.is_null()).then_some(hkl.0 as isize)
}

pub fn layout_list() -> Vec<isize> {
    let n = unsafe { GetKeyboardLayoutList(None) };
    let Ok(len) = usize::try_from(n) else {
        return Vec::new();
    };
    let mut list = vec![HKL(std::ptr::null_mut()); len];
    let got = unsafe { GetKeyboardLayoutList(Some(&mut list)) };
    list.truncate(usize::try_from(got).unwrap_or(0));
    list.iter().map(|h| h.0 as isize).collect()
}

fn translate(hkl: isize, vk: u16, scan: u16, level: Level) -> Option<char> {
    let layout = Some(HKL(hkl as *mut c_void));
    let mut buf = [0u16; 8];
    let n = unsafe {
        ToUnicodeEx(
            u32::from(vk),
            u32::from(scan & 0xFF),
            &layouts::key_state(level),
            &mut buf,
            TOUNICODE_NO_STATE_CHANGE,
            layout,
        )
    };
    if n < 0 {
        // Мёртвая клавиша. До Windows 10 1607 флаг не действует, и она осталась бы в буфере
        // ядра — сбрасываем пробелом.
        let mut flush = [0u16; 8];
        unsafe {
            ToUnicodeEx(
                0x20,
                0x39,
                &layouts::key_state(Level::Base),
                &mut flush,
                TOUNICODE_NO_STATE_CHANGE,
                layout,
            )
        };
    }
    layouts::printable_char(&buf, n)
}

fn reg_string(root: HKEY, sub: &str, value: &str) -> Option<String> {
    let (sub, value) = (wide(sub), wide(value));
    let mut size = 0u32;
    let rc = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let rc = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Название раскладки из реестра (`Layout Text`); для вариантов — `HKL`.
fn layout_name(hkl: isize) -> String {
    layouts::klid_of(hkl)
        .and_then(|klid| {
            reg_string(
                HKEY_LOCAL_MACHINE,
                &format!(r"SYSTEM\CurrentControlSet\Control\Keyboard Layouts\{klid}"),
                "Layout Text",
            )
        })
        .unwrap_or_else(|| layouts::layout_id(hkl).0)
}

/// Таблицы включённых раскладок через `ToUnicodeEx`.
pub fn build_layouts(list: &[isize]) -> LayoutSet {
    LayoutSet(
        list.iter()
            .map(|&hkl| {
                layouts::build_layout(
                    layouts::layout_id(hkl),
                    layouts::lang_of(hkl),
                    &layout_name(hkl),
                    |vk, scan, level| translate(hkl, vk, scan, level),
                )
            })
            .collect(),
    )
}

/// Просит окно с фокусом сменить раскладку (`WM_INPUTLANGCHANGEREQUEST`) и ждёт, пока поток
/// окна её применит, не дольше `timeout`. Приложение вправе отказаться; окна администратора
/// сообщение не получают (UIPI).
pub fn select_layout(hkl: isize, timeout: Duration) -> bool {
    let Some((hwnd, tid, _)) = foreground() else {
        return false;
    };
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<GUITHREADINFO>() as u32;
    let focus = unsafe { GetGUIThreadInfo(tid, &mut info) }.is_ok() && !info.hwndFocus.0.is_null();
    let target = if focus {
        info.hwndFocus
    } else {
        HWND(hwnd as *mut c_void)
    };
    let posted = unsafe {
        PostMessageW(
            Some(target),
            WM_INPUTLANGCHANGEREQUEST,
            WPARAM(0),
            LPARAM(hkl),
        )
    };
    if posted.is_err() {
        return false;
    }
    let start = Instant::now();
    loop {
        if unsafe { GetKeyboardLayout(tid) }.0 as isize as u32 == hkl as u32 {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Полный путь exe процесса.
pub fn exe_path(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(process) };
    ok.then(|| String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]))
}

/// Модификаторы по `GetAsyncKeyState` — только для пересинхронизации вне колбэка хука: в колбэке
/// асинхронное состояние ещё не учло обрабатываемое событие.
pub fn async_mods() -> ModState {
    const SIDES: [(u16, u8); 8] = [
        (keycode::VK_LSHIFT, ModState::LSHIFT),
        (keycode::VK_RSHIFT, ModState::RSHIFT),
        (keycode::VK_LCONTROL, ModState::LCTRL),
        (keycode::VK_RCONTROL, ModState::RCTRL),
        (keycode::VK_LMENU, ModState::LALT),
        (keycode::VK_RMENU, ModState::RALT),
        (keycode::VK_LWIN, ModState::LWIN),
        (keycode::VK_RWIN, ModState::RWIN),
    ];
    ModState(
        SIDES
            .iter()
            .filter(|&&(vk, _)| unsafe { GetAsyncKeyState(i32::from(vk)) } < 0)
            .fold(0, |acc, &(_, bit)| acc | bit),
    )
}

pub fn caps_on() -> bool {
    let state = unsafe { GetKeyState(i32::from(keycode::VK_CAPITAL)) };
    state & 1 != 0
}

fn input(stroke: &Stroke) -> INPUT {
    let (vk, scan, flags) = match *stroke {
        Stroke::Key { vk, scan, up } => {
            let ext = if scan & EXT != 0 {
                KEYEVENTF_EXTENDEDKEY.0
            } else {
                0
            };
            (
                vk,
                scan & 0xFF,
                ext | if up { KEYEVENTF_KEYUP.0 } else { 0 },
            )
        }
        Stroke::Unicode { unit, up } => (
            0,
            unit,
            KEYEVENTF_UNICODE.0 | if up { KEYEVENTF_KEYUP.0 } else { 0 },
        ),
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: MARK_OWN,
            },
        },
    }
}

/// `SendInput` со своей пометкой в `dwExtraInfo`: хук отдаёт эти события назад с `own`.
pub struct SendInputSink;

impl Sink for SendInputSink {
    fn send(&self, strokes: &[Stroke]) {
        if strokes.is_empty() {
            return;
        }
        let inputs: Vec<INPUT> = strokes.iter().map(input).collect();
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if (sent as usize) < inputs.len() {
            // UIPI (окно администратора) отказ не сообщает; здесь — только видимые отказы.
            log::warn!("SendInput принял {sent} из {} событий", inputs.len());
        }
    }

    fn pause_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Форматы, данные которых — не `HGLOBAL` (GDI-объекты, private-форматы): не копируются.
fn copyable(format: u32) -> bool {
    !matches!(
        format,
        2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E | 0x200..=0x3FF
    )
}

/// Буфер обмена открыт этим потоком; закрывается при выходе из области.
struct Opened;

impl Drop for Opened {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn read_global(handle: HANDLE) -> Option<Vec<u8>> {
    let global = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(global) };
    let ptr = unsafe { GlobalLock(global) };
    if ptr.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();
    let _ = unsafe { GlobalUnlock(global) };
    Some(bytes)
}

fn put_global(format: u32, bytes: &[u8]) -> bool {
    let Ok(global) = (unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }) else {
        return false;
    };
    let ptr = unsafe { GlobalLock(global) };
    if ptr.is_null() {
        let _ = unsafe { GlobalFree(Some(global)) };
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len()) };
    let _ = unsafe { GlobalUnlock(global) };
    // После успешного SetClipboardData память принадлежит системе.
    if unsafe { SetClipboardData(format, Some(HANDLE(global.0))) }.is_err() {
        let _ = unsafe { GlobalFree(Some(global)) };
        return false;
    }
    true
}

/// Идентификатор формата: число — как есть, имя — `RegisterClipboardFormatW`.
fn format_id(format: &str) -> Option<u32> {
    if let Ok(id) = format.parse::<u32>() {
        return Some(id);
    }
    let name = wide(format);
    let id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    (id != 0).then_some(id)
}

/// Системный буфер обмена. Владелец — невидимое окно потока хука (без владельца
/// `SetClipboardData` после `EmptyClipboard` не работает).
pub struct WinClipOs {
    pub owner: Arc<AtomicIsize>,
}

impl WinClipOs {
    fn open(&self) -> Option<Opened> {
        let owner = HWND(self.owner.load(Ordering::Acquire) as *mut c_void);
        for _ in 0..10 {
            if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
                return Some(Opened);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl ClipboardOs for WinClipOs {
    fn read_all(&self) -> Option<Vec<Item>> {
        let _opened = self.open()?;
        let mut items = Vec::new();
        let mut format = 0;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if !copyable(format) {
                continue;
            }
            if let Ok(handle) = unsafe { GetClipboardData(format) }
                && let Some(bytes) = read_global(handle)
            {
                items.push((format.to_string(), bytes));
            }
        }
        Some(items)
    }

    fn read(&self, format: &str) -> Option<Vec<u8>> {
        let id = format_id(format)?;
        let _opened = self.open()?;
        let handle = unsafe { GetClipboardData(id) }.ok()?;
        read_global(handle)
    }

    fn replace(&self, items: &[Item]) -> bool {
        let ids: Vec<Option<u32>> = items.iter().map(|(f, _)| format_id(f)).collect();
        let Some(_opened) = self.open() else {
            return false;
        };
        let _ = unsafe { EmptyClipboard() };
        for (id, (_, bytes)) in ids.into_iter().zip(items) {
            if let Some(id) = id {
                put_global(id, bytes);
            }
        }
        true
    }

    fn sequence(&self) -> i64 {
        i64::from(unsafe { GetClipboardSequenceNumber() })
    }
}

pub fn autostart() -> bool {
    reg_string(HKEY_CURRENT_USER, apps::RUN_KEY, apps::RUN_VALUE).is_some()
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    let (key, name) = (wide(apps::RUN_KEY), wide(apps::RUN_VALUE));
    let rc: WIN32_ERROR = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = wide(&apps::run_command(&exe.to_string_lossy()));
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                (command.len() * 2) as u32,
            )
        }
    } else {
        match unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            rc => rc,
        }
    };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("реестр, ключ Run: код {}", rc.0))
    }
}
