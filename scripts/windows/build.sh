#!/bin/sh
# Кросс-сборка keyswitcher.exe (Windows x86_64, GNU ABI) с macOS или Linux через mingw-w64.
# Результат: dist/windows/keyswitcher.exe. Учитывает CARGO_TARGET_DIR, если он задан.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TARGET=x86_64-pc-windows-gnu

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "нет линковщика x86_64-w64-mingw32-gcc: поставьте mingw-w64 (brew install mingw-w64)" >&2
    exit 1
fi
if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "нет цели $TARGET: rustup target add $TARGET" >&2
    exit 1
fi

cd "$ROOT"
cargo build --release --target "$TARGET" -p ks-app

TARGET_DIR=${CARGO_TARGET_DIR:-target}
case "$TARGET_DIR" in
    /*) ;;
    *) TARGET_DIR="$ROOT/$TARGET_DIR" ;;
esac

mkdir -p dist/windows
cp "$TARGET_DIR/$TARGET/release/keyswitcher.exe" dist/windows/keyswitcher.exe
echo "собрано: dist/windows/keyswitcher.exe ($(wc -c < dist/windows/keyswitcher.exe | tr -d ' ') байт)"
