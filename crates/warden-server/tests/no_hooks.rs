//! Development no-hooks runs leave fixture account folders byte-identical and create no backup ledger.
#![cfg(debug_assertions)]

mod common;

use common::{Fixture, write};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use warden_core::model::Provider;
use warden_server::app::App;

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(at).expect("directory") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if entry.file_type().expect("type").is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).expect("relative").to_owned(),
                    std::fs::read(path).expect("file"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn setup_switches_remove_and_repairs_write_no_hooks_or_backups() {
    let f = Fixture::new();
    let first = f.claude_folder(".claude-first", "first@example.com", "fixture-first");
    let other = f.codex_folder(".codex-other", "other@example.com", "fixture-other");
    write(
        &first.join("settings.json"),
        r#"{ "permissions": { "allow": [] } }"#,
    );
    write(&other.join("config.toml"), "model = 'example'\n");
    let before = tree(f.home.path());
    let mut env = f.env();
    env.no_hooks = true;
    let app = App::new(
        f.data.path().to_owned(),
        env,
        (
            f.endpoint.clone(),
            f.sign_in_server.clone(),
            f.resets.clone(),
        ),
        Arc::new(|_, _, _, _| panic!("no sign-in")),
        Arc::new(|_| {}),
        Some(f.route_exe.clone()),
        None,
    )
    .expect("app");
    app.finish_setup(
        &[
            (
                Provider::Claude,
                first.to_string_lossy().into_owned(),
                "First".into(),
            ),
            (
                Provider::Codex,
                other.to_string_lossy().into_owned(),
                "Other".into(),
            ),
        ],
        false,
    )
    .expect("setup");
    app.repair_account_hooks("main").expect("main hooks");
    app.repair_account_hooks("a2").expect("status hook");
    app.update_account("a2", None, Some(false)).expect("off");
    app.update_account("a2", None, Some(true)).expect("on");
    app.repair_account_hooks("main").expect("repair");
    app.repair_hooks();
    app.remove_account("a2").expect("remove");
    assert_eq!(tree(f.home.path()), before);
    assert!(!f.data.path().join("hooks").exists());
    let state = serde_json::to_value(app.state()).expect("state");
    assert_eq!(state["hooksOff"], true);
}
