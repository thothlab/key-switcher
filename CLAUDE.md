<!-- prd:start -->
# KeySwitcher

Нативный переключатель раскладки RU↔EN для macOS, Windows и Linux на Rust: исправляет слово, набранное не в той раскладке («ghbdtn» → «привет»), на пробеле и по горячим клавишам. Вживую проверен только macOS; Windows и Linux собираются, но не запускались.

## Команды

| Команда | Что делает |
|---------|------------|
| `source "$HOME/.cargo/env"` | Подключить Rust: zsh на этой машине сам его не видит |
| `cargo test --workspace` | Все тесты (~240); один крейт — `cargo test -p ks-core`, один тест — `cargo test -p ks-core <имя>` |
| `cargo clippy --workspace --all-targets -- -D warnings` | Линтер; держит коммит вместе с `cargo fmt --all -- --check` |
| `cargo clippy -p ks-windows -p ks-app --target x86_64-pc-windows-gnu -- -D warnings` | Проверка под Windows; под Linux — `-p ks-linux -p ks-app --target x86_64-unknown-linux-gnu` |
| `KS_SIGN_IDENTITY="KeySwitcher Local" sh scripts/macos/build-app.sh` | Собрать и подписать `dist/KeySwitcher.app`; после — `pkill -f "KeySwitcher.app/Contents/MacOS/keyswitcher"; open dist/KeySwitcher.app` |
| `dist/KeySwitcher.app/Contents/MacOS/keyswitcher --status` | Состояние работающего агента: права, перехват, раскладки, приложение в фокусе |
| `dist/KeySwitcher.app/Contents/MacOS/keyswitcher --selftest` | 18 живых сценариев в собственном документе TextEdit (нужны права TCC; при чужом фокусе отказывается) |
| `dist/KeySwitcher.app/Contents/MacOS/keyswitcher --settings` | Окно настроек; `--section hotkeys` (general, apps, words, permissions, about) — сразу на разделе |
| `KS_SETTINGS_SHOT=/tmp/s.bmp target/debug/keyswitcher --settings --section apps` | Только отладочная сборка: окно снимает свой кадр в BMP и закрывается (`KS_SETTINGS_SIZE=640x420` — размер); `sips -s format png` — в PNG |
| `sh scripts/windows/build.sh` / `sh scripts/linux/build.sh` | Кросс-сборка `dist/windows/keyswitcher.exe` (mingw-w64) и `dist/linux/keyswitcher` (zig + cargo-zigbuild) |
| `cargo run --release -p ks-eval -- --part control` | Замер качества распознавания; `--write docs/eval.md` обновляет отчёт |
| `cargo run --release -p ks-datagen` | Перегенерировать `data/` из hunspell (сеть; Tatoeba только с `--resample-eval`) |
| `git tag vX.Y.Z && git push ssh://git@ssh.github.com:443/thothlab/key-switcher.git vX.Y.Z` | Релиз: CI (`.github/workflows/release.yml`) собирает `.dmg`/`.zip`/`.tar.gz` и публикует предварительный релиз; тег = `version` из `Cargo.toml`. Без тега — `gh workflow run release.yml` (только артефакты) |

## Каталоги

```
crates/ks-core       движок: буфер слова, жесты и хоткеи (input/), правило распознавания (detect/), настройки (config/)
crates/ks-data       Lexicon: словари fst, символьная LM, короткие слова, слабые формы, исключённые приложения
crates/ks-platform   контракт ОС-слоя (Backend, KeyHandler, TrayHost) и общая модель исполнения (intercept, exec)
crates/ks-macos      CGEventTap, TIS/UCKeyTranslate, CGEventPost, NSPasteboard, LaunchAgent
crates/ks-windows    WH_KEYBOARD_LL, SendInput, KLC-таблицы, ключ Run
crates/ks-linux      evdev без захвата + uinput, смена раскладки X11/KDE/Sway/Hyprland, трей ksni, udev
crates/ks-app        бинарник keyswitcher: агент, трей, сокет agent.sock, --status/--selftest/--settings
crates/ks-settings   окно настроек на egui (отдельный процесс)
crates/ks-eval       замер качества на Tatoeba (data/eval/, части tune/control)
tools/ks-datagen     генератор data/ из hunspell LibreOffice
data/                сгенерированные словари и модели (коммитятся), лицензии — data/licenses/, THIRD_PARTY.md
scripts/             сборка .app и локальный сертификат (macos), build.sh (windows, linux), udev-правило (linux)
packaging/           заметки к релизу и README внутри .dmg/.zip/.tar.gz (CI берёт их отсюда)
docs/                build-macos.md, build-windows.md, build-linux.md, eval.md
design/              бриф дизайнеру (prompt.md: окно настроек, иконки приложения и трея), снимки текущего окна; сюда же ложится сдача
```

## Архитектура

