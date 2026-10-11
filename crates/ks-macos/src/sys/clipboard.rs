//! Буфер обмена macOS (`NSPasteboard`) для конвертации выделения: снимок всех элементов со
//! всеми типами и восстановление после вставки. Содержимое не логируется.

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};

use ks_core::AppId;
use ks_platform::exec::{Clipboard, ClipboardSnapshot};

/// Здесь вставка через буфер обмена не работает (`analysis/platforms.md` §1.4): выделение не
/// трогается, даже если автоисправление в приложении включено.
const NO_PASTE: [&str; 2] = ["com.apple.Terminal", "com.googlecode.iterm2"];

pub struct MacClipboard;

impl Clipboard for MacClipboard {
    fn usable_in(&self, app: &AppId) -> bool {
        !NO_PASTE.contains(&app.0.as_str())
    }

    fn change_count(&self) -> i64 {
        autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount() as i64)
    }

    fn snapshot(&self) -> ClipboardSnapshot {
        autoreleasepool(|_| {
            let items = NSPasteboard::generalPasteboard()
                .pasteboardItems()
                .map(|a| a.to_vec())
                .unwrap_or_default();
            ClipboardSnapshot(
                items
                    .iter()
                    .map(|item| {
                        item.types()
                            .iter()
                            .filter_map(|t| {
                                item.dataForType(&t).map(|d| (t.to_string(), d.to_vec()))
                            })
                            .collect()
                    })
                    .collect(),
            )
        })
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) {
        autoreleasepool(|_| {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            if snapshot.0.is_empty() {
                return;
            }
            let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
                .0
                .iter()
                .map(|types| {
                    let item = NSPasteboardItem::new();
                    for (t, data) in types {
                        item.setData_forType(&NSData::with_bytes(data), &NSString::from_str(t));
                    }
                    ProtocolObject::from_retained(item)
                })
                .collect();
            if !pb.writeObjects(&NSArray::from_retained_slice(&items)) {
                log::warn!("буфер обмена не восстановлен после конвертации выделения");
            }
        });
    }

    fn text(&self) -> Option<String> {
        autoreleasepool(|_| {
            NSPasteboard::generalPasteboard()
                .stringForType(unsafe { NSPasteboardTypeString })
                .map(|s| s.to_string())
        })
    }

    fn set_text(&self, text: &str) {
        autoreleasepool(|_| {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
        });
    }
}
