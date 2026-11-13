//! Review regressions on made-up account trees and fake endpoints only.
mod common;
use common::{Fixture, folder_bytes, write};
use serde_json::json;
use std::sync::{Arc, Barrier};
use warden_core::{
    hooks::{self, HookTool, Want},
    model::Provider,
};
use warden_server::app::App;

fn app(f: &Fixture) -> Arc<App> {
    Arc::new(
        App::new(
            f.data.path().into(),
            f.env(),
            (f.endpoint.clone(), f.sign_in_server.clone()),
            Arc::new(|_, _, _, _| Ok(())),
            Arc::new(|_| {}),
            Some(f.route_exe.clone()),
            None,
        )
        .expect("app"),
    )
}
fn want(f: &Fixture) -> Want {
    Want {
        tool: Some(HookTool {
            exe: f.route_exe.clone(),
            data_dir: f.data.path().into(),
            #[cfg(debug_assertions)]
            home: Some(f.home.path().into()),
        }),
        route: true,
        status: true,
    }
}

#[test]
fn version_one_cleanup_runs_once_and_reset_account_ids_are_never_reused() {
    let f = Fixture::new();
    let work = f.claude_folder(".claude-old", "old@example.com", "fixture");
    let codex = f.codex_folder(".codex-old", "old@example.com", "fixture");
    let private = f.codex_folder(".codex", "private@example.com", "fixture-private");
    let original_work = folder_bytes(&work);
    let original_codex = folder_bytes(&codex);
    for (provider, folder, id) in [
        (Provider::Claude, &work, "a1"),
        (Provider::Codex, &codex, "a2"),
        (Provider::Codex, &private, "a3"),
    ] {
        hooks::sync(provider, folder, id, f.data.path(), &want(&f)).expect("old fixture hooks");
    }
    let private_before = folder_bytes(&private);
    let old = json!({"version":1,"nextId":4,"accounts":[
        {"id":"a1","provider":"claude","folder":work}, {"id":"a2","provider":"codex","folder":codex},
        {"id":"a3","provider":"codex","folder":private}
    ]}).to_string();
    write(&f.data.path().join("settings.json"), &old);
    let a = app(&f);
    assert_eq!(folder_bytes(&work), original_work);
    assert_eq!(folder_bytes(&codex), original_codex);
    assert_eq!(folder_bytes(&private), private_before);
    let id = a
        .add_existing(Provider::Claude, work.to_str().expect("path"), "Work")
        .expect("add");
    assert!(!["a1", "a2", "a3"].contains(&id.as_str()));
    let installed = folder_bytes(&work);
    drop(a);
    let a = app(&f);
    assert_eq!(
        folder_bytes(&work),
        installed,
        "old cleanup must not remove new hooks on restart"
    );
    assert_eq!(
        std::fs::read_to_string(f.data.path().join("settings.json.bad")).expect("archive"),
        old
    );
    drop(a);
    write(&f.data.path().join("settings.json"), "{broken");
    let a = app(&f);
    let next = a
        .add_existing(Provider::Codex, codex.to_str().expect("path"), "New")
        .expect("add after reset");
    assert_ne!(next, id);
    assert!(!["a1", "a2", "a3"].contains(&next.as_str()));
    assert_eq!(folder_bytes(&private), private_before);
}

#[test]
fn start_installs_main_without_warden_accounts_and_skip_allows_a_signed_out_app() {
    for skip in [false, true] {
        let f = Fixture::new();
        if skip {
            std::fs::remove_file(f.home.path().join(".claude.json")).expect("sign out fake app");
        }
        let a = app(&f);
        a.finish_setup(&[], skip).expect("finish");
        let s = serde_json::to_value(a.state()).expect("state");
        assert_eq!(s["setupDone"], true);
        assert_eq!(
            s["main"]["hooks"],
            if skip { "notInstalled" } else { "installed" }
        );
        if skip {
            let work = f.codex_folder(".codex-work", "work@example.com", "fixture");
            a.add_existing(Provider::Codex, work.to_str().expect("path"), "Work")
                .expect("add first");
            assert_eq!(
                serde_json::to_value(a.state()).expect("state")["main"]["hooks"],
                "installed"
            );
        }
    }
}

#[test]
fn concurrent_switches_repair_and_removal_keep_hooks_in_the_current_state() {
    let f = Fixture::new();
    let work = f.claude_folder(".claude-work", "work@example.com", "fixture");
    let before = folder_bytes(&work);
    let a = app(&f);
    let id = a
        .add_existing(Provider::Claude, work.to_str().expect("path"), "Work")
        .expect("add");
    for _ in 0..12 {
        let barrier = Arc::new(Barrier::new(3));
        std::thread::scope(|scope| {
            for on in [false, true] {
                let a = a.clone();
                let barrier = barrier.clone();
                let id = &id;
                scope.spawn(move || {
                    barrier.wait();
                    a.update_account(id, None, Some(on)).expect("switch");
                });
            }
            barrier.wait();
        });
        let s = serde_json::to_value(a.state()).expect("state");
        assert_eq!(
            s["accounts"][0]["hooks"],
            if s["accounts"][0]["enabled"] == true {
                "installed"
            } else {
                "notInstalled"
            }
        );
    }
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let barrier1 = barrier.clone();
        let a1 = a.clone();
        let id1 = &id;
        scope.spawn(move || {
            barrier1.wait();
            let _ = a1.repair_account_hooks(id1);
        });
        let barrier2 = barrier.clone();
        let a2 = a.clone();
        let id2 = &id;
        scope.spawn(move || {
            barrier2.wait();
            a2.remove_account(id2).expect("remove");
        });
        barrier.wait();
    });
    assert_eq!(folder_bytes(&work), before);
    assert_eq!(
        serde_json::to_value(a.state()).expect("state")["accounts"],
        json!([])
    );
}
