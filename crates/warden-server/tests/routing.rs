//! Fixed-main delegation and switch-driven hooks through the replacement API; throwaway folders only.
mod common;
use common::{Fixture, call, eventually, write};
use serde_json::{Value, json};
use std::{net::SocketAddr, path::Path};
fn add(addr: SocketAddr, provider: &str, folder: &Path) -> Value {
    let reply = call(
        addr,
        "POST",
        "/api/accounts",
        Some(&json!({"provider":provider,"folder":folder,"label":"Work"}).to_string()),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()
}
fn patch(addr: SocketAddr, id: &str, on: bool) -> Value {
    let reply = call(
        addr,
        "PATCH",
        &format!("/api/accounts/{id}"),
        Some(&json!({"enabled":on}).to_string()),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()
}
#[test]
fn a_switch_installs_only_status_and_restores_each_providers_original_files() {
    for provider in ["claude", "codex"] {
        let f = Fixture::new();
        let folder = if provider == "claude" {
            f.claude_folder(".claude-work", "work@example.com", "fake")
        } else {
            f.codex_folder(".codex-work", "work@example.com", "fake")
        };
        if provider == "claude" {
            write(
                &folder.join("settings.json"),
                "{\n  \"model\": \"example\"\n}",
            );
        } else {
            write(&folder.join("config.toml"), "model = 'example'\r\n");
        }
        let before = common::folder_bytes(&folder);
        let server = f.start();
        let addr = server.addr;
        let s = add(addr, provider, &folder);
        assert_eq!(s["accounts"][0]["hooks"], "installed");
        let files = common::folder_bytes(&folder);
        let text = files
            .values()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("event"));
        assert!(!text.contains("route --account"));
        assert!(!text.contains("\"route\""));
        assert_eq!(
            patch(addr, "a1", false)["accounts"][0]["hooks"],
            "notInstalled"
        );
        assert_eq!(common::folder_bytes(&folder), before);
        assert_eq!(patch(addr, "a1", true)["accounts"][0]["hooks"], "installed");
        assert_eq!(call(addr, "DELETE", "/api/accounts/a1", None).status, 200);
        assert_eq!(common::folder_bytes(&folder), before);
        server.stop();
    }
}
#[test]
fn setup_installs_both_main_hooks_and_skip_installs_nothing() {
    for skip in [true, false] {
        let f = Fixture::new();
        let work = f.claude_folder(".claude-work", "work@example.com", "fake");
        let before = common::folder_bytes(f.home.path());
        let server = f.start();
        let body = if skip {
            json!({"accounts":[],"skip":true})
        } else {
            json!({"accounts":[{"provider":"claude","folder":work,"label":"Work"}]})
        };
        let reply = call(server.addr, "POST", "/api/setup", Some(&body.to_string()));
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert_eq!(reply.json()["main"]["id"], "main");
        assert_eq!(reply.json()["setupDone"], true);
        if skip {
            assert_eq!(common::folder_bytes(f.home.path()), before);
        } else {
            let text = std::fs::read_to_string(f.home.path().join(".claude/settings.json"))
                .expect("main hooks");
            assert!(text.contains("\"route\""));
            assert!(text.contains("\"event\""));
            assert_eq!(reply.json()["main"]["hooks"], "installed");
        }
        server.stop();
    }
}
#[test]
fn setup_refuses_a_signed_out_or_missing_claude_app_before_adding_accounts() {
    for missing in [true, false] {
        let f = Fixture::new();
        let work = f.claude_folder(".claude-work", "work@example.com", "fake");
        if missing {
            std::fs::remove_dir_all(f.home.path().join("AppData/Local/AnthropicClaude"))
                .expect("remove fake app");
        } else {
            std::fs::remove_file(f.home.path().join(".claude.json")).expect("sign out fake app");
        }
        let server = f.start();
        let body = json!({"accounts":[{"provider":"claude","folder":work,"label":"Work"}]});
        assert_eq!(
            call(server.addr, "POST", "/api/setup", Some(&body.to_string())).status,
            409
        );
        let s = call(server.addr, "GET", "/api/state", None).json();
        assert_eq!(s["setupDone"], false);
        assert_eq!(s["accounts"], json!([]));
        server.stop();
    }
}
#[test]
fn missing_and_untrusted_status_hooks_show_repair_and_repair_rewrites_them() {
    for provider in ["claude", "codex"] {
        let f = Fixture::new();
        let folder = if provider == "claude" {
            f.claude_folder(".claude-work", "work@example.com", "fake")
        } else {
            f.codex_folder(".codex-work", "work@example.com", "fake")
        };
        let server = f.start();
        add(server.addr, provider, &folder);
        if provider == "claude" {
            write(&folder.join("settings.json"), "{}");
        } else {
            write(&folder.join("config.toml"), "model = 'example'\n");
        }
        eventually("missing or untrusted hook", || {
            call(server.addr, "GET", "/api/state", None).json()["accounts"][0]["hooks"]
                != "installed"
        });
        let reply = call(server.addr, "POST", "/api/accounts/a1/hooks/repair", None);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert_eq!(reply.json()["accounts"][0]["hooks"], "installed");
        server.stop();
    }
}
#[test]
fn same_as_main_and_signed_out_accounts_get_no_delegated_work() {
    let f = Fixture::new();
    let same = f.claude_folder(".claude-same", "fixture-main@example.com", "fake");
    let codex = f.codex_folder(".codex-work", "other@example.com", "fake");
    let server = f.start();
    add(server.addr, "claude", &same);
    add(server.addr, "codex", &codex);
    call(server.addr, "POST", "/api/usage/refresh", None);
    let s = call(server.addr, "GET", "/api/state", None).json();
    assert_eq!(s["accounts"][0]["sameAsMain"], true);
    assert_eq!(s["delegation"]["delegated"]["accountId"], "a2");
    std::fs::remove_file(codex.join("auth.json")).expect("fake sign out");
    eventually("signed out gets no work", || {
        call(server.addr, "GET", "/api/state", None).json()["delegation"]["delegated"].is_null()
    });
    assert_eq!(
        call(server.addr, "GET", "/api/state", None).json()["delegation"]["mainWorksItself"],
        true
    );
    server.stop();
}
#[test]
fn old_cli_is_a_problem_and_repair_writes_nothing_until_it_is_updated() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "work@example.com", "fake");
    let bin = f.home.path().join("bin");
    write(&bin.join("claude.cmd"), "@echo off\r\n");
    write(
        &bin.join("node_modules/@anthropic-ai/claude-code/package.json"),
        r#"{"version":"2.1.100"}"#,
    );
    let before = common::folder_bytes(&folder);
    let server = f.start();
    let s = add(server.addr, "claude", &folder);
    assert!(
        s["problems"]
            .as_array()
            .expect("problems")
            .iter()
            .any(|p| p["kind"] == "cliTooOld" && p["version"] == "2.1.100")
    );
    assert_eq!(
        call(server.addr, "POST", "/api/accounts/a1/hooks/repair", None).status,
        409
    );
    assert_eq!(common::folder_bytes(&folder), before);
    write(
        &bin.join("node_modules/@anthropic-ai/claude-code/package.json"),
        r#"{"version":"2.1.139"}"#,
    );
    call(server.addr, "POST", "/api/tools/scan", None);
    assert_eq!(
        call(server.addr, "POST", "/api/accounts/a1/hooks/repair", None).json()["accounts"][0]["hooks"],
        "installed"
    );
    server.stop();
}

