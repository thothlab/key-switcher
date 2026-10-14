//! Растры значка в строке меню и в трее (`design/icons/tray`, копии в `assets/tray`): по
//! состоянию; на macOS — чёрный шаблон @2x, на Windows — под светлую или тёмную панель задач и
//! размер малого значка, на Linux — цветные 24 и 48 для StatusNotifierItem.

use ks_platform::TrayStatus;

/// Растр RGBA 8 бит.
pub struct Art {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// PNG → RGBA; `None` — не PNG RGBA 8 бит.
pub fn decode(png_bytes: &[u8]) -> Option<Art> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(Art {
        width: info.width,
        height: info.height,
        rgba: buf,
    })
}

/// Имя файла состояния.
pub fn stem(status: TrayStatus) -> &'static str {
    match status {
        TrayStatus::Working => "working",
        TrayStatus::AutocorrectOff => "autocorrect-off",
        TrayStatus::SecureInput => "secure-input",
        TrayStatus::NeedsPermissions => "needs-permissions",
        TrayStatus::NeedsLayouts => "needs-layouts",
        TrayStatus::ConfigError => "config-error",
    }
}

pub const ALL: [TrayStatus; 6] = [
    TrayStatus::Working,
    TrayStatus::AutocorrectOff,
    TrayStatus::SecureInput,
    TrayStatus::NeedsPermissions,
    TrayStatus::NeedsLayouts,
    TrayStatus::ConfigError,
];

macro_rules! asset {
    ($dir:literal, $file:expr) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tray/",
            $dir,
            "/",
            $file
        )) as &[u8]
    };
}

/// Чёрный шаблон 36×36 (18 pt @2x): строка меню перекрашивает его под тему.
#[cfg(target_os = "macos")]
pub fn macos(status: TrayStatus) -> Option<Art> {
    decode(match status {
        TrayStatus::Working => asset!("macos", "working@2x.png"),
        TrayStatus::AutocorrectOff => asset!("macos", "autocorrect-off@2x.png"),
        TrayStatus::SecureInput => asset!("macos", "secure-input@2x.png"),
        TrayStatus::NeedsPermissions => asset!("macos", "needs-permissions@2x.png"),
        TrayStatus::NeedsLayouts => asset!("macos", "needs-layouts@2x.png"),
        TrayStatus::ConfigError => asset!("macos", "config-error@2x.png"),
    })
}

#[cfg(windows)]
macro_rules! windows_set {
    ($($stem:literal),*) => {
        &[$(
            (concat!($stem, "-light-16"), asset!("windows", concat!($stem, "-light-16.png"))),
            (concat!($stem, "-light-20"), asset!("windows", concat!($stem, "-light-20.png"))),
            (concat!($stem, "-light-24"), asset!("windows", concat!($stem, "-light-24.png"))),
            (concat!($stem, "-light-32"), asset!("windows", concat!($stem, "-light-32.png"))),
            (concat!($stem, "-dark-16"), asset!("windows", concat!($stem, "-dark-16.png"))),
            (concat!($stem, "-dark-20"), asset!("windows", concat!($stem, "-dark-20.png"))),
            (concat!($stem, "-dark-24"), asset!("windows", concat!($stem, "-dark-24.png"))),
            (concat!($stem, "-dark-32"), asset!("windows", concat!($stem, "-dark-32.png"))),
        )*]
    };
}

#[cfg(windows)]
const WINDOWS: &[(&str, &[u8])] = windows_set!(
    "working",
    "autocorrect-off",
    "secure-input",
    "needs-permissions",
    "needs-layouts",
    "config-error"
);

/// Значок под панель задач: `light` — светлая (тёмный колпачок), `px` — сторона малого значка;
/// берётся ближайший нарисованный размер не меньше `px` (16, 20, 24, 32).
#[cfg(windows)]
pub fn windows(status: TrayStatus, light: bool, px: u32) -> Option<Art> {
    let size = [16, 20, 24, 32]
        .into_iter()
        .find(|s| *s >= px)
        .unwrap_or(32);
    let theme = if light { "light" } else { "dark" };
    let name = format!("{}-{theme}-{size}", stem(status));
    WINDOWS
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, bytes)| decode(bytes))
}

/// Цветной значок 24 и 48: хост StatusNotifierItem выбирает размер сам.
#[cfg(target_os = "linux")]
pub fn linux(status: TrayStatus) -> Vec<Art> {
    let files: [&[u8]; 2] = match status {
        TrayStatus::Working => [
            asset!("linux", "working-24.png"),
            asset!("linux", "working-48.png"),
        ],
        TrayStatus::AutocorrectOff => [
            asset!("linux", "autocorrect-off-24.png"),
            asset!("linux", "autocorrect-off-48.png"),
        ],
        TrayStatus::SecureInput => [
            asset!("linux", "secure-input-24.png"),
            asset!("linux", "secure-input-48.png"),
        ],
        TrayStatus::NeedsPermissions => [
            asset!("linux", "needs-permissions-24.png"),
            asset!("linux", "needs-permissions-48.png"),
        ],
        TrayStatus::NeedsLayouts => [
            asset!("linux", "needs-layouts-24.png"),
            asset!("linux", "needs-layouts-48.png"),
        ],
        TrayStatus::ConfigError => [
            asset!("linux", "config-error-24.png"),
            asset!("linux", "config-error-48.png"),
        ],
    };
    files.into_iter().filter_map(decode).collect()
}
