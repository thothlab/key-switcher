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

const USAGE: &str = "использование: keyswitcher [--status | --selftest | --settings [--section <general|hotkeys|apps|words>] | --version]";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => match ks_app::agent::run(backend()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("KeySwitcher: {e:#}");
                ExitCode::FAILURE
            }
        },
        Some("--status") => status(),
        Some("--selftest") => selftest(),
        Some("--settings") => settings(&args[1..]),
        Some("--version") => {
            println!("KeySwitcher {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
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
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

#[cfg(not(target_os = "macos"))]
fn selftest() -> ExitCode {
    eprintln!("--selftest пока есть только на macOS");
    ExitCode::from(2)
}

fn settings(args: &[String]) -> ExitCode {
    use ks_settings::Section;
    let section = match args {
        [] => None,
        [flag, name] if flag == "--section" => match name.as_str() {
            "general" => Some(Section::General),
            "hotkeys" => Some(Section::Hotkeys),
            "apps" => Some(Section::Apps),
            "words" => Some(Section::Words),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(path) = paths::config_file() else {
        eprintln!("не найден каталог настроек");
        return ExitCode::FAILURE;
    };
    match ks_settings::run(section, &path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("KeySwitcher: {e:#}");
            ExitCode::FAILURE
        }
    }
}
