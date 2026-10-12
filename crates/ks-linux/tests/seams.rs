//! Швы бэкенда Linux, проверяемые на любой ОС: коды evdev, окружение по переменным среды,
//! команды и ответы IPC (Sway, Hyprland, KDE), права, поправки модели исполнения без захвата.
//! Ожидаемые значения — из `linux/input-event-codes.h`, протокола i3-ipc и примеров ответов.

use std::collections::HashMap;
use std::path::Path;

use ks_core::{
    Action, Disposition, KeyDir, KeyEvent, KeyStroke, Lang, LayoutId, LayoutMap, LayoutSet, Level,
    Modifiers, Outcome, PhysicalKey as K, ReplaceKind,
};
use ks_linux::desktop::{Desktop, Environment, Session, SwitchMethod};
use ks_linux::ipc::{hyprland, kde, sway};
use ks_linux::layouts::XkbCatalog;
use ks_linux::{autostart, keymap, nograb, perms, typing};
use ks_platform::PermissionKind;

#[test]
fn evdev_codes_map_to_physical_keys() {
    // linux/input-event-codes.h
    let cases = [
        (1, K::Escape),
        (28, K::Enter),
        (30, K::KeyA),
        (41, K::Backquote),
        (43, K::Backslash),
        (44, K::KeyZ),
        (57, K::Space),
        (86, K::IntlBackslash), // KEY_102ND
        (100, K::AltRight),
        (119, K::Pause),
        (125, K::MetaLeft),
        (126, K::MetaRight),
    ];
    for (code, key) in cases {
        assert_eq!(keymap::to_physical(code), key, "код {code}");
        assert_eq!(keymap::to_evdev(key), Some(code), "{key}");
    }
    assert_eq!(keymap::to_physical(0x2a0), K::Other(0x2a0));
    assert_eq!(keymap::to_evdev(K::Other(0x2a0)), Some(0x2a0));
}

fn env(vars: &[(&str, &str)]) -> Environment {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Environment::detect(|name| map.get(name).cloned())
}

#[test]
fn environment_is_detected_from_variables() {
    let x11 = env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "i3")]);
    assert_eq!(x11.session, Session::X11);
    assert_eq!(x11.switch, Some(SwitchMethod::XkbGroup));

    let kde = env(&[
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("DISPLAY", ":1"),
        ("XDG_CURRENT_DESKTOP", "KDE"),
    ]);
    assert_eq!(
        (kde.session, kde.desktop.clone()),
        (Session::Wayland, Desktop::Kde)
    );
    assert_eq!(kde.switch, Some(SwitchMethod::KdeDbus));
    assert!(!kde.knows_focus());

    let kde_x11 = env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "KDE")]);
    assert_eq!(kde_x11.switch, Some(SwitchMethod::KdeDbus));
    assert!(kde_x11.knows_focus());

    let sway = env(&[
        ("WAYLAND_DISPLAY", "wayland-1"),
        ("SWAYSOCK", "/run/user/1000/sway-ipc.1000.42.sock"),
    ]);
    assert_eq!(sway.switch, Some(SwitchMethod::SwayIpc));

    let hypr = env(&[
        ("WAYLAND_DISPLAY", "wayland-1"),
        ("HYPRLAND_INSTANCE_SIGNATURE", "abc_123"),
        ("XDG_CURRENT_DESKTOP", "Hyprland"),
    ]);
    assert_eq!(hypr.switch, Some(SwitchMethod::HyprlandIpc));

    let gnome = env(&[
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
    ]);
    assert_eq!(gnome.desktop, Desktop::Gnome);
    assert_eq!(gnome.switch, None);
    assert!(gnome.unsupported_reason().is_some());

    let gnome_x11 = env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "GNOME")]);
    assert_eq!(gnome_x11.switch, Some(SwitchMethod::XkbGroup));

    let mate = env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "MATE")]);
    assert_eq!(mate.switch, None);

    let labwc = env(&[
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("XDG_CURRENT_DESKTOP", "labwc:wlroots"),
    ]);
    assert_eq!(labwc.switch, None);

    let tty = env(&[]);
    assert_eq!((tty.session, tty.switch), (Session::Unknown, None));
}

