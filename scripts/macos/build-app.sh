#!/bin/sh
# Сборка и подпись dist/KeySwitcher.app (агент в строке меню, без иконки в Dock).
#   scripts/macos/build-app.sh                                   — arm64 (хост)
#   KS_UNIVERSAL=1 scripts/macos/build-app.sh                    — universal: arm64 + x86_64 (lipo)
#   KS_SIGN_IDENTITY="KeySwitcher Local" scripts/macos/build-app.sh — стабильная подпись
# Без KS_SIGN_IDENTITY подпись ad-hoc: права TCC слетают после каждой пересборки.
set -eu
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

TARGET_DIR="${CARGO_TARGET_DIR:-target}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BUNDLE_ID="com.keyswitcher.app"
APP="dist/KeySwitcher.app"

if [ "${KS_UNIVERSAL:-0}" = 1 ]; then
    cargo build --release -p ks-app --target aarch64-apple-darwin
    cargo build --release -p ks-app --target x86_64-apple-darwin
    BIN="$TARGET_DIR/universal/keyswitcher"
    mkdir -p "$(dirname "$BIN")"
    lipo -create -output "$BIN" \
        "$TARGET_DIR/aarch64-apple-darwin/release/keyswitcher" \
        "$TARGET_DIR/x86_64-apple-darwin/release/keyswitcher"
else
    cargo build --release -p ks-app
    BIN="$TARGET_DIR/release/keyswitcher"
fi

# Бандл перезаписывается на месте: бинарник, Info.plist, подпись.
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp -f "$BIN" "$APP/Contents/MacOS/keyswitcher"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>KeySwitcher</string>
    <key>CFBundleDisplayName</key><string>KeySwitcher</string>
    <key>CFBundleExecutable</key><string>keyswitcher</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>KeySwitcher — автопереключатель раскладки RU↔EN. Словари: hunspell (LibreOffice), см. THIRD_PARTY.md.</string>
</dict>
</plist>
PLIST

if [ -n "${KS_SIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --identifier "$BUNDLE_ID" --sign "$KS_SIGN_IDENTITY" "$APP"
else
    echo "ВНИМАНИЕ: KS_SIGN_IDENTITY не задана — подпись ad-hoc. После каждой пересборки права" >&2
    echo "«Мониторинг ввода» и «Универсальный доступ» придётся выдать заново (docs/build-macos.md)." >&2
    codesign --force --options runtime --identifier "$BUNDLE_ID" --sign - "$APP"
fi
codesign --verify --strict "$APP"
echo "собрано: $APP ($VERSION)"
