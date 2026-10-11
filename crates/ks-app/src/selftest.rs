//! Проверочный режим `--selftest`: сценарии из `selftest/*.toml` (встроены при сборке)
//! печатаются в новый документ TextEdit нажатиями «как пользователь», текст поля читается
//! через Accessibility. Исполняется внутри работающего агента — права TCC агента, не Терминала.

use ks_core::PhysicalKey;
use serde::Deserialize;

include!(concat!(env!("OUT_DIR"), "/selftest_files.rs"));

#[derive(Clone, Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// `"en"` или `"ru"`.
    pub layout_before: String,
    /// Клавиши W3C; `"Shift+KeyA"` — с Shift.
    pub input: Vec<String>,
    pub expect_text: String,
    pub expect_layout: String,
    /// Пауза между нажатиями, мс.
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

#[derive(Deserialize)]
struct File {
    scenario: Vec<Scenario>,
}

impl Scenario {
    /// Нажатия сценария: клавиша и нужен ли Shift.
    pub fn strokes(&self) -> Result<Vec<(PhysicalKey, bool)>, String> {
        self.input
            .iter()
            .map(|token| {
                let (shift, name) = match token.strip_prefix("Shift+") {
                    Some(rest) => (true, rest),
                    None => (false, token.as_str()),
                };
                name.parse::<PhysicalKey>()
                    .map(|k| (k, shift))
                    .map_err(|e| format!("{}: {e}", self.name))
            })
            .collect()
    }
}

/// Все встроенные сценарии в порядке имён файлов.
pub fn scenarios() -> Result<Vec<Scenario>, String> {
    let mut all = Vec::new();
    for (file, text) in SCENARIO_FILES {
        let parsed: File = toml::from_str(text).map_err(|e| format!("{file}: {e}"))?;
        for s in &parsed.scenario {
            s.strokes()?;
            for lang in [&s.layout_before, &s.expect_layout] {
                if lang != "en" && lang != "ru" {
                    return Err(format!(
                        "{file}: {}: раскладка «{lang}», ждём en/ru",
                        s.name
                    ));
                }
            }
        }
        all.extend(parsed.scenario);
    }
    Ok(all)
}

/// Описание расхождения без содержимого поля: ожидаемая строка, длина фактической
/// и индекс первого расхождающегося символа. `None` — совпало.
pub fn mismatch(expected: &str, actual: &str) -> Option<String> {
    if expected == actual {
        return None;
    }
    let at = expected
        .chars()
        .zip(actual.chars())
        .take_while(|(a, b)| a == b)
        .count();
    Some(format!(
        "ожидалось {expected:?} ({} симв.), в поле {} симв., первое расхождение на символе {at}",
        expected.chars().count(),
        actual.chars().count()
    ))
}

#[cfg(target_os = "macos")]
pub use mac::run;

#[cfg(target_os = "macos")]
mod mac {
    use std::io::{self, Write};
    use std::time::{Duration, Instant};

    use ks_core::{Lang, LayoutId, ResetReason};
    use ks_macos::keycode::{self, FLAG_SHIFT};
    use ks_macos::probe::{self, Field};
    use ks_platform::{Backend, KeyHandler};

    use super::{Scenario, mismatch, scenarios};

    const TEXTEDIT: &str = "com.apple.TextEdit";
    /// Собственный документ проверки: трогать можно только его.
    const FILE_NAME: &str = "KeySwitcher-selftest.txt";
    const DEFAULT_DELAY_MS: u64 = 40;
    /// Нейтральный префикс перед каждым сценарием: набор идёт не с начала документа, и
    /// автокапитализация macOS первую букву не трогает. Ожидания сценариев — после него.
    const PREFIX: &str = "x ";

    fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let start = Instant::now();
        loop {
            if let Some(v) = f() {
                return Some(v);
            }
            if start.elapsed() > timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn textedit_pid() -> Result<i32, String> {
        match probe::frontmost_app() {
            Some((id, pid)) if id == TEXTEDIT => Ok(pid),
            _ => Err("фокус ушёл из TextEdit — проверка остановлена".to_owned()),
        }
    }

    fn field() -> Result<Field, String> {
        probe::own_field(textedit_pid()?, FILE_NAME)
    }

    fn open_textedit() -> Result<(), String> {
        let path = std::env::temp_dir().join(FILE_NAME);
        std::fs::write(&path, "").map_err(|e| e.to_string())?;
        std::process::Command::new("open")
            .args(["-a", "TextEdit"])
            .arg(&path)
            .status()
            .map_err(|e| e.to_string())?;
        let mut last = String::new();
        wait_for(Duration::from_secs(10), || match field() {
            Ok(_) => Some(()),
            Err(e) => {
                last = e;
                None
            }
        })
        .ok_or_else(|| format!("документ {FILE_NAME} не открылся в TextEdit: {last}"))
    }

    fn lang_code(lang: Lang) -> &'static str {
        match lang {
            Lang::En => "en",
            Lang::Ru => "ru",
            Lang::Other => "other",
        }
    }