#[test]
fn sway_switch_command_is_framed_as_i3_ipc() {
    let cmd = sway::switch_command(1);
    assert_eq!(cmd, "input type:keyboard xkb_switch_layout 1");
    let frame = sway::frame(sway::RUN_COMMAND, &cmd);
    let mut expected = b"i3-ipc".to_vec();
    expected.extend_from_slice(&[39, 0, 0, 0, 0, 0, 0, 0]);
    expected.extend_from_slice(cmd.as_bytes());
    assert_eq!(frame, expected);
    assert_eq!(sway::header(&frame[..14]), Some((39, 0)));
    assert_eq!(sway::header(b"i3-ipX\0\0\0\0\0\0\0\0"), None);
    assert!(sway::command_ok(r#"[{"success": true}]"#));
    assert!(!sway::command_ok(
        r#"[{"success": false, "error": "Unknown/invalid command"}]"#
    ));
}

#[test]
fn sway_inputs_and_tree_are_parsed() {
    let inputs = r#"[
      {"identifier": "0:0:KeySwitcher_virtual_keyboard", "name": "KeySwitcher virtual keyboard",
       "type": "keyboard", "xkb_layout_names": ["English (US)"], "xkb_active_layout_index": 0},
      {"identifier": "1:1:AT_Translated_Set_2_keyboard", "name": "AT Translated Set 2 keyboard",
       "type": "keyboard", "xkb_layout_names": ["English (US)", "Russian (Windows)"],
       "xkb_active_layout_index": 1, "xkb_active_layout_name": "Russian (Windows)"},
      {"identifier": "2:7:SynPS/2_Synaptics_TouchPad", "type": "touchpad"}
    ]"#;
    let (specs, active) = sway::keyboard_layouts(
        inputs,
        "KeySwitcher virtual keyboard",
        &XkbCatalog::builtin(),
    )
    .expect("клавиатура найдена");
    let ids: Vec<String> = specs.iter().map(|s| s.id().0).collect();
    assert_eq!(ids, ["us", "ru(winkeys)"]);
    assert_eq!(active, 1);

    let tree = r#"{"id": 1, "type": "root", "focused": false, "nodes": [
      {"id": 2, "type": "output", "focused": false, "nodes": [
        {"id": 3, "type": "con", "focused": false, "app_id": "foot", "nodes": []},
        {"id": 4, "type": "con", "focused": true, "app_id": "org.telegram.desktop", "nodes": []}
      ]}],
      "floating_nodes": []}"#;
    assert_eq!(
        sway::focused_app(tree).map(|f| f.0),
        Some("org.telegram.desktop".to_owned())
    );
    let xwayland = r#"{"focused": false, "nodes": [], "floating_nodes": [
      {"focused": true, "app_id": null, "window_properties": {"class": "Code"}}]}"#;
    assert_eq!(
        sway::focused_app(xwayland).map(|f| f.0),
        Some("Code".to_owned())
    );
}

