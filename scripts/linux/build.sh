#!/bin/sh
# Сборка KeySwitcher для Linux x86_64 с macOS или Linux через cargo-zigbuild:
#   brew install zig && cargo install cargo-zigbuild   (один раз)
#   sh scripts/linux/build.sh
# Результат: dist/linux/keyswitcher и рядом dist/linux/install-udev-rule.sh — приложение ищет
# скрипт рядом с собой и показывает команду настройки прав в разделе «Разрешения».
# KS_GLIBC — минимальная версия glibc целевой системы (по умолчанию 2.31: Ubuntu 20.04+).
set -eu

cd "$(dirname "$0")/../.."
GLIBC="${KS_GLIBC:-2.31}"
TARGET=x86_64-unknown-linux-gnu
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

command -v cargo-zigbuild >/dev/null 2>&1 || {
    echo "нужен cargo-zigbuild: brew install zig && cargo install cargo-zigbuild" >&2
    exit 1
}
cargo zigbuild --release -p ks-app --target "$TARGET.$GLIBC"
mkdir -p dist/linux
cp "$TARGET_DIR/$TARGET/release/keyswitcher" dist/linux/keyswitcher
cp scripts/linux/install-udev-rule.sh dist/linux/install-udev-rule.sh
echo "dist/linux/keyswitcher (glibc >= $GLIBC)"
