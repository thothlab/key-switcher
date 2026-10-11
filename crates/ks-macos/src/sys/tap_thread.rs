//! Поток перехвата: активный `CGEventTap` и свой run loop. Колбэк только переводит событие
//! в `RawEvent` → `InputEvent` и отдаёт решение общей `TapLogic`; замены исполняет другой поток.

use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use core_foundation::base::TCFType;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use ks_platform::BackendError;

use super::ffi::{CGEventTapEnable, CGEventTapIsEnabled};
use ks_platform::intercept::{TapControl, TapLogic, TapShared, Verdict};

use crate::tap::{MacEvent, RawEvent, RawKind, clock_ms, input_event};

/// Порт перехвата; нулевой, пока перехват не поднят.
#[derive(Clone, Default)]
pub struct TapPort(Arc<AtomicPtr<c_void>>);

impl TapPort {
    pub fn enabled(&self) -> bool {
        let port = self.0.load(Ordering::Acquire);
        !port.is_null() && unsafe { CGEventTapIsEnabled(port) }
    }
}

impl TapControl for TapPort {
    fn reenable(&self) {
        let port = self.0.load(Ordering::Acquire);
        if !port.is_null() {
            unsafe { CGEventTapEnable(port, true) };
        }
    }
}

fn raw_event(etype: CGEventType, event: &CGEvent) -> RawEvent {
    let kind = match etype {
        CGEventType::KeyDown => RawKind::KeyDown,
        CGEventType::KeyUp => RawKind::KeyUp,
        CGEventType::FlagsChanged => RawKind::FlagsChanged,
        CGEventType::LeftMouseDown | CGEventType::RightMouseDown | CGEventType::OtherMouseDown => {
            RawKind::MouseDown
        }
        CGEventType::TapDisabledByTimeout => RawKind::DisabledByTimeout,
        CGEventType::TapDisabledByUserInput => RawKind::DisabledByUserInput,
        _ => RawKind::Other,
    };
    if matches!(
        kind,
        RawKind::DisabledByTimeout | RawKind::DisabledByUserInput | RawKind::Other
    ) {
        return RawEvent::key(kind, 0, 0);
    }
    let field = |f| event.get_integer_value_field(f);
    RawEvent {
        kind,
        keycode: field(EventField::KEYBOARD_EVENT_KEYCODE) as u16,
        flags: event.get_flags().bits(),
        autorepeat: field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0,
        user_data: field(EventField::EVENT_SOURCE_USER_DATA),
        time_ms: clock_ms(),
    }
}

const GATE_PENDING: u8 = 0;
const GATE_STARTED: u8 = 1;
const GATE_CANCELLED: u8 = 2;
const START_TIMEOUT: Duration = Duration::from_secs(3);

/// Поднимает перехват на отдельном потоке; ждёт, пока tap создан или отказан. По таймауту
/// запуск отменяется: поток, создавший tap позже, его не включает и выходит.
pub fn spawn(
    logic: Arc<TapLogic<MacEvent>>,
    shared: Arc<TapShared<MacEvent>>,
    port: TapPort,
) -> Result<(), BackendError> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let gate = Arc::new(AtomicU8::new(GATE_PENDING));
    let thread_gate = gate.clone();
    std::thread::Builder::new()
        .name("ks-tap".to_owned())
        .spawn(move || run(logic, shared, port, ready_tx, &thread_gate))
        .map_err(|e| BackendError::Os(e.to_string()))?;
    match ready_rx.recv_timeout(START_TIMEOUT) {
        Ok(result) => result,
        Err(_) => {
            let cancelled = gate
                .compare_exchange(
                    GATE_PENDING,
                    GATE_CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok();
            if cancelled {
                Err(BackendError::Os(
                    "перехват не поднялся за 3 с — запуск отменён".to_owned(),
                ))
            } else {
                // Поток успел занять ворота: его ответ уже в пути.
                ready_rx.recv().unwrap_or_else(|_| {
                    Err(BackendError::Os("поток перехвата завершился".to_owned()))
                })
            }
        }
    }
}

fn run(
    logic: Arc<TapLogic<MacEvent>>,
    shared: Arc<TapShared<MacEvent>>,
    port: TapPort,
    ready: mpsc::Sender<Result<(), BackendError>>,
    gate: &AtomicU8,
) {
    let control = port.clone();
    let mask = vec![
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
        CGEventType::LeftMouseDown,
        CGEventType::RightMouseDown,
        CGEventType::OtherMouseDown,
    ];
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        mask,
        move |_proxy, etype, event| match logic
            .handle(&input_event(&raw_event(etype, event)), &control)
        {
            Verdict::Pass => CallbackResult::Keep,
            Verdict::Swallow => CallbackResult::Drop,
        },
    );
    let Ok(tap) = tap else {
        // CGEventTapCreate отдаёт NULL без Input Monitoring/Accessibility.
        let _ = ready.send(Err(BackendError::NoPermission));
        return;
    };
    let Ok(source) = tap.mach_port().create_runloop_source(0) else {
        let _ = ready.send(Err(BackendError::Os("нет источника run loop".to_owned())));
        return;
    };
    if gate
        .compare_exchange(
            GATE_PENDING,
            GATE_STARTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        // Запуск уже отменён по таймауту: tap уничтожается, не будучи включённым.
        return;
    }
    port.0.store(
        tap.mach_port().as_concrete_TypeRef() as *mut c_void,
        Ordering::Release,
    );
    CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    shared.capturing.store(true, Ordering::Release);
    let _ = ready.send(Ok(()));
    CFRunLoop::run_current();
    shared.capturing.store(false, Ordering::Release);
    port.0.store(ptr::null_mut(), Ordering::Release);
}