#[test]
fn hyprland_commands_and_replies() {
    assert_eq!(hyprland::switch_command(1), "switchxkblayout all 1");
    assert!(hyprland::command_ok("ok\n"));
    assert!(!hyprland::command_ok("Invalid device"));
    let paths = hyprland::socket_paths(Some("/run/user/1000"), "sig");
    assert_eq!(paths[0], Path::new("/run/user/1000/hypr/sig/.socket.sock"));
    assert_eq!(paths[1], Path::new("/tmp/hypr/sig/.socket.sock"));

    let devices = r#"{"mice": [], "keyboards": [
      {"address": "0x1", "name": "power-button", "layout": "us", "variant": "",
       "active_keymap": "English (US)", "main": false},
      {"address": "0x2", "name": "at-translated-set-2-keyboard", "layout": "us,ru",
       "variant": ",winkeys", "options": "grp:alt_shift_toggle",
       "active_keymap": "Russian (Windows)", "main": true}
    ]}"#;
    let (specs, active) = hyprland::keyboard_layouts(
        devices,
        "KeySwitcher virtual keyboard",
        &XkbCatalog::builtin(),
    )
    .expect("клавиатура");
    let ids: Vec<String> = specs.iter().map(|s| s.id().0).collect();
    assert_eq!(ids, ["us", "ru(winkeys)"]);
    assert_eq!(active, "Russian (Windows)");
    assert_eq!(specs[1].description, "Russian (Windows)");

    let window = r#"{"address": "0x5", "class": "firefox", "title": "Новая вкладка"}"#;
    assert_eq!(
        hyprland::focused_app(window).map(|f| f.0),
        Some("firefox".to_owned())
    );
    assert_eq!(hyprland::focused_app("{}"), None);
}

#[test]
fn kde_layout_list_is_parsed_in_order() {
    let reply = vec![
        ("us".to_owned(), String::new(), "English (US)".to_owned()),
        (
            "ru".to_owned(),
            String::new(),
            "Russian (Windows)".to_owned(),
        ),
        (
            "ru".to_owned(),
            "ру".to_owned(),
            "Russian (какой-то вариант)".to_owned(),
        ),
    ];
    let specs = kde::layouts(&reply, &XkbCatalog::builtin());
    let ids: Vec<String> = specs.iter().map(|s| s.id().0).collect();
    assert_eq!(ids, ["us", "ru(winkeys)", "ru"]);
    assert_eq!(specs[2].description, "Russian (какой-то вариант)");
}

#[test]
fn no_access_gives_the_setup_command() {
    let fix = perms::fix_command(None);
    assert_eq!(fix, "sudo sh scripts/linux/install-udev-rule.sh");
    let statuses = perms::statuses(perms::Access::default(), &fix);
    let kinds: Vec<PermissionKind> = statuses.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            PermissionKind::LinuxInputDevices,
            PermissionKind::LinuxUinput
        ]
    );
    assert!(statuses.iter().all(|s| !s.granted && s.fix_hint == fix));

    let granted = perms::statuses(
        perms::Access {
            input: true,
            uinput: false,
        },
        &fix,
    );
    assert!(granted[0].granted && !granted[1].granted);

    let spaced = perms::fix_command(Some(Path::new("/opt/Key Switcher/install-udev-rule.sh")));
    assert_eq!(spaced, "sudo sh '/opt/Key Switcher/install-udev-rule.sh'");
}

fn layouts() -> LayoutSet {
    let mut us = LayoutMap::new(LayoutId("us".into()), Lang::En, "English (US)");
    let mut ru = LayoutMap::new(LayoutId("ru".into()), Lang::Ru, "Russian");
    for (key, en, ruc) in [
        (K::KeyG, 'g', 'п'),
        (K::KeyH, 'h', 'р'),
        (K::KeyB, 'b', 'и'),
    ] {
        us.insert(key, Level::Base, en);
        ru.insert(key, Level::Base, ruc);
    }
    us.insert(K::Space, Level::Base, ' ');
    ru.insert(K::Space, Level::Base, ' ');
    LayoutSet(vec![us, ru])
}

fn down(key: K, mods: Modifiers) -> KeyEvent {
    KeyEvent {
        key,
        dir: KeyDir::Down,
        mods,
        repeat: false,
        time_ms: 0,
    }
}

fn replace(erase: usize) -> Action {
    Action::Replace {
        erase,
        keys: vec![
            KeyStroke {
                key: K::KeyG,
                shift: false,
            },
            KeyStroke {
                key: K::Space,
                shift: false,
            },
        ],
        text: "п ".to_owned(),
        target_layout: Some(LayoutId("ru".into())),
        kind: ReplaceKind::Auto,
    }
}

