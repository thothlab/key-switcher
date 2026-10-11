//! Встраивает сценарии `--selftest` из `selftest/*.toml`: каждая задача кладёт свой файл,
//! список собирается сам.

use std::path::Path;

fn main() {
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
