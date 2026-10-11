<!-- prd:start -->
# KeySwitcher

Нативный переключатель раскладки клавиатуры для macOS, Windows и Linux: исправляет текст, набранный не в той раскладке («ghbdtn» -> «привет»), на лету и по горячим клавишам.

## Команды

| Команда | Что делает |
|---------|------------|
| `source "$HOME/.cargo/env"` | Подключить Rust: zsh на этой машине сам его не видит |
| `cargo test -p <crate>` | Тесты крейта; полный набор — `cargo test --workspace` |
| `cargo clippy -p ks-windows --target x86_64-pc-windows-gnu -- -D warnings` | Проверка компиляции под Windows (так же `x86_64-unknown-linux-gnu` для Linux) |
| `KS_SIGN_IDENTITY="KeySwitcher Local" sh scripts/macos/build-app.sh` | Собрать и подписать `dist/KeySwitcher.app` |
| `dist/KeySwitcher.app/Contents/MacOS/keyswitcher --status` | Состояние работающего агента: права, перехват, раскладки |
| `dist/KeySwitcher.app/Contents/MacOS/keyswitcher --selftest` | Живая проверка в собственном документе TextEdit (нужны права TCC) |
| `cargo run --release -p ks-eval -- --part control` | Замер качества распознавания (`docs/eval.md`) |

## Что уже кусалось

- Права TCC привязаны к подписи: собирай с сертификатом «KeySwitcher Local» (`scripts/macos/create-local-cert.sh`), иначе после каждой пересборки права молча слетают.
- После выдачи прав агента надо перезапустить: TCC применяет их только к новому процессу.
- Автокапитализация macOS делает заглавной первую букву документа — самопроверка поэтому печатает после префикса «x ».
- Параллельные задачи в одном дереве: каждая собирает в свой `CARGO_TARGET_DIR`, иначе ждут блокировку cargo.

## Инициативы и PRD

Крупные изменения здесь ведутся скиллом `/prd`: он пишет PRD и спек-дельты,
режет их на задачи и исполняет по коммиту на задачу.

Где что лежит: `~/Documents/openclaw_obsidian_vault/Projects/key-switcher/switcher/PRD/` — требования, PRD, спек-дельты, задачи и отчёты;
`.../PRD/run/prd_01/dashboard.html` — ход работы.

Два правила, которые стоит знать до того, как что-то менять: снять требование
из `prd_XX_manifest.md` может только человек, от которого пришёл запрос, а спек-дельта
попадает в живую спеку не раньше команды `/prd archive NN`.

Работа прервалась на середине — фраза «продолжи PRD» поднимает состояние
из `run/<prd_XX>/state.js`; ничего пересказывать не нужно.
<!-- prd:end -->