fn erase_of(out: &Outcome) -> Option<usize> {
    out.actions.iter().find_map(|a| match a {
        Action::Replace { erase, .. } => Some(*erase),
        _ => None,
    })
}

#[test]
fn space_boundary_already_typed_is_erased_too() {
    let out = Outcome {
        disposition: Disposition::Swallow,
        actions: vec![replace(1)],
    };
    let fixed = nograb::adjust(out, &down(K::Space, Modifiers::default()), &layouts());
    assert_eq!(erase_of(&fixed.outcome), Some(2));
    assert!(!fixed.reset);
}

#[test]
fn enter_boundary_is_not_corrected() {
    let out = Outcome {
        disposition: Disposition::Swallow,
        actions: vec![replace(1)],
    };
    let fixed = nograb::adjust(out, &down(K::Enter, Modifiers::default()), &layouts());
    assert!(fixed.outcome.actions.is_empty());
}

#[test]
fn ctrl_z_reaching_the_app_cancels_engine_undo() {
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    let out = Outcome {
        disposition: Disposition::Swallow,
        actions: Vec::new(),
    };
    assert!(nograb::adjust(out, &down(K::KeyZ, ctrl), &layouts()).reset);
    let shifted = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    };
    let out = Outcome {
        disposition: Disposition::Pass,
        actions: Vec::new(),
    };
    assert!(!nograb::adjust(out, &down(K::KeyZ, shifted), &layouts()).reset);
}

#[test]
fn letter_typed_before_layout_switch_is_retyped() {
    let out = Outcome {
        disposition: Disposition::Pass,
        actions: vec![Action::SwitchLayout(LayoutId("ru".into()))],
    };
    let fixed = nograb::adjust(out, &down(K::KeyH, Modifiers::default()), &layouts());
    match fixed.outcome.actions.as_slice() {
        [
            Action::Replace {
                erase,
                keys,
                text,
                target_layout,
                ..
            },
        ] => {
            assert_eq!(*erase, 1);
            assert_eq!(
                keys,
                &[KeyStroke {
                    key: K::KeyH,
                    shift: false
                }]
            );
            assert_eq!(text, "р");
            assert_eq!(target_layout, &Some(LayoutId("ru".into())));
        }
        _ => panic!("ожидалась замена одной буквы"),
    }
}

#[test]
fn text_falls_back_to_a_layout_that_types_it() {
    let set = layouts();
    let strokes =
        typing::strokes_for("пи", set.get(&LayoutId("us".into())), &set).expect("набирается");
    assert_eq!(
        strokes,
        [
            KeyStroke {
                key: K::KeyG,
                shift: false
            },
            KeyStroke {
                key: K::KeyB,
                shift: false
            }
        ]
    );
    match typing::selection_action("пир", &set) {
        Some(Action::Replace {
            erase: 0,
            target_layout: Some(id),
            ..
        }) => assert_eq!(id.0, "ru"),
        _ => panic!("ожидался ввод поверх выделения в русской раскладке"),
    }
}

#[test]
fn autostart_entry_quotes_exec_path() {
    assert_eq!(
        autostart::exec_value(Path::new("/home/u/My Apps/keyswitcher")),
        "\"/home/u/My Apps/keyswitcher\""
    );
    assert_eq!(
        autostart::exec_value(Path::new("/usr/bin/keyswitcher")),
        "/usr/bin/keyswitcher"
    );
    let entry = autostart::entry(Path::new("/usr/bin/keyswitcher"));
    assert!(entry.contains("\nExec=/usr/bin/keyswitcher\n"));
    assert!(autostart::is_enabled(&entry));
    assert!(!autostart::is_enabled("[Desktop Entry]\nHidden=true\n"));
    assert_eq!(
        autostart::dir(None, Some("/home/u")).as_deref(),
        Some(Path::new("/home/u/.config/autostart"))
    );
}
