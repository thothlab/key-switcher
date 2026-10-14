//! Проверка новой версии и обновление по кнопке. Единственный сетевой запрос приложения:
//! список релизов GitHub (`curl`, он есть в macOS, Windows 10+ и почти везде в Linux; своего
//! HTTP-клиента и TLS в бинарнике нет). Ничего, кроме версии, наружу не уходит. Ставится
//! обновление только после нажатия пользователя: на macOS подменяется бандл и агент
//! перезапускается, на Windows и Linux открывается страница релиза.

use std::process::Command;

const RELEASES: &str = "https://api.github.com/repos/thothlab/key-switcher/releases?per_page=5";
pub const RELEASES_PAGE: &str = "https://github.com/thothlab/key-switcher/releases";

/// Суффикс файла релиза для этой ОС (см. `.github/workflows/release.yml`).
const ASSET_SUFFIX: &str = if cfg!(target_os = "macos") {
    "-macos-universal.dmg"
} else if cfg!(windows) {
    "-windows-x86_64.zip"
} else {
    "-linux-x86_64.tar.gz"
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Без «v»: «0.1.1».
    pub version: String,
    /// Файл для этой ОС; `None` — в релизе его нет.
    pub asset: Option<String>,
}

/// Числа версии: «0.1.10» → [0, 1, 10]; всё нечисловое — 0.
fn parts(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split(['.', '-'])
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// `candidate` новее `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parts(candidate), parts(current));
    (0..a.len().max(b.len())).find_map(|i| {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        (x != y).then_some(x > y)
    }) == Some(true)
}

/// Строка-значение ключа JSON, начиная с `from`: `"key": "value"` → (значение, позиция после).
fn string_value(json: &str, key: &str, from: usize) -> Option<(String, usize)> {
    let needle = format!("\"{key}\"");
    let at = from + json.get(from..)?.find(&needle)? + needle.len();
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let consumed = json.len() - rest.len() + end + 1;
    Some((rest[..end].to_owned(), consumed))
}

/// Самый новый релиз из ответа GitHub (список идёт от новых к старым, черновиков в нём нет).
pub fn parse_latest(json: &str) -> Option<Release> {
    let (tag, after) = string_value(json, "tag_name", 0)?;
    let end = json
        .get(after..)
        .and_then(|rest| rest.find("\"tag_name\""))
        .map_or(json.len(), |i| after + i);
    let mut asset = None;
    let mut at = after;
    while let Some((url, next)) = string_value(&json[..end], "browser_download_url", at) {
        if url.ends_with(ASSET_SUFFIX) {
            asset = Some(url);
            break;
        }
        at = next;
    }
    Some(Release {
        version: tag.trim_start_matches('v').to_owned(),
        asset,
    })
}

fn curl() -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "20", "-H", "User-Agent: KeySwitcher"]);
    cmd
}

/// Новее текущей версии релиз на GitHub, если он есть; любая ошибка сети — `None`.
pub fn check() -> Option<Release> {
    let out = curl().arg(RELEASES).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let release = parse_latest(&String::from_utf8_lossy(&out.stdout))?;
    is_newer(&release.version, env!("CARGO_PKG_VERSION")).then_some(release)
}

/// Открывает страницу релизов в браузере.
pub fn open_page() {
    let result = if cfg!(target_os = "macos") {
        Command::new("/usr/bin/open").arg(RELEASES_PAGE).status()
    } else if cfg!(windows) {
        Command::new("cmd")
            .args(["/c", "start", "", RELEASES_PAGE])
            .status()
    } else {
        Command::new("xdg-open").arg(RELEASES_PAGE).status()
    };
    if let Err(e) = result {
        log::warn!("страница релизов не открыта: {e}");
    }
}

/// Скачивает релиз и подменяет бандл `KeySwitcher.app`; запускать новый экземпляр — дело
/// вызывающего. Ошибка описывает шаг и не содержит ничего, кроме путей и кодов выхода.
#[cfg(target_os = "macos")]
pub fn install(release: &Release) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    let url = release
        .asset
        .as_deref()
        .ok_or("в релизе нет файла для macOS")?;
    let bundle = ks_macos::bundle::current_bundle().ok_or("агент запущен не из KeySwitcher.app")?;
    let work = std::env::temp_dir().join(format!("ks-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    let mount = work.join("mnt");
    fs::create_dir_all(&mount).map_err(|e| format!("временный каталог: {e}"))?;
    let run = |what: &str, cmd: &mut Command| -> Result<(), String> {
        match cmd.output() {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(format!("{what}: {}", o.status)),
            Err(e) => Err(format!("{what}: {e}")),
        }
    };
    let dmg = work.join("update.dmg");
    run("загрузка", curl().arg("-o").arg(&dmg).arg(url))?;
    run(
        "монтирование образа",
        Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noverify",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(&dmg),
    )?;
    let staged = bundle.with_extension("app.new");
    let _ = fs::remove_dir_all(&staged);
    let copied = run(
        "копирование",
        Command::new("/usr/bin/ditto")
            .arg(mount.join("KeySwitcher.app"))
            .arg(&staged),
    );
    let _ = Command::new("/usr/bin/hdiutil")
        .arg("detach")
        .arg(&mount)
        .output();
    copied?;
    let old = bundle.with_extension("app.old");
    let _ = fs::remove_dir_all(&old);
    fs::rename(&bundle, &old).map_err(|e| format!("замена бандла: {e}"))?;
    if let Err(e) = fs::rename(&staged, &bundle) {
        let _ = fs::rename(&old, &bundle);
        return Err(format!("замена бандла: {e}"));
    }
    let _ = fs::remove_dir_all(&old);
    let _ = fs::remove_dir_all(Path::new(&work));
    Ok(())
}
