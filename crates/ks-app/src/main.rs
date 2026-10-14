// Windows: агент без консольного окна. Командам с выводом консоль родителя подключается сама
// (`attach_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]
//! `keyswitcher` — агент автопереключения раскладки RU↔EN.
//! Без аргументов — агент в строке меню; `--status`, `--selftest`, `--settings [--section <имя>]`.

use std::process::ExitCode;
use std::sync::Arc;

use ks_app::paths;
use ks_app::status::{Source, report};
use ks_platform::Backend;

#[cfg(target_os = "macos")]
fn backend() -> Arc<dyn Backend> {
    Arc::new(ks_macos::MacBackend::new())
}

#[cfg(windows)]
fn backend() -> Arc<dyn Backend> {
    Arc::new(ks_windows::WindowsBackend::new())
}

#[cfg(target_os = "linux")]
fn backend() -> Arc<dyn Backend> {
    Arc::new(ks_linux::LinuxBackend::new())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
compile_error!("KeySwitcher собирается только под macOS, Windows и Linux");

fn usage() -> String {
    let sections: Vec<&str> = ks_settings::Section::ALL.iter().map(|s| s.name()).collect();
    format!(
        "использование: keyswitcher [--status | --selftest | --settings [--section <{}>] | --version]",
        sections.join("|")
    )
}

/// Windows: у процесса GUI-подсистемы нет своей консоли; вывод `--status`, `--version` и ошибок
/// идёт в консоль, из которой команду запустили (если она есть).
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: вызов без указателей; ошибка (консоли у родителя нет) ничего не ломает.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if !args.is_empty() {
        attach_console();
    }
    match args.first().map(String::as_str) {
        None => match ks_app::agent::run(backend()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("KeySwitcher: {e:#}");
                ExitCode::FAILURE
            }
        },
        Some("--status") => status(),
        // служебная: агент спрашивает права новым процессом (выданное право TCC видно только ему)
        Some("--permissions") => permissions(),
        Some("--selftest") => selftest(),
        Some("--settings") => settings(&args[1..]),
        Some("--version") => {
            println!("KeySwitcher {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{}", usage());
            ExitCode::from(2)
        }
    }
}

/// «granted» — у этого процесса есть все права, иначе «missing».
fn permissions() -> ExitCode {
    let all = backend().permissions().iter().all(|p| p.granted);
    println!("{}", if all { "granted" } else { "missing" });
    ExitCode::SUCCESS
}

/// Состояние из процесса агента; агент не запущен — то, что видно этой команде, с пометкой.
fn status() -> ExitCode {
    #[cfg(unix)]
    if let Some(socket) = paths::agent_socket() {
        let mut answered = false;
        let asked = ks_app::ipc::ask(&socket, "status", |line| {
            answered = true;
            println!("{line}");
        });
        if asked.is_ok() && answered {
            return ExitCode::SUCCESS;
        }
    }
    print!("{}", report(&*backend(), None, Source::Local));
    ExitCode::SUCCESS
}

#[cfg(target_os = "macos")]
fn selftest() -> ExitCode {
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    let Some(socket) = paths::agent_socket() else {
        eprintln!("не найден каталог настроек");
        return ExitCode::from(2);
    };
    if UnixStream::connect(&socket).is_err() {
        // Права TCC привязаны к KeySwitcher.app, а у процесса из Терминала — права Терминала,
        // поэтому проверку исполняет агент, запущенный через LaunchServices.
        let Some(app) = bundle_path() else {
            eprintln!("агент не запущен: запустите KeySwitcher.app и повторите --selftest");
            return ExitCode::from(2);
        };
        eprintln!("агент не запущен — запускаю {}", app.display());
        let _ = std::process::Command::new("open").arg(&app).status();
        let start = Instant::now();
        while UnixStream::connect(&socket).is_err() {
            if start.elapsed() > Duration::from_secs(15) {
                eprintln!("агент не ответил за 15 с");
                return ExitCode::from(2);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let mut result = None;
    let asked = ks_app::ipc::ask(&socket, "selftest", |line| {
        match line.strip_prefix("RESULT: ") {
            Some(r) => result = Some(r == "PASS"),
            None => println!("{line}"),
        }
    });
    if let Err(e) = asked {
        eprintln!("агент не ответил: {e}");
    }
    match result {
        Some(true) => ExitCode::SUCCESS,
        Some(false) => ExitCode::FAILURE,
        None => ExitCode::from(2),
    }
}

#[cfg(target_os = "macos")]
fn bundle_path() -> Option<std::path::PathBuf> {
    ks_macos::bundle::current_bundle()
}

#[cfg(not(target_os = "macos"))]
fn selftest() -> ExitCode {
    eprintln!("--selftest пока есть только на macOS");
    ExitCode::from(2)
}

fn settings(args: &[String]) -> ExitCode {
    use ks_settings::{Host, Section};
    let section = match args {
        [] => None,
        [flag, name] if flag == "--section" => match name.parse::<Section>() {
            Ok(section) => Some(section),
            Err(e) => {
                eprintln!("{e}\n{}", usage());
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("{}", usage());
            return ExitCode::from(2);
        }
    };
    let Some(path) = paths::config_file() else {
        eprintln!("не найден каталог настроек");
        return ExitCode::FAILURE;
    };
    let host = Host {
        backend: Some(backend()),
        builtin: ks_core::Lexicon::embedded().builtin_apps().to_vec(),
        restart: Some(Arc::new(restart_agent)),
    };
    match ks_settings::run_with(section, &path, host) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KeySwitcher: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// «Перезапустить KeySwitcher» из окна: работающий агент перезапускает себя сам (команда сокета
/// `restart`), агента нет — KeySwitcher.app запускается заново. Возвращает, когда по сокету
/// отвечает новый агент (другой pid), или ошибку по истечении `RESTART_LIMIT`.
#[cfg(target_os = "macos")]
fn restart_agent() -> Result<(), String> {
    const RESTART_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);
    let socket = paths::agent_socket().ok_or("не найден каталог настроек")?;
    let old = agent_pid(&socket);
    if old.is_some() {
        let mut reply = Vec::new();
        ks_app::ipc::ask(&socket, "restart", |line| reply.push(line.to_owned()))
            .map_err(|e| format!("агент не ответил на перезапуск: {e}"))?;
        if !reply.iter().any(|l| l == "OK") {
            return Err(reply.join(" "));
        }
    } else {
        let app =
            bundle_path().ok_or("KeySwitcher не запущен, а окно открыто не из KeySwitcher.app")?;
        std::process::Command::new("/usr/bin/open")
            .arg(&app)
            .status()
            .map_err(|e| format!("KeySwitcher.app не открыт: {e}"))?;
    }
    let deadline = std::time::Instant::now() + RESTART_LIMIT;
    while std::time::Instant::now() < deadline {
        if let Some(pid) = agent_pid(&socket)
            && Some(pid) != old
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(format!(
        "KeySwitcher не поднялся за {} с — откройте KeySwitcher.app вручную",
        RESTART_LIMIT.as_secs()
    ))
}

#[cfg(not(target_os = "macos"))]
fn restart_agent() -> Result<(), String> {
    Err("перезапуск из окна есть только на macOS".to_owned())
}

/// Pid агента, который отвечает по сокету; `None` — никто не отвечает.
#[cfg(target_os = "macos")]
fn agent_pid(socket: &std::path::Path) -> Option<u32> {
    let mut pid = None;
    ks_app::ipc::ask(socket, "pid", |line| pid = line.trim().parse().ok()).ok()?;
    pid
}
