//! Состояние окна настроек без отрисовки: правки списков, горячих клавиш и флагов над `Config`.
//! Каждое действие окна — `Settings::edit`: перечитать файл, применить одну правку, проверить,
//! записать. Так окно не затирает то, что агент записал, пока окно открыто (команды меню), а
//! испорченный файл без явного действия пользователя не перезаписывается.

use std::path::Path;

use ks_core::{
    AppCategory, AppId, BuiltinApp, Config, ConfigError, Hotkey, HotkeyConfig, HotkeyError, Os,
    WordRules, config::MAX_PER_ACTION, validate_hotkeys,
};

/// Действие с горячими клавишами — строка раздела «Горячие клавиши».
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    UndoOrConvertSelection,
    SwitchLayout,
    UndoOrConvertWord,
    ConvertSelection,
    ToggleAutocorrect,
}

impl HotkeyAction {
    /// Порядок строк окна: сначала действия Punto Switcher, потом жесты Caramba.
    pub const ALL: [HotkeyAction; 5] = [
        HotkeyAction::UndoOrConvertSelection,
        HotkeyAction::ToggleAutocorrect,
        HotkeyAction::SwitchLayout,
        HotkeyAction::UndoOrConvertWord,
        HotkeyAction::ConvertSelection,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::UndoOrConvertSelection => {
                "Отменить конвертацию или сменить раскладку выделенного текста"
            }
            HotkeyAction::ToggleAutocorrect => "Включить/выключить автопереключение",
            HotkeyAction::SwitchLayout => "Сменить раскладку",
            HotkeyAction::UndoOrConvertWord => {
                "Отменить исправление или сменить раскладку последнего слова"
            }
            HotkeyAction::ConvertSelection => "Сменить раскладку выделенного текста",
        }
    }

    fn of(self, h: &HotkeyConfig) -> &Vec<Hotkey> {
        match self {
            HotkeyAction::UndoOrConvertSelection => &h.undo_or_convert_selection,
            HotkeyAction::SwitchLayout => &h.switch_layout,
            HotkeyAction::UndoOrConvertWord => &h.undo_or_convert_word,
            HotkeyAction::ConvertSelection => &h.convert_selection,
            HotkeyAction::ToggleAutocorrect => &h.toggle_autocorrect,
        }
    }

    fn of_mut(self, h: &mut HotkeyConfig) -> &mut Vec<Hotkey> {
        match self {
            HotkeyAction::UndoOrConvertSelection => &mut h.undo_or_convert_selection,
            HotkeyAction::SwitchLayout => &mut h.switch_layout,
            HotkeyAction::UndoOrConvertWord => &mut h.undo_or_convert_word,
            HotkeyAction::ConvertSelection => &mut h.convert_selection,
            HotkeyAction::ToggleAutocorrect => &mut h.toggle_autocorrect,
        }
    }
}

/// Отказ добавить слово. Текст показывается в окне и самого слова не содержит.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WordError {
    #[error(
        "Слово из 1–2 букв не добавляется: короткие слова автоматика решает по контексту, \
         запрет сломал бы их исправление"
    )]
    TooShort,
    #[error("Это слово уже в списке")]
    Duplicate,
    #[error("Введите слово")]
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error("Это приложение уже в списке")]
    Duplicate,
    #[error("Введите идентификатор приложения")]
    Empty,
}

/// Строка встроенного списка в окне: одно приложение или семейство («JetBrains IDE и Android
/// Studio») — все его идентификаторы разом. Издания с одним названием (IntelliJ IDEA и
/// IntelliJ IDEA CE) — тоже одна строка.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltinRow {
    pub category: AppCategory,
    /// Название приложения или семейства.
    pub title: &'static str,
    /// Названия приложений семейства без повторов; пусто — строка и есть приложение.
    pub members: Vec<&'static str>,
    pub ids: Vec<AppId>,
}

