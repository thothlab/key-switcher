# egui, eframe

Окно настроек рисуется egui/eframe (MIT OR Apache-2.0). Запасными шрифтами после Golos Text
стоят шрифты egui по умолчанию (крейт `epaint_default_fonts` 0.36, лицензия
«(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0»). Значков ⌘ ⌥ ⇧ ⌃ и стрелок «↔» в них нет,
поэтому последними запасными окно подключает системные шрифты символов, читая их с диска при
запуске (macOS — Apple Symbols, Windows — Segoe UI Symbol, Linux — DejaVu Sans); в бинарник они
не входят.

| Шрифт | Лицензия |
|---|---|
| Ubuntu-Light | Ubuntu Font Licence 1.0 |
| Hack-Regular | MIT (Source Foundry Authors, 2018); часть DejaVu — общественное достояние |
| NotoEmoji-Regular | SIL Open Font License 1.1 |
| emoji-icon-font | MIT (John Slegers, 2014) |

Полные тексты лицензий лежат рядом со шрифтами в крейте `epaint_default_fonts`
(`fonts/UFL.txt`, `fonts/Hack-Regular.txt`, `fonts/OFL.txt`, `fonts/emoji-icon-font-mit-license.txt`).
