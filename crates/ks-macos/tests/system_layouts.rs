//! Таблицы `UCKeyTranslate` с этой машины против фикстур задачи 01 (US, «Русская»,
//! «Русская – ПК»). Свой `main` без тестовой обвязки: TIS разрешён только на главном потоке.

#[cfg(target_os = "macos")]
fn main() {
    use std::path::Path;

    use ks_core::{Lang, LayoutId, LayoutMap, PhysicalKey};
    use ks_macos::keycode;
    use ks_macos::layouts::LEVELS;
    use ks_macos::system::{layout_by_id, machine_kbd_type, main_thread};

    fn fixture(file: &str) -> LayoutMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ks-core/tests/fixtures")
            .join(file);
        let text = std::fs::read_to_string(&path).expect("фикстура");
        let mut map = LayoutMap::new(LayoutId(String::new()), Lang::Ru, file);
        for line in text
            .lines()
            .filter(|l| !l.starts_with(['#', '@']) && !l.is_empty())
        {
            let mut cols = line.split('\t');
            let key: PhysicalKey = cols.next().unwrap().parse().expect("код W3C");
            for (level, cell) in LEVELS.into_iter().zip(cols) {
                if let Some(ch) = cell.chars().next() {
                    map.insert(key, level, ch);
                }
            }
        }
        map
    }

    fn compare(os: &LayoutMap, fx: &LayoutMap, skip: &[PhysicalKey]) -> Vec<String> {
        let mut diffs = Vec::new();
        for (_, key) in keycode::printable().filter(|(_, k)| !skip.contains(k)) {
            for level in LEVELS {
                let (a, b) = (os.char_for(key, level), fx.char_for(key, level));
                if a != b {
                    diffs.push(format!("{key} {level:?}: ОС {a:?}, фикстура {b:?}"));
                }
            }
        }
        diffs
    }

    let mtm = main_thread("тест таблиц").expect("главный поток");
    let machine = machine_kbd_type(mtm);
    let cases = [
        ("com.apple.keylayout.US", "us.tsv", Lang::En),
        ("com.apple.keylayout.Russian", "ru_apple.tsv", Lang::Ru),
        ("com.apple.keylayout.RussianWin", "ru_pc.tsv", Lang::Ru),
    ];
    let mut failed = 0;
    for (id, file, lang) in cases {
        let fx = fixture(file);
        // Фикстуры сняты при kbdType 40 (ANSI): сверка целиком.
        let os = layout_by_id(mtm, id, 40).unwrap_or_else(|| panic!("нет раскладки {id}"));
        let mut diffs = compare(&os, &fx, &[]);
        if os.lang != lang {
            diffs.push(format!("язык {:?}, ожидался {lang:?}", os.lang));
        }
        // С kbdType машины клавиши ISO (`§`/`` ` ``) могут поменяться местами — их не сверяем.
        let iso = [PhysicalKey::Backquote, PhysicalKey::IntlBackslash];
        let os_machine = layout_by_id(mtm, id, machine).expect("раскладка");
        diffs.extend(
            compare(&os_machine, &fx, &iso)
                .into_iter()
                .map(|d| format!("kbdType {machine}: {d}")),
        );
        if diffs.is_empty() {
            println!("test {id} ({file}) ... ok");
        } else {
            failed += 1;
            println!("test {id} ({file}) ... FAILED");
            for d in diffs {
                println!("    {d}");
            }
        }
    }
    assert_eq!(failed, 0, "таблицы ОС расходятся с фикстурами");
    println!("test result: ok. 3 passed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
