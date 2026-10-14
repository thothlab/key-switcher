//! Раздел «О программе»: встраивает корневой `THIRD_PARTY.md` (лицензии данных) и `NOTICE*.md`
//! каждого крейта, где они есть (заимствованный код и шрифты). Заголовок карточки — первая строка
//! файла без «# » и «NOTICE — ». Новый `NOTICE*.md` подхватывается без правки кода.

use std::path::{Path, PathBuf};

fn notices(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("NOTICE") && n.ends_with(".md"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn title(file: &Path, fallback: &str) -> String {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let first = text.lines().next().unwrap_or_default();
    let title = first.trim_start_matches('#').trim();
    let title = title.strip_prefix("NOTICE — ").unwrap_or(title).trim();
    if title.is_empty() {
        fallback.to_owned()
    } else {
        title.to_owned()
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    println!("cargo:rerun-if-changed={}", crates.display());
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    dirs.sort();
    for dir in dirs {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        println!("cargo:rerun-if-changed={}", dir.display());
        for file in notices(&dir) {
            println!("cargo:rerun-if-changed={}", file.display());
            entries.push((title(&file, &name), file.display().to_string()));
        }
    }
    let third_party = root.join("THIRD_PARTY.md");
    println!("cargo:rerun-if-changed={}", third_party.display());
    let mut out = format!(
        "pub static THIRD_PARTY: &str = include_str!({:?});\npub static NOTICES: &[(&str, &str)] = &[\n",
        third_party.display().to_string()
    );
    for (title, file) in entries {
        out += &format!("    ({title:?}, include_str!({file:?})),\n");
    }
    out += "];\n";
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("notices.rs");
    std::fs::write(dest, out).unwrap();
}
