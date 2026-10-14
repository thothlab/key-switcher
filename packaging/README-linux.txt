KeySwitcher — автопереключатель раскладки RU↔EN

Установка
  sudo sh install-udev-rule.sh   # один раз: доступ к клавиатуре и /dev/uinput
  # перелогиньтесь, затем
  ./keyswitcher

Удобно положить keyswitcher и install-udev-rule.sh в ~/.local/bin/.
Иконка приложения: cp -r icons/hicolor ~/.local/share/icons/
Откат правила: sudo sh install-udev-rule.sh --uninstall
Автозапуск — флажок в настройках (меню значка → «Настройки…»); на Sway и Hyprland без
XDG autostart добавьте exec keyswitcher (Sway) или exec-once = keyswitcher (Hyprland).

Окружения: X11, KDE Plasma, Sway, Hyprland. GNOME на Wayland не поддерживается.

Горячие клавиши
  двойной Shift или Pause — отменить исправление / сменить раскладку последнего слова
  Shift + Pause           — сменить раскладку выделенного текста (X11)
  оба Shift               — включить/выключить автоисправление
  Ctrl + Z                — сразу после исправления вернуть как было

Настройки: ~/.config/keyswitcher/config.toml
Состояние: ./keyswitcher --status

Лицензии словарей и заимствованного кода — в папке licenses.
https://github.com/thothlab/key-switcher