- Поток: перехват ОС → `InputEvent<N>` → `ks_platform::intercept::TapLogic` (удержание нажатий во время замены, сроки) → `KeyHandler::on_key` → `ks_core::Engine::on_key` → `Outcome { disposition, actions }` → `ks_platform::exec::Executor` исполняет `Action::Replace/SwitchLayout/ConvertSelection` через `Poster`/`Switcher`/`Clipboard` бэкенда.
- Ядро не знает ОС: время только из `KeyEvent.time_ms`, раскладки — `LayoutMap`, построенные бэкендом из ОС (`UCKeyTranslate`, `ToUnicodeEx`, xkbcommon), не зашитые.
- Решение на границе слова (`detect::decide`): вето (безопасный ввод, исключённое приложение через `AppRules::excludes`, `words.never`, вид токена, пароль) → CaseFix → словарное вето (кроме слабых форм `data/weak_forms.tsv`) → короткие слова по спискам и контексту → LM. Однобуквенное слово перед исправленным словом исправляется вместе с ним.
- Отмена: ⇧⇧/Pause, ⌘Z/Ctrl+Z (на отпускании модификатора) и Punto-аккорд ⌘⌥⇧; отменённое слово не исправляется снова до перезапуска (память только в процессе).
- Встроенный список приложений без автоисправления — `tools/ks-datagen/curated/excluded_apps.tsv` (ОС, категория, id, сверка, название, семейство); `data/excluded_apps.tsv` из него генерируется. Окно показывает названия, семейство (JetBrains и Android Studio) — одной строкой; в конфиге исключения остаются по id.
- Настройки — `~/Library/Application Support/KeySwitcher/config.toml` (Windows `%APPDATA%\KeySwitcher`, Linux `~/.config/keyswitcher`); агент следит за файлом и применяет на лету, окно настроек пишет тот же файл. Битый файл не перезаписывается.
- Умолчания хоткеев: macOS — как Punto Switcher (`undo_or_convert_selection = ["Meta+Alt+Shift"]`, `switch_layout = ["Meta+Space"]`, одиночный Shift выключен); Windows/Linux — `HotkeyConfig::classic` (⇧⇧, Pause, Shift+Pause, оба Shift). `--selftest` всегда гоняется на `classic`.

## Соглашения

- Набранный текст и значения `config.toml` никогда не попадают в логи, статус и ошибки; сети в продуктовых крейтах нет.
- Новый бэкенд подключается к модели `ks-platform`, а не копирует её: состояние модификаторов хранится в событии на момент захвата, `Switcher::select_layout` укладывается в `SWITCH_TIMEOUT_MS`.
- Тест обязан падать без своей правки; тесты ядра строят движок на `common::classic()`.

## Окружение

- `KS_SIGN_IDENTITY` — имя сертификата подписи для `build-app.sh` (локально «KeySwitcher Local»).
- `KS_UNIVERSAL` — собрать универсальный (arm64 + x86_64) `.app`.
- `CARGO_TARGET_DIR` — свой каталог сборки, если в дереве параллельно собирается что-то ещё.

## Что уже кусалось

- Права TCC привязаны к подписи: собирай с сертификатом «KeySwitcher Local» (`scripts/macos/create-local-cert.sh`), иначе после каждой пересборки права молча слетают.
- После выдачи прав агента надо перезапустить: TCC применяет их только к новому процессу.
- Автокапитализация macOS делает заглавной первую букву документа — самопроверка поэтому печатает после префикса «x ».
- `.githooks/prepare-commit-msg` (через `core.hooksPath`) вырезает строки `Co-Authored-By` из сообщений коммитов.
- У терминала этой машины нет прав «Запись экрана» и «Универсальный доступ»: `screencapture` и синтетические щелчки и нажатия из shell не работают. Вид окна настроек — через `KS_SETTINGS_SHOT`, щелчки и запись сочетаний в окне проверяет пользователь.
- Токен `gh` без права `workflow`: файлы `.github/workflows/` по HTTPS не пушатся, порт 22 закрыт — пушить через `ssh://git@ssh.github.com:443/thothlab/key-switcher.git`.
- Релизный `.dmg` подписан ad-hoc (Developer ID нет): у пользователя Gatekeeper просит «Всё равно открыть», а права TCC после каждого обновления выдаются заново. Тексты установки — `packaging/`.
- ⌘Пробел и ⌃Пробел macOS забирает раньше окна (Spotlight, смена источника ввода): окно предлагает их готовыми в списке «…», а не записывает нажатием. В своём окне (`com.keyswitcher.app`) агент пропускает все нажатия — `OWN_APP_IDS` в `ks-app/src/handler.rs`.

## Инициативы и PRD

Документы первой инициативы: `~/Documents/openclaw_obsidian_vault/Projects/key-switcher/switcher/PRD/` — запрос, требования, PRD, спек-дельты, задачи и отчёты; анализ аналогов, платформ и алгоритма — там же в `analysis/`.
<!-- prd:end -->