/// Строки встроенного списка по категориям, внутри категории — в порядке списка.
pub fn builtin_rows(apps: &[BuiltinApp]) -> Vec<BuiltinRow> {
    let mut rows: Vec<BuiltinRow> = Vec::new();
    for app in apps {
        let title = app.family.unwrap_or(app.name);
        match rows
            .iter_mut()
            .find(|r| r.category == app.category && r.title == title)
        {
            Some(row) => {
                row.ids.push(app.id.clone());
                if app.family.is_some() && !row.members.contains(&app.name) {
                    row.members.push(app.name);
                }
            }
            None => rows.push(BuiltinRow {
                category: app.category,
                title,
                members: app.family.map(|_| app.name).into_iter().collect(),
                ids: vec![app.id.clone()],
            }),
        }
    }
    rows.sort_by_key(|r| r.category);
    rows
}

/// Исключена ли строка встроенного списка: семейство может быть исключено частично — меню
/// «Не исправлять в …» переключает одно приложение.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exclusion {
    /// Автоматика выключена во всех приложениях строки (как по умолчанию).
    All,
    /// Включена в части приложений.
    Partly,
    /// Пользователь вернул автоматику во всех приложениях строки.
    None,
}

pub struct Settings {
    config: Config,
    /// Файл настроек не прочитан (текст без значений из файла): править нельзя.
    error: Option<String>,
    builtin: Vec<BuiltinApp>,
    builtin_ids: Vec<AppId>,
}

impl Settings {
    /// `builtin` — встроенный список приложений без автоматики (`Lexicon::builtin_apps`).
    pub fn new(loaded: Result<Config, ConfigError>, builtin: Vec<BuiltinApp>) -> Settings {
        let (config, error) = match loaded {
            Ok(config) => (config, None),
            Err(e) => (Config::default_for(Os::current()), Some(e.to_string())),
        };
        Settings {
            config,
            error,
            builtin_ids: builtin.iter().map(|a| a.id.clone()).collect(),
            builtin,
        }
    }

    pub fn load(path: &Path, builtin: Vec<BuiltinApp>) -> Settings {
        Settings::new(Config::load(path), builtin)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Ошибка чтения файла настроек — показать в окне.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Файл прочитан — правки можно записывать.
    pub fn writable(&self) -> bool {
        self.error.is_none()
    }

    pub fn builtin(&self) -> &[AppId] {
        &self.builtin_ids
    }

    pub fn builtin_rows(&self) -> Vec<BuiltinRow> {
        builtin_rows(&self.builtin)
    }

    /// Название приложения из встроенного списка.
    pub fn builtin_name(&self, app: &AppId) -> Option<&'static str> {
        self.builtin.iter().find(|a| a.id == *app).map(|a| a.name)
    }

