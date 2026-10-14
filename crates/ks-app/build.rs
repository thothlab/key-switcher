//! Встраивает сценарии `--selftest` из `selftest/*.toml`: каждая задача кладёт свой файл,
//! список собирается сам. Для Windows ещё и иконку `.exe` из `packaging/icons/windows`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("selftest");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "toml"));
    files.sort();
    let mut out = String::from("pub static SCENARIO_FILES: &[(&str, &str)] = &[\n");
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.file_name().unwrap().to_string_lossy();
        out += &format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        );
    }
    out += "];\n";
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("selftest_files.rs");
    std::fs::write(dest, out).unwrap();
}

/// Иконка `keyswitcher.exe`: ICO из PNG всех размеров и ресурс, скомпилированный `windres`
/// (mingw-w64; другой — через `WINDRES`). Без `windres` exe собирается без иконки.
fn windows_icon() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/icons/windows");
    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-env-changed=WINDRES");
    let images: Vec<(u32, Vec<u8>)> = [16, 24, 32, 48, 64, 128, 256]
        .into_iter()
        .filter_map(|n| {
            std::fs::read(dir.join(format!("icon-{n}.png")))
                .ok()
                .map(|b| (n, b))
        })
        .collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("keyswitcher.ico");
    std::fs::write(&ico, ico_bytes(&images)).unwrap();
    let rc = out.join("keyswitcher.rc");
    let ico_path = ico.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).unwrap();
    let obj = out.join("keyswitcher-icon.o");
    let windres =
        std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".to_owned());
    let built = Command::new(&windres)
        .arg(&rc)
        .args(["-O", "coff", "-o"])
        .arg(&obj)
        .status()
        .is_ok_and(|s| s.success());
    if built {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    } else {
        println!("cargo:warning=нет {windres}: keyswitcher.exe собран без иконки");
    }
}

/// ICO с PNG внутри (Windows Vista+): заголовок, каталог по 16 байт, затем PNG как есть.
fn ico_bytes(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, png) in images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}