    fn run_one(
        s: &Scenario,
        handler: &dyn KeyHandler,
        layout_for: &dyn Fn(&str) -> Option<LayoutId>,
        lang_of: &dyn Fn(&LayoutId) -> &'static str,
    ) -> Result<(), String> {
        let target = layout_for(&s.layout_before).ok_or("нет раскладки layout_before")?;
        let id = target.clone();
        if !probe::on_main(move |mtm| probe::select_layout(mtm, &id)) {
            return Err("не удалось включить раскладку layout_before".to_owned());
        }
        if !field()?.set_text(PREFIX) {
            return Err("поле документа проверки не меняется через AXValue".to_owned());
        }
        std::thread::sleep(Duration::from_millis(300));
        handler.on_reset(ResetReason::External);
        field()?;
        let delay = Duration::from_millis(s.delay_ms.unwrap_or(DEFAULT_DELAY_MS));
        for (key, shift) in s.strokes()? {
            let code = keycode::from_physical(key).ok_or(format!("нет кода для {key}"))?;
            let flags = if shift { FLAG_SHIFT } else { 0 };
            probe::user_key(code, true, flags);
            probe::user_key(code, false, flags);
            std::thread::sleep(delay);
        }
        let expected = format!("{PREFIX}{}", s.expect_text);
        let mut text = String::new();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            text = field()?.text().unwrap_or_default();
            if text == expected {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let layout = probe::on_main(probe::current_layout).unwrap_or_default();
        let lang = lang_of(&layout);
        let mut errors: Vec<String> = mismatch(&expected, &text).into_iter().collect();
        if lang != s.expect_layout {
            errors.push(format!("раскладка {lang}, ожидалась {}", s.expect_layout));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Прогоняет все сценарии, пишет `PASS`/`FAIL` по строке и итог; `true` — все прошли.
    /// Ошибка записи в сокет останавливает проверку сразу.
    pub fn run(out: &mut dyn Write, handler: &dyn KeyHandler, backend: &dyn Backend) -> bool {
        run_inner(out, handler, backend).unwrap_or(false)
    }

    fn run_inner(
        out: &mut dyn Write,
        handler: &dyn KeyHandler,
        backend: &dyn Backend,
    ) -> io::Result<bool> {
        let missing: Vec<String> = backend
            .permissions()
            .into_iter()
            .filter(|p| !p.granted)
            .map(|p| p.explanation)
            .collect();
        if !missing.is_empty() {
            writeln!(out, "FAIL: у агента нет прав: {}", missing.join(" "))?;
            return Ok(false);
        }
        let layouts = backend.layouts();
        let Some((en, ru)) = layouts.pair() else {
            writeln!(out, "FAIL: Нужны раскладки RU и EN")?;
            return Ok(false);
        };
        let (en_id, ru_id) = (en.id.clone(), ru.id.clone());
        let layout_for = |code: &str| match code {
            "en" => Some(en_id.clone()),
            "ru" => Some(ru_id.clone()),
            _ => None,
        };
        let lang_of = |id: &LayoutId| layouts.get(id).map_or("other", |m| lang_code(m.lang));
        let list = match scenarios() {
            Ok(list) => list,
            Err(e) => {
                writeln!(out, "FAIL: сценарии не читаются: {e}")?;
                return Ok(false);
            }
        };
        if let Err(e) = open_textedit() {
            writeln!(out, "FAIL: {e}")?;
            return Ok(false);
        }
        let mut failed = 0;
        for s in &list {
            match run_one(s, handler, &layout_for, &lang_of) {
                Ok(()) => writeln!(out, "PASS {}", s.name)?,
                Err(e) => {
                    failed += 1;
                    writeln!(out, "FAIL {}: {e}", s.name)?;
                }
            }
            out.flush()?;
        }
        if let Ok(field) = field() {
            field.set_text("");
        }
        writeln!(out, "ИТОГ: {} PASS, {failed} FAIL", list.len() - failed)?;
        Ok(failed == 0)
    }
}
