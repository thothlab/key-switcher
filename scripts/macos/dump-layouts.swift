#!/usr/bin/env swift
// Одноразовый дамп таблиц раскладок macOS через UCKeyTranslate в формат фикстур ks-core.
// Запуск из корня репозитория:
//   swift scripts/macos/dump-layouts.swift [каталог] [kbdType]
// kbdType по умолчанию 40 (ANSI): клавиши `§` (keycode 10) и `` ` `` (keycode 50) на ISO
// ведут себя иначе (analysis/detection.md §7, подвох 3), поэтому тип пишется в заголовок файла.
// Фикстура KBDRU (Windows) из этого дампа не снимается: её строит соседний шаг вручную по §7.
import Carbon
import Foundation

let keys: [(UInt16, String)] = [
    (0, "KeyA"), (1, "KeyS"), (2, "KeyD"), (3, "KeyF"), (4, "KeyH"), (5, "KeyG"), (6, "KeyZ"), (7, "KeyX"),
    (8, "KeyC"), (9, "KeyV"), (10, "IntlBackslash"), (11, "KeyB"), (12, "KeyQ"), (13, "KeyW"), (14, "KeyE"),
    (15, "KeyR"), (16, "KeyY"), (17, "KeyT"), (18, "Digit1"), (19, "Digit2"), (20, "Digit3"), (21, "Digit4"),
    (22, "Digit6"), (23, "Digit5"), (24, "Equal"), (25, "Digit9"), (26, "Digit7"), (27, "Minus"), (28, "Digit8"),
    (29, "Digit0"), (30, "BracketRight"), (31, "KeyO"), (32, "KeyU"), (33, "BracketLeft"), (34, "KeyI"),
    (35, "KeyP"), (37, "KeyL"), (38, "KeyJ"), (39, "Quote"), (40, "KeyK"), (41, "Semicolon"), (42, "Backslash"),
    (43, "Comma"), (44, "Slash"), (45, "KeyN"), (46, "KeyM"), (47, "Period"), (50, "Backquote"),
]

let layouts: [(String, String, String)] = [
    ("com.apple.keylayout.US", "en", "us.tsv"),
    ("com.apple.keylayout.Russian", "ru", "ru_apple.tsv"),
    ("com.apple.keylayout.RussianWin", "ru", "ru_pc.tsv"),
]

let args = CommandLine.arguments
let outDir = args.count > 1 ? args[1] : "crates/ks-core/tests/fixtures"
let kbdType: UInt32 = args.count > 2 ? UInt32(args[2])! : 40

let kind: String
switch Int(KBGetLayoutType(Int16(kbdType))) {
case Int(kKeyboardANSI): kind = "ANSI"
case Int(kKeyboardISO): kind = "ISO"
case Int(kKeyboardJIS): kind = "JIS"
default: kind = "unknown"
}

func inputSource(_ id: String) -> TISInputSource? {
    let filter = [kTISPropertyInputSourceID as String: id] as CFDictionary
    guard let list = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource] else {
        return nil
    }
    return list.first
}

func translate(_ layout: UnsafePointer<UCKeyboardLayout>, _ code: UInt16, _ modifiers: Int) -> String {
    var dead: UInt32 = 0
    var length = 0
    var chars = [UniChar](repeating: 0, count: 8)
    let state = UInt32((modifiers >> 8) & 0xFF)
    let status = UCKeyTranslate(layout, code, UInt16(kUCKeyActionDown), state, kbdType,
                                OptionBits(1 << kUCKeyTranslateNoDeadKeysBit), &dead, chars.count, &length, &chars)
    guard status == noErr, length > 0 else { return "" }
    let s = String(utf16CodeUnits: chars, count: length)
    guard s.unicodeScalars.count == 1, let scalar = s.unicodeScalars.first,
          scalar.value >= 0x20, scalar.value != 0x7F else { return "" }
    return s
}

let osVersion = ProcessInfo.processInfo.operatingSystemVersionString
for (id, lang, file) in layouts {
    guard let src = inputSource(id),
          let raw = TISGetInputSourceProperty(src, kTISPropertyUnicodeKeyLayoutData) else {
        FileHandle.standardError.write("нет раскладки \(id)\n".data(using: .utf8)!)
        exit(1)
    }
    let data = Unmanaged<CFData>.fromOpaque(raw).takeUnretainedValue()
    var out = "# KeySwitcher: фикстура раскладки, снята scripts/macos/dump-layouts.swift\n"
    out += "# источник: UCKeyTranslate, kbdType=\(kbdType) (\(kind)), LMGetKbdType машины=\(LMGetKbdType()), macOS \(osVersion)\n"
    out += "# колонки: код W3C, Base, Shift, Caps, Shift+Caps; пусто — символа нет\n"
    out += "@id\t\(id)\n@lang\t\(lang)\n@name\t\(id)\n"
    let bytes = CFDataGetBytePtr(data)!
    bytes.withMemoryRebound(to: UCKeyboardLayout.self, capacity: 1) { layout in
        for (code, w3c) in keys {
            let cols = [0, shiftKey, alphaLock, shiftKey | alphaLock].map { translate(layout, code, $0) }
            out += ([w3c] + cols).joined(separator: "\t") + "\n"
        }
    }
    try! out.write(toFile: "\(outDir)/\(file)", atomically: true, encoding: .utf8)
}
print("kbdType=\(kbdType) (\(kind)) -> \(outDir)")
