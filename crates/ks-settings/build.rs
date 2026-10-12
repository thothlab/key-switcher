//! Раздел «О программе»: встраивает корневой `THIRD_PARTY.md` (лицензии данных) и `NOTICE.md`
//! каждого крейта, где они есть (заимствованный код и шрифты). Новый `NOTICE.md` подхватывается
//! без правки кода.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    println!("cargo:rerun-if-changed={}", crates.display());
    let mut notices: Vec<(String, String)> = std::fs::read_dir(&crates)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join("NOTICE.md").is_file())
                .map(|p| {
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    let file = p.join("NOTICE.md").display().to_string();
                    (name, file)
                })
                .collect()
        })
        .unwrap_or_default();
    notices.sort();
    let third_party = root.join("THIRD_PARTY.md");
    println!("cargo:rerun-if-changed={}", third_party.display());
    let mut out = format!(
        "pub static THIRD_PARTY: &str = include_str!({:?});\npub static NOTICES: &[(&str, &str)] = &[\n",
        third_party.display().to_string()
    );
    for (name, file) in notices {
        println!("cargo:rerun-if-changed={file}");
        out += &format!("    ({name:?}, include_str!({file:?})),\n");
    }
    out += "];\n";
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("notices.rs");
    std::fs::write(dest, out).unwrap();
}
