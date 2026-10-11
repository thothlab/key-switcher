# KeySwitcher

Автопереключатель раскладки RU↔EN для macOS, Windows и Linux. Набрали «ghbdtn» в английской
раскладке и нажали пробел — в поле окажется «привет », а раскладка станет русской; «пщщв» в
русской станет «good». Работает локально: без сети, набранный текст никуда не пишется.

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
| Ядро (распознавание, настройки) | в работе: исправление слова на границе по словарю |
| macOS | заготовка бэкенда |
| Windows | заготовка бэкенда, собирается кросс-компиляцией |
| Linux | заготовка бэкенда, проверяется `cargo clippy` под Linux-целью |