    /// Перечитывает файл: окно видит правки агента (команды меню). Битый файл — ошибка,
    /// показанные значения остаются прежними.
    pub fn reload(&mut self, path: &Path) {
        match Config::load(path) {
            Ok(config) => {
                self.config = config;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Одно действие окна: перечитать файл, применить `change`, записать. Битый файл — `Err`
    /// с текстом ошибки, файл не трогается; отказ `change` — его текст, файл не трогается.
    pub fn edit(
        &mut self,
        path: &Path,
        change: impl FnOnce(&mut Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        self.reload(path);
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        change(self)?;
        self.config
            .save(path)
            .map_err(|e| format!("файл настроек не записан: {e}"))
    }

    /// Явное действие пользователя при испорченном файле: записать умолчания поверх.
    pub fn reset_to_defaults(&mut self, path: &Path) -> Result<(), String> {
        let config = Config::default_for(Os::current());
        config
            .save(path)
            .map_err(|e| format!("файл настроек не записан: {e}"))?;
        self.config = config;
        self.error = None;
        Ok(())
    }

    pub fn set_autocorrect(&mut self, on: bool) {
        self.config.autocorrect = on;
    }

    pub fn set_single_shift_switch(&mut self, on: bool) {
        self.config.single_shift_switch = on;
    }

    /// Флаг в файле; запись автозапуска в системе ставит и снимает агент.
    pub fn set_autostart(&mut self, on: bool) {
        self.config.autostart = on;
    }

    pub fn set_check_updates(&mut self, on: bool) {
        self.config.check_updates = on;
    }

    pub fn hotkeys(&self, action: HotkeyAction) -> &[Hotkey] {
        action.of(&self.config.hotkeys)
    }

    /// Назначает `hotkey` в ячейку `slot` (`slot` = число назначений — добавить). Дубль,
    /// одиночная буква или цифра, третье назначение — ошибка, назначения не меняются.
    pub fn set_hotkey(
        &mut self,
        action: HotkeyAction,
        slot: usize,
        hotkey: Hotkey,
    ) -> Result<(), HotkeyError> {
        let mut candidate = self.config.hotkeys.clone();
        let list = action.of_mut(&mut candidate);
        if slot >= MAX_PER_ACTION {
            return Err(HotkeyError::TooMany);
        }
        match list.get_mut(slot) {
            Some(cell) => *cell = hotkey,
            None => list.push(hotkey),
        }
        validate_hotkeys(&candidate)?;
        self.config.hotkeys = candidate;
        Ok(())
    }

    /// «Убрать»: пустой список — действие без хоткея.
    pub fn remove_hotkey(&mut self, action: HotkeyAction, slot: usize) {
        let list = action.of_mut(&mut self.config.hotkeys);
        if slot < list.len() {
            list.remove(slot);
        }
    }

    /// Флажок действия снят: все назначения убираются, прежние возвращаются вызывающему.
    pub fn clear_hotkeys(&mut self, action: HotkeyAction) -> Vec<Hotkey> {
        std::mem::take(action.of_mut(&mut self.config.hotkeys))
    }

    /// Флажок поставлен снова: прежние назначения возвращаются целиком. Сочетание успели отдать
    /// другому действию — ошибка, назначения не меняются.
    pub fn restore_hotkeys(
        &mut self,
        action: HotkeyAction,
        hotkeys: Vec<Hotkey>,
    ) -> Result<(), HotkeyError> {
        let mut candidate = self.config.hotkeys.clone();
        *action.of_mut(&mut candidate) = hotkeys;
        validate_hotkeys(&candidate)?;
        self.config.hotkeys = candidate;
        Ok(())
    }

    pub fn add_word(&mut self, word: &str) -> Result<(), WordError> {
        let word = word.trim();
        if word.is_empty() {
            return Err(WordError::Empty);
        }
        if !WordRules::accepts(word) {
            return Err(WordError::TooShort);
        }
        if !self.config.words.add(word) {
            return Err(WordError::Duplicate);
        }
        Ok(())
    }

    pub fn remove_word(&mut self, word: &str) {
        self.config.words.never.retain(|w| w != word);
    }

    pub fn clear_words(&mut self) {
        self.config.words.never.clear();
    }

    /// Пользователь включил автоматику в приложении из встроенного списка.
    pub fn builtin_enabled(&self, app: &AppId) -> bool {
        self.config.apps.enabled.contains(app)
    }

    pub fn set_builtin_enabled(&mut self, app: &AppId, on: bool) {
        let enabled = &mut self.config.apps.enabled;
        enabled.retain(|a| a != app);
        if on {
            enabled.push(app.clone());
        }
    }

    pub fn exclusion(&self, row: &BuiltinRow) -> Exclusion {
        let enabled = row.ids.iter().filter(|id| self.builtin_enabled(id)).count();
        match enabled {
            0 => Exclusion::All,
            n if n == row.ids.len() => Exclusion::None,
            _ => Exclusion::Partly,
        }
    }

    /// Флажок строки: `true` — автоматика выключена во всех приложениях строки.
    pub fn set_excluded(&mut self, row: &BuiltinRow, excluded: bool) {
        for id in &row.ids {
            self.set_builtin_enabled(id, !excluded);
        }
    }

    /// Своё исключение: автоматика выключена в приложении `id`.
    pub fn add_app(&mut self, id: &str) -> Result<(), AppError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Empty);
        }
        let app = AppId(id.to_owned());
        if self.config.apps.disabled.contains(&app) {
            return Err(AppError::Duplicate);
        }
        self.config.apps.disabled.push(app);
        Ok(())
    }

    pub fn remove_app(&mut self, app: &AppId) {
        self.config.apps.disabled.retain(|a| a != app);
    }
}
