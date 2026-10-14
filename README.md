# KeySwitcher

Автопереключатель раскладки RU↔EN для macOS, Windows и Linux. Набрали «ghbdtn» в английской
раскладке и нажали пробел — в поле окажется «привет », а раскладка станет русской; «пщщв» в
русской станет «good». Работает локально, набранный текст никуда не пишется; в сеть ходит только проверка новой версии (раз в сутки, отключается), обновление ставится по кнопке в меню значка.

## Установка

Готовые сборки — на странице [Releases](https://github.com/thothlab/key-switcher/releases):
`.dmg` для macOS 13+ (Apple Silicon и Intel), `.zip` для Windows 10/11, `.tar.gz` для Linux
(glibc 2.31+). Как поставить и выдать права — в описании релиза и в файле внутри архива.
Сборки собирает CI ([.github/workflows/release.yml](.github/workflows/release.yml)) по тегу
`vX.Y.Z`.

## Устройство

Один Cargo workspace на Rust: ядро без зависимостей от ОС (`crates/ks-core`), данные
(`crates/ks-data`), контракт ОС-слоя (`crates/ks-platform`) и по бэкенду на систему
(`crates/ks-macos`, `crates/ks-windows`, `crates/ks-linux`) на прямых системных биндингах.
Бинарник агента — `keyswitcher` (`crates/ks-app`), окно настроек — `keyswitcher --settings`.

## Сборка

- macOS — [docs/build-macos.md](docs/build-macos.md)
- Windows — [docs/build-windows.md](docs/build-windows.md)
- Linux — [docs/build-linux.md](docs/build-linux.md)

Проверка ядра на любой ОС: `cargo test -p ks-core -p ks-data -p ks-platform`.

## Статус платформ

| Платформа | Состояние |
|---|---|
| macOS 13+ | работает, проверен вживую; сборка подписана ad-hoc, без нотаризации |
| Windows 10/11 | собирается, вживую не запускался |
| Linux (X11, KDE, Sway, Hyprland) | собирается, вживую не запускался; GNOME на Wayland не поддерживается |
