//! Окружение по переменным среды: сессия (X11/Wayland), рабочий стол и способ смены раскладки.
//! Без системных вызовов: переменные приходят замыканием.

/// Тип графической сессии.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    X11,
    Wayland,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Desktop {
    Kde,
    Gnome,
    Sway,
    Hyprland,
    Mate,
    /// Прочее: значение `XDG_CURRENT_DESKTOP` как есть (может быть пустым).
    Other(String),
}

/// Чем переключается раскладка (`analysis/platforms.md` §3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchMethod {
    /// X11: `XkbLockGroup`.
    XkbGroup,
    /// KDE Plasma (X11 и Wayland): D-Bus `org.kde.keyboard /Layouts`.
    KdeDbus,
    /// Sway: `input type:keyboard xkb_switch_layout`.
    SwayIpc,
    /// Hyprland: `switchxkblayout`.
    HyprlandIpc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub session: Session,
    pub desktop: Desktop,
    /// `None` — смена раскладки в этом окружении не поддерживается.
    pub switch: Option<SwitchMethod>,
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

impl Environment {
    /// Окружение по набору переменных: `var("WAYLAND_DISPLAY")` и т.п.
    /// Wayland побеждает `DISPLAY`: внутри Wayland-сессии `DISPLAY` указывает на XWayland.
    pub fn detect(var: impl Fn(&str) -> Option<String>) -> Environment {
        let session_type = non_empty(var("XDG_SESSION_TYPE")).map(|s| s.to_ascii_lowercase());
        let session = if non_empty(var("WAYLAND_DISPLAY")).is_some()
            || session_type.as_deref() == Some("wayland")
        {
            Session::Wayland
        } else if non_empty(var("DISPLAY")).is_some() || session_type.as_deref() == Some("x11") {
            Session::X11
        } else {
            Session::Unknown
        };
        let current = non_empty(var("XDG_CURRENT_DESKTOP")).unwrap_or_default();
        let has = |name: &str| {
            current
                .split(':')
                .any(|part| part.trim().eq_ignore_ascii_case(name))
        };
        let desktop = if non_empty(var("HYPRLAND_INSTANCE_SIGNATURE")).is_some() || has("Hyprland")
        {
            Desktop::Hyprland
        } else if non_empty(var("SWAYSOCK")).is_some() || has("sway") {
            Desktop::Sway
        } else if has("KDE") || var("KDE_FULL_SESSION").as_deref() == Some("true") {
            Desktop::Kde
        } else if has("MATE") {
            Desktop::Mate
        } else if has("GNOME") {
            Desktop::Gnome
        } else {
            Desktop::Other(current.clone())
        };
        let switch = match (&desktop, session) {
            (_, Session::Unknown) => None,
            (Desktop::Hyprland, Session::Wayland) => Some(SwitchMethod::HyprlandIpc),
            (Desktop::Sway, Session::Wayland) => Some(SwitchMethod::SwayIpc),
            (Desktop::Kde, _) => Some(SwitchMethod::KdeDbus),
            // MATE: группа XKB не отражает реальную раскладку (PRD «Вне целей»).
            (Desktop::Mate, _) => None,
            (_, Session::X11) => Some(SwitchMethod::XkbGroup),
            // GNOME, labwc (Budgie/Xfce), прочие композиторы Wayland: внешнего способа нет.
            (_, Session::Wayland) => None,
        };
        Environment {
            session,
            desktop,
            switch,
        }
    }

    /// Окружение текущего процесса.
    pub fn from_process() -> Environment {
        Environment::detect(|name| std::env::var(name).ok())
    }

    /// Для `--status` и меню: «Wayland, KDE Plasma».
    pub fn describe(&self) -> String {
        let session = match self.session {
            Session::X11 => "X11",
            Session::Wayland => "Wayland",
            Session::Unknown => "сессия не определена",
        };
        let desktop = match &self.desktop {
            Desktop::Kde => "KDE Plasma",
            Desktop::Gnome => "GNOME",
            Desktop::Sway => "Sway",
            Desktop::Hyprland => "Hyprland",
            Desktop::Mate => "MATE",
            Desktop::Other(name) if name.is_empty() => "рабочий стол не определён",
            Desktop::Other(name) => name,
        };
        format!("{session}, {desktop}")
    }

    /// Почему исправления здесь выключены; `None` — окружение поддерживается.
    pub fn unsupported_reason(&self) -> Option<String> {
        self.switch.is_none().then(|| {
            format!(
                "смена раскладки в этом окружении не поддерживается ({}): исправления выключены",
                self.describe()
            )
        })
    }

    /// Приложение в фокусе узнаётся (X11, Sway, Hyprland). На Wayland в KDE и GNOME — нет.
    pub fn knows_focus(&self) -> bool {
        match self.switch {
            Some(SwitchMethod::SwayIpc | SwitchMethod::HyprlandIpc) => true,
            Some(SwitchMethod::XkbGroup) => true,
            Some(SwitchMethod::KdeDbus) => self.session == Session::X11,
            None => false,
        }
    }
}
