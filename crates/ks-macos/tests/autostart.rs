//! app-shell «Автозапуск»: запись `~/Library/LaunchAgents/com.keyswitcher.app.plist` ставится и
//! убирается; путь бандла определяется по исполняемому файлу.

use std::path::Path;

use ks_macos::bundle;

fn temp_home(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ks-autostart-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn launch_agent_is_created_kept_and_removed() {
    let home = temp_home("cycle");
    let file = bundle::launch_agent_path(&home);
    assert_eq!(
        file,
        home.join("Library/LaunchAgents/com.keyswitcher.app.plist")
    );
    let exe = Path::new("/Applications/Key & Switcher.app/Contents/MacOS/keyswitcher");

    assert!(
        bundle::apply_launch_agent(&file, exe, true).unwrap(),
        "создан"
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("<string>com.keyswitcher.app</string>"));
    assert!(text.contains("/Applications/Key &amp; Switcher.app/Contents/MacOS/keyswitcher"));
    assert!(text.contains("<key>RunAtLoad</key><true/>"));
    assert!(
        !text.contains("KeepAlive"),
        "«Выход» не должен перезапускать агент"
    );

    assert!(
        !bundle::apply_launch_agent(&file, exe, true).unwrap(),
        "то же содержимое не переписывается"
    );
    assert!(
        bundle::apply_launch_agent(&file, exe, false).unwrap(),
        "убран"
    );
    assert!(!file.exists());
    assert!(
        !bundle::apply_launch_agent(&file, exe, false).unwrap(),
        "убирать нечего"
    );
}

#[test]
fn bundle_is_found_only_for_an_executable_inside_an_app() {
    assert_eq!(
        bundle::bundle_of(Path::new(
            "/x/dist/KeySwitcher.app/Contents/MacOS/keyswitcher"
        )),
        Some(Path::new("/x/dist/KeySwitcher.app").to_path_buf())
    );
    assert_eq!(
        bundle::bundle_of(Path::new("/x/target/release/keyswitcher")),
        None
    );
}