#[test]
fn startup_rewrites_old_main_hooks_and_removes_legacy_routing_and_disabled_hooks() {
    use warden_core::{
        hooks::{self, HookTool, Want},
        model::Provider,
        store,
    };
    let f = Fixture::new();
    let main = f.home.path().join(".claude");
    let work = f.claude_folder(".claude-work", "work@example.com", "fixture");
    let off = f.codex_folder(".codex-off", "off@example.com", "fixture");
    write(&main.join("settings.json"), "{\"model\":\"fixture-main\"}");
    write(&work.join("settings.json"), "{\"model\":\"fixture-work\"}");
    write(&off.join("config.toml"), "model='fixture-off'\r\n");
    let off_before = common::folder_bytes(&off);
    let server = f.start();
    add(server.addr, "claude", &work);
    add(server.addr, "codex", &off);
    server.stop();
    let mut settings: store::Settings =
        store::load(f.data.path(), store::SETTINGS_FILE).expect("settings");
    settings.accounts[1].enabled = false;
    store::save(f.data.path(), store::SETTINGS_FILE, &settings).expect("off settings");
    let old = Want {
        tool: Some(HookTool {
            #[cfg(debug_assertions)]
            home: None,
            exe: f.home.path().join("old/warden.exe"),
            data_dir: f.data.path().to_owned(),
        }),
        route: true,
        status: true,
    };
    for (provider, folder, id) in [
        (Provider::Claude, &main, "main"),
        (Provider::Claude, &work, "a1"),
        (Provider::Codex, &off, "a2"),
    ] {
        hooks::sync(provider, folder, id, f.data.path(), &old).expect("old fixture hooks");
    }
    let server = f.start();
    let s = call(server.addr, "GET", "/api/state", None).json();
    assert_eq!(s["main"]["hooks"], "installed");
    assert_eq!(s["accounts"][0]["hooks"], "installed");
    assert_eq!(s["accounts"][1]["hooks"], "notInstalled");
    let main_text = std::fs::read_to_string(main.join("settings.json")).expect("main");
    let work_text = std::fs::read_to_string(work.join("settings.json")).expect("work");
    assert!(main_text.contains("\"route\""));
    assert!(!work_text.contains("\"route\""));
    assert!(work_text.contains("\"event\""));
    assert!(!main_text.contains("old/warden.exe"));
    assert!(!work_text.contains("old/warden.exe"));
    assert_eq!(common::folder_bytes(&off), off_before);
    assert!(s.get("hooksUpdated").is_none());
    assert!(f.log_text().contains("updated Warden's hooks"));
    server.stop();
}
