#!/bin/sh
# KeySwitcher: разовая настройка доступа к клавиатурам и мыши (/dev/input/event*) и к
# виртуальной клавиатуре (/dev/uinput) для пользователя активной локальной сессии.
#
#   sudo sh install-udev-rule.sh              установить
#   sudo sh install-udev-rule.sh --uninstall  откатить
#
# Что делает:
#   1. пишет /etc/udev/rules.d/70-keyswitcher.rules: тег uaccess — systemd-logind выдаёт ACL
#      только пользователю активной локальной сессии (не всем процессам, как группа input).
#      Номер 70 < 73: тег должен стоять до 73-seat-late.rules, где ACL применяются;
#   2. пишет /etc/modules-load.d/keyswitcher-uinput.conf: модуль uinput грузится при старте;
#   3. загружает uinput сейчас и применяет правила к уже подключённым устройствам.
#
# Откат (--uninstall): удаляет оба файла и перечитывает правила. ACL снимаются при следующем
# событии устройства или после перезагрузки; модуль uinput остаётся загруженным до перезагрузки.
set -eu

RULES=/etc/udev/rules.d/70-keyswitcher.rules
MODULES=/etc/modules-load.d/keyswitcher-uinput.conf

if [ "$(id -u)" -ne 0 ]; then
    echo "Нужны права root: sudo sh $0 ${1:-}" >&2
    exit 1
fi

reload() {
    udevadm control --reload-rules
    udevadm trigger --action=change --subsystem-match=input --subsystem-match=misc
}

case "${1:-}" in
    --uninstall)
        rm -f "$RULES" "$MODULES"
        reload
        echo "KeySwitcher: правило udev удалено."
        exit 0
        ;;
    "")
        ;;
    *)
        echo "использование: sudo sh $0 [--uninstall]" >&2
        exit 2
        ;;
esac

cat > "$RULES" <<'RULES'
# KeySwitcher: доступ активной локальной сессии к клавиатурам, мыши, тачпаду и uinput.
# Удалить: sudo sh install-udev-rule.sh --uninstall
KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"
SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_KEYBOARD}=="1", TAG+="uaccess"
SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_MOUSE}=="1", TAG+="uaccess"
SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_TOUCHPAD}=="1", TAG+="uaccess"
RULES

echo uinput > "$MODULES"
modprobe uinput || echo "KeySwitcher: модуль uinput не загружен (возможно, он встроен в ядро)" >&2
reload
echo "KeySwitcher: правило установлено ($RULES). Если KeySwitcher уже запущен, перезапустите его."
