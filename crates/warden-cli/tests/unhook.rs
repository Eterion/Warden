//! The uninstaller's command, against made-up folders only, including while the server still holds its lock.
#[path = "../../warden-server/tests/common/mod.rs"]
mod common;
use common::{Fixture, call, write};
use serde_json::json;
use std::{path::Path, process::Command};
fn run(f: &Fixture) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_warden"))
        .arg("unhook")
        .arg("--data-dir")
        .arg(f.data.path())
        .env("USERPROFILE", f.home.path())
        .env("HOME", f.home.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .output()
        .expect("unhook")
}
fn add(f: &Fixture, addr: std::net::SocketAddr, provider: &str, folder: &Path) {
    let reply = call(
        addr,
        "POST",
        "/api/accounts",
        Some(&json!({"provider":provider,"folder":folder,"label":"Work"}).to_string()),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    // The CLI fixture's login is never valid for a network request, even if a command were dispatched wrongly.
    assert!(f.endpoint.tokens_seen.lock().expect("lock").is_empty());
}
#[test]
fn uninstall_restores_main_and_all_warden_files_and_leaves_protected_folders_alone() {
    let f = Fixture::new();
    let main = f.home.path().join(".claude");
    let work = f.claude_folder(".claude-work", "work@example.com", "unused");
    write(
        &work.join(".credentials.json"),
        "invalid fixture login; never a usable token",
    );
    write(
        &main.join(".credentials.json"),
        "private fixture login; never a usable token",
    );
    let codex = f.codex_folder(".codex-work", "codex@example.com", "fixture");
    let private = f.codex_folder(".codex", "private@example.com", "fixture-private");
    write(&private.join("hooks.json"), r#"{"hooks":{"Stop":[]}}"#);
    write(
        &main.join("settings.json"),
        "{\r\n  \"model\": \"main-fixture\"\r\n}\r\n",
    );
    write(&work.join("settings.json"), "{\"model\":\"work-fixture\"}");
    write(&codex.join("config.toml"), "model = 'fixture'\r\n");
    let before = common::folder_bytes(f.home.path());
    let server = f.start();
    add(&f, server.addr, "claude", &work);
    add(&f, server.addr, "codex", &codex);
    let repair = call(server.addr, "POST", "/api/accounts/main/hooks/repair", None);
    assert_eq!(repair.status, 200, "{}", repair.body);
    assert!(f.data.path().join("hooks/main.json").exists());
    assert!(warden_core::store::server_running(f.data.path()));
    let result = run(&f);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(common::folder_bytes(f.home.path()), before);
    for id in ["main", "a1", "a2"] {
        assert!(!f.data.path().join(format!("hooks/{id}.json")).exists());
    }
    common::eventually("the live server reports the removed hooks", || {
        let s = call(server.addr, "GET", "/api/state", None).json();
        s["main"]["hooks"] == "notInstalled"
            && s["accounts"]
                .as_array()
                .expect("accounts")
                .iter()
                .all(|a| a["hooks"] == "notInstalled")
    });
    assert!(run(&f).status.success(), "idempotent while running");
    server.stop();
    assert!(run(&f).status.success(), "idempotent after exit");
    assert_eq!(common::folder_bytes(f.home.path()), before);
}
#[test]
fn unsupported_settings_are_left_intact_and_uninstall_reports_failure() {
    let f = Fixture::new();
    write(
        &f.data.path().join("settings.json"),
        r#"{"version":3,"accounts":[]}"#,
    );
    let before = common::folder_bytes(f.home.path());
    let result = run(&f);
    assert!(!result.status.success());
    assert_eq!(
        std::fs::read_to_string(f.data.path().join("settings.json")).expect("kept"),
        r#"{"version":3,"accounts":[]}"#
    );
    assert_eq!(common::folder_bytes(f.home.path()), before);
}
#[test]
fn main_is_cleaned_even_when_settings_are_unsupported_or_broken() {
    use warden_core::{
        hooks::{self, HookTool, Want},
        model::Provider,
    };
    for bytes in ["{broken", r#"{"version":3,"accounts":[]}"#] {
        let f = Fixture::new();
        let main = f.home.path().join(".claude");
        write(
            &main.join("settings.json"),
            "{\r\n  \"model\": \"fixture\"\r\n}\r\n",
        );
        let before = common::folder_bytes(f.home.path());
        hooks::sync(
            Provider::Claude,
            &main,
            "main",
            f.data.path(),
            &Want {
                tool: Some(HookTool {
                    exe: f.route_exe.clone(),
                    data_dir: f.data.path().into(),
                    #[cfg(debug_assertions)]
                    home: None,
                }),
                route: true,
                status: true,
            },
        )
        .expect("install fixture hooks");
        write(&f.data.path().join("settings.json"), bytes);
        assert!(
            !run(&f).status.success(),
            "warn once through the uninstaller"
        );
        assert_eq!(common::folder_bytes(f.home.path()), before);
        assert_eq!(
            std::fs::read_to_string(f.data.path().join("settings.json")).expect("kept"),
            bytes
        );
    }
}

#[test]
fn version_one_folder_list_is_used_for_uninstall_without_opening_codex_app() {
    use warden_core::{
        hooks::{self, HookTool, Want},
        model::Provider,
    };
    let f = Fixture::new();
    let work = f.claude_folder(".claude-old", "work@example.com", "fixture");
    let private = f.codex_folder(".codex", "private@example.com", "fixture-private");
    let before = common::folder_bytes(f.home.path());
    hooks::sync(
        Provider::Claude,
        &work,
        "a12",
        f.data.path(),
        &Want {
            tool: Some(HookTool {
                exe: f.route_exe.clone(),
                data_dir: f.data.path().into(),
                #[cfg(debug_assertions)]
                home: None,
            }),
            route: true,
            status: true,
        },
    )
    .expect("old hooks");
    write(
        &f.data.path().join("settings.json"),
        &json!({"version":1,"accounts":[
            {"id":"a12","provider":"claude","folder":work},
            {"id":"a13","provider":"codex","folder":private}
        ]})
        .to_string(),
    );
    assert!(run(&f).status.success());
    assert_eq!(common::folder_bytes(f.home.path()), before);
}
#[test]
fn installer_runs_unhook_before_removing_files_and_keeps_hooks_on_update() {
    let hooks = include_str!("../../warden-desktop/installer-hooks.nsh");
    let pre = hooks
        .split("!macro NSIS_HOOK_PREUNINSTALL")
        .nth(1)
        .expect("before uninstall")
        .split("!macroend")
        .next()
        .expect("macro");
    assert!(pre.contains("unhook --data-dir \"$APPDATA\\Warden\""));
    let config: serde_json::Value = serde_json::from_str(include_str!(
        "../../warden-desktop/tauri.installer.conf.json"
    ))
    .expect("installer config");
    let destination = config["bundle"]["resources"]
        .as_object()
        .expect("resources")
        .iter()
        .find(|(source, _)| source.ends_with("/warden.exe"))
        .expect("CLI resource")
        .1
        .as_str()
        .expect("destination");
    assert!(pre.contains(&format!("$INSTDIR\\{}", destination.replace('/', "\\"))));
    assert!(pre.contains("$UpdateMode != 1"));
    assert!(!pre.contains("Abort"));
    assert_eq!(pre.matches("MessageBox").count(), 1);
    assert!(pre.contains("MB_OK|MB_ICONEXCLAMATION"));
    assert!(pre.contains("/SD IDOK"));
    // An update keeps Start with Windows too: only an uninstall removes it.
    let post = hooks
        .split("!macro NSIS_HOOK_POSTUNINSTALL")
        .nth(1)
        .expect("after uninstall")
        .split("!macroend")
        .next()
        .expect("macro");
    let guard = post.find("${If} $UpdateMode != 1").expect("update guard");
    let removal = post
        .find(
            "DeleteRegValue HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Run\" \"Warden\"",
        )
        .expect("removes the Run value");
    assert!(guard < removal && post[removal..].contains("${EndIf}"));
}

/// A development build's `--home` points the uninstaller's command at a test home: main and the saved accounts there
/// are cleaned, and the main folder of the home the process would otherwise use is never touched.
#[cfg(debug_assertions)]
#[test]
fn unhook_with_a_test_home_cleans_it_and_leaves_the_process_home_alone() {
    use warden_core::{
        hooks::{self, HookTool, Want},
        model::Provider,
        testdir::Dir,
    };
    // The test home: main and a saved Warden account, both with Warden's hooks, put in by the server.
    let f = Fixture::new();
    let main = f.home.path().join(".claude");
    let work = f.claude_folder(".claude-work", "work@example.com", "unused");
    // Logins that are never usable for a request: adding the account reads no usage with them.
    write(
        &work.join(".credentials.json"),
        "invalid fixture login; never a usable token",
    );
    write(
        &main.join(".credentials.json"),
        "private fixture login; never a usable token",
    );
    write(
        &main.join("settings.json"),
        "{
  \"model\": \"main-fixture\"
}
",
    );
    write(&work.join("settings.json"), "{\"model\":\"work-fixture\"}");
    let test_home_clean = common::folder_bytes(f.home.path());
    let server = f.start();
    add(&f, server.addr, "claude", &work);
    let repair = call(server.addr, "POST", "/api/accounts/main/hooks/repair", None);
    assert_eq!(repair.status, 200, "{}", repair.body);
    server.stop();
    assert_ne!(common::folder_bytes(f.home.path()), test_home_clean);

    // The process home: its own main with Warden's hooks, from another data folder.
    let process_home = Dir::new("process-home");
    let other_data = Dir::new("other-data");
    let process_main = process_home.path().join(".claude");
    write(
        &process_main.join("settings.json"),
        "{
  \"model\": \"fixture\"
}
",
    );
    let process_clean = common::folder_bytes(process_home.path());
    hooks::sync(
        Provider::Claude,
        &process_main,
        "main",
        other_data.path(),
        &Want {
            tool: Some(HookTool {
                exe: f.route_exe.clone(),
                data_dir: other_data.path().into(),
                home: None,
            }),
            route: true,
            status: true,
        },
    )
    .expect("install fixture hooks");
    let process_hooked = common::folder_bytes(process_home.path());
    assert_ne!(process_hooked, process_clean);

    let unhook = |data_dir: &Path, test_home: Option<&Path>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_warden"));
        cmd.arg("unhook").arg("--data-dir").arg(data_dir);
        if let Some(home) = test_home {
            cmd.arg("--home").arg(home);
        }
        cmd.env("USERPROFILE", process_home.path())
            .env("HOME", process_home.path())
            .env("CLAUDE_CONFIG_DIR", &process_main)
            .env_remove("CODEX_HOME")
            .output()
            .expect("unhook")
    };
    let result = unhook(f.data.path(), Some(f.home.path()));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(common::folder_bytes(f.home.path()), test_home_clean);
    assert_eq!(common::folder_bytes(process_home.path()), process_hooked);
    for id in ["main", "a1"] {
        assert!(!f.data.path().join(format!("hooks/{id}.json")).exists());
    }
    // Without --home, the process home's main is the one cleaned.
    assert!(unhook(other_data.path(), None).status.success());
    assert_eq!(common::folder_bytes(process_home.path()), process_clean);
}
