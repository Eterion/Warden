use super::*;
use crate::testdir::Dir;

#[cfg(debug_assertions)]
#[test]
fn debug_routing_handlers_pass_the_fixture_home_but_status_events_do_not_need_it() {
    let tool = HookTool {
        exe: "fixture/warden.exe".into(),
        data_dir: "fixture/data".into(),
        home: Some("fixture/home with spaces".into()),
    };
    for h in handlers(Provider::Claude, &tool, "main") {
        let args = h.args.expect("exec form");
        assert_eq!(
            &args[args.len() - 2..],
            &["--home", "fixture/home with spaces"]
        );
    }
    assert!(
        !status_handler(Provider::Claude, &tool, "main")
            .args
            .expect("exec form")
            .contains(&"--home".into())
    );
}

struct Fx {
    _dir: Dir,
    folder: PathBuf,
    hook: HookTool,
}

fn fx(tag: &str) -> Fx {
    let dir = Dir::new(tag);
    let folder = dir.path().join("account");
    fs::create_dir_all(&folder).expect("mkdir");
    let hook = HookTool {
        #[cfg(debug_assertions)]
        home: None,
        exe: PathBuf::from("/opt/warden/warden.exe"),
        data_dir: dir.path().join("data"),
    };
    Fx {
        _dir: dir,
        folder,
        hook,
    }
}

/// The routing hook alone.
fn install(provider: Provider, folder: &Path, id: &str, hook: &HookTool) -> io::Result<()> {
    let want = Want {
        tool: Some(hook.clone()),
        route: true,
        status: false,
    };
    sync(provider, folder, id, &hook.data_dir, &want)
}

fn want(f: &Fx, route: bool, status: bool) -> Want {
    Want {
        tool: Some(f.hook.clone()),
        route,
        status,
    }
}

fn read(p: &Path) -> Option<Vec<u8>> {
    fs::read(p).ok()
}

/// The status hook as earlier builds wrote it.
const OLD_CURL: &str = r#"curl.exe -q -s -m 1 --noproxy 127.0.0.1 -H "Content-Type: application/json" --data-binary "@-" "http://127.0.0.1:4777/api/events?account=a1""#;

const CLAUDE_SETTINGS_FIXTURE: &str = "{\r\n  \"model\": \"opus\",\r\n  \"hooks\": {\r\n    \"UserPromptSubmit\": [\r\n      {\r\n        \"hooks\": [\r\n          {\r\n            \"type\": \"command\",\r\n            \"command\": \"echo mine\"\r\n          }\r\n        ]\r\n      }\r\n    ]\r\n  },\r\n  \"zeta\": 1\r\n}";

#[test]
fn the_hooks_run_warden_directly_for_the_account() {
    let tool = HookTool {
        #[cfg(debug_assertions)]
        home: None,
        exe: PathBuf::from("%USERPROFILE%/AppData/Local/Programs/My Warden/warden.exe"),
        data_dir: PathBuf::from("%USERPROFILE%/AppData/Roaming/Ann's $HOME %TEMP% Warden"),
    };
    // Claude: the exec form, the program and its arguments as they are; no shell, so no quoting.
    let [route, refresh] = handlers(Provider::Claude, &tool, "a2");
    assert_eq!(
        route.command,
        "%USERPROFILE%/AppData/Local/Programs/My Warden/warden.exe"
    );
    let data = tool.data_dir.to_string_lossy().into_owned();
    assert_eq!(
        route.args,
        Some(vec![
            "route".into(),
            "--account".into(),
            "a2".into(),
            "--data-dir".into(),
            data.clone()
        ])
    );
    assert_eq!((route.timeout, route.is_async), (ROUTE_TIMEOUT_SEC, false));
    assert_eq!(
        refresh.args.as_deref().map(|a| &a[..2]),
        Some(&["refresh".to_owned(), "--if-stale".to_owned()][..])
    );
    assert_eq!(
        (refresh.timeout, refresh.is_async),
        (REFRESH_TIMEOUT_SEC, true)
    );
    let status = status_handler(Provider::Claude, &tool, "a2");
    assert_eq!(
        status.args,
        Some(vec![
            "event".into(),
            "--account".into(),
            "a2".into(),
            "--data-dir".into(),
            data
        ])
    );
    assert_eq!(
        (status.timeout, status.is_async),
        (STATUS_TIMEOUT_SEC, true)
    );
    for h in [&route, &refresh] {
        assert!(is_route(&h.line()) && !is_status(&h.line()), "{}", h.line());
    }
    assert!(is_status(&status.line()) && !is_route(&status.line()));

    // Codex: one line for cmd.exe /C, with forward slashes; plain paths as they are, others in double quotes.
    let plain_tool = HookTool {
        #[cfg(debug_assertions)]
        home: None,
        exe: PathBuf::from("/opt/warden/warden.exe"),
        data_dir: PathBuf::from("/var/warden"),
    };
    assert_eq!(
        handlers(Provider::Codex, &plain_tool, "a2")[0].command,
        "/opt/warden/warden.exe route --account a2 --data-dir /var/warden"
    );
    assert_eq!(handlers(Provider::Codex, &plain_tool, "a2")[0].args, None);
    let spaced = HookTool {
        #[cfg(debug_assertions)]
        home: None,
        exe: PathBuf::from("/opt/my tools/warden.exe"),
        data_dir: PathBuf::from("/var/warden"),
    };
    assert_eq!(
        status_handler(Provider::Codex, &spaced, "a2").command,
        r#""/opt/my tools/warden.exe" event --account a2 --data-dir /var/warden"#
    );
    assert_eq!(
        handlers(Provider::Codex, &spaced, "a2")[1].command,
        r#""/opt/my tools/warden.exe" refresh --if-stale --account a2 --data-dir /var/warden"#
    );
    assert!(!is_ours("echo route --account"));
    assert!(!is_ours("echo event --account a1"));
}

#[test]
fn codex_routing_hook_is_refused_for_a_path_cmd_would_expand() {
    let mut f = fx("codex-percent");
    f.hook.data_dir = f.hook.data_dir.join("100%TEMP%");
    let err = install(Provider::Codex, &f.folder, "a2", &f.hook).expect_err("refused");
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(read(&f.folder.join(CODEX_HOOKS)), None, "nothing written");
    assert_eq!(read(&f.folder.join(CODEX_CONFIG)), None, "nothing written");
    // Codex's status hook runs the same warden.exe with the same folder: refused too.
    sync(
        Provider::Codex,
        &f.folder,
        "a2",
        &f.hook.data_dir,
        &want(&f, false, true),
    )
    .expect_err("status refused");
    // Claude runs warden.exe with no shell, so it takes the path as it is.
    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("Claude");
}

#[test]
fn claude_install_reinstall_and_removal_leave_the_file_byte_for_byte() {
    let f = fx("claude-hooks");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, CLAUDE_SETTINGS_FIXTURE).expect("write");
    let before = read(&path);
    assert_eq!(
        setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true)).routing,
        Setup::NotInstalled
    );

    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("install");
    let installed = fs::read_to_string(&path).expect("read");
    assert_eq!(
        setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true)).routing,
        Setup::Installed
    );
    let v: Value = serde_json::from_str(&installed).expect("json");
    let groups = v["hooks"][ROUTE_EVENT].as_array().expect("groups");
    assert_eq!(
        groups.len(),
        2,
        "the user's own hook stays, Warden's is added after it"
    );
    assert_eq!(groups[0]["hooks"][0]["command"], "echo mine");
    let data = f.hook.data_dir.to_string_lossy().into_owned();
    let route = &groups[1]["hooks"][0];
    assert_eq!(route["type"], "command");
    assert_eq!(route["command"], "/opt/warden/warden.exe");
    assert_eq!(
        route["args"],
        serde_json::json!(["route", "--account", "a1", "--data-dir", data])
    );
    assert_eq!(route["timeout"], 10);
    assert!(route.get("async").is_none());
    assert!(route.get("shell").is_none(), "the exec form needs no shell");
    let refresh = &groups[1]["hooks"][1];
    assert_eq!(refresh["command"], "/opt/warden/warden.exe");
    assert_eq!(
        refresh["args"],
        serde_json::json!([
            "refresh",
            "--if-stale",
            "--account",
            "a1",
            "--data-dir",
            data
        ])
    );
    assert_eq!(refresh["async"], true);
    assert_eq!(refresh["timeout"], 60);
    assert!(installed.contains("\r\n"), "keeps the file's line endings");
    let keys: Vec<&String> = v.as_object().expect("object").keys().collect();
    assert_eq!(keys, ["model", "hooks", "zeta"], "keeps the order of keys");

    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("reinstall");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        installed,
        "reinstall changes nothing"
    );

    remove(Provider::Claude, &f.folder, "a1", &f.hook.data_dir).expect("remove");
    assert_eq!(read(&path), before);
    assert_eq!(
        setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true)).routing,
        Setup::NotInstalled
    );
    assert!(
        !ledger_path(&f.hook.data_dir, "a1").exists(),
        "nothing left behind"
    );
}

#[test]
fn a_file_warden_created_is_deleted_again() {
    let f = fx("claude-new");
    let path = f.folder.join(CLAUDE_SETTINGS);
    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("install");
    assert!(path.is_file());
    remove(Provider::Claude, &f.folder, "a1", &f.hook.data_dir).expect("remove");
    assert!(!path.exists());
}

#[test]
fn a_file_changed_after_install_keeps_the_changes_and_loses_only_the_hook() {
    let f = fx("claude-edited");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, "{\n  \"model\": \"opus\"\n}\n").expect("write");
    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("install");
    let mut v: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
    v["theme"] = Value::from("dark");
    fs::write(
        &path,
        serde_json::to_string_pretty(&v).expect("json") + "\n",
    )
    .expect("write");
    remove(Provider::Claude, &f.folder, "a1", &f.hook.data_dir).expect("remove");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "{\n  \"model\": \"opus\",\n  \"theme\": \"dark\"\n}\n"
    );
}

#[test]
fn a_broken_file_is_left_alone() {
    let f = fx("claude-broken");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, "{ not json").expect("write");
    assert!(install(Provider::Claude, &f.folder, "a1", &f.hook).is_err());
    assert_eq!(fs::read_to_string(&path).expect("read"), "{ not json");
}

#[test]
fn the_codex_hash_is_over_the_normalised_identity() {
    // The exact bytes Codex hashes (sorted keys, compact), so this pins the format.
    let identity = serde_json::json!({
        "event_name": "user_prompt_submit",
        "hooks": [{ "async": false, "command": "x", "timeout": 10, "type": "command" }],
    })
    .to_string();
    assert_eq!(
        identity,
        r#"{"event_name":"user_prompt_submit","hooks":[{"async":false,"command":"x","timeout":10,"type":"command"}]}"#
    );
    let hash = codex_trusted_hash(
        ROUTE_EVENT,
        &Handler {
            command: "x".into(),
            args: None,
            timeout: 10,
            is_async: false,
        },
    );
    let expected: String = Sha256::digest(identity.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hash, format!("sha256:{expected}"));
}

const CODEX_CONFIG_FIXTURE: &str = "model = \"gpt-5\"\n\n[projects.'C:\\<work>']\ntrust_level = \"trusted\"\n\n[hooks.state.'C:\\<elsewhere>\\hooks.json:stop:0:0']\nenabled = false\n";
const CODEX_HOOKS_FIXTURE: &str = "{\n  \"hooks\": {\n    \"UserPromptSubmit\": [\n      {\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"mine.cmd\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";

#[test]
fn codex_install_trusts_the_hook_and_removal_restores_both_files() {
    let f = fx("codex-hooks");
    let config = f.folder.join(CODEX_CONFIG);
    let hooks = f.folder.join(CODEX_HOOKS);
    fs::write(&config, CODEX_CONFIG_FIXTURE).expect("write");
    fs::write(&hooks, CODEX_HOOKS_FIXTURE).expect("write");
    let (config_before, hooks_before) = (read(&config), read(&hooks));

    install(Provider::Codex, &f.folder, "a3", &f.hook).expect("install");
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true)).routing,
        Setup::Installed
    );
    let text = fs::read_to_string(&config).expect("read");
    let table: toml::Table = text.parse().expect("still valid TOML");
    let state = table["hooks"]["state"].as_table().expect("hooks.state");
    let path = hooks.to_string_lossy().to_string();
    // The hook is the second group (index 1): the user's own group keeps index 0 and its trust.
    for (hi, h) in handlers(Provider::Codex, &f.hook, "a3").iter().enumerate() {
        let hash = codex_trusted_hash(ROUTE_EVENT, h);
        for key in [
            format!("{}:user_prompt_submit:1:{hi}", path.replace('/', "\\")),
            format!("{}:user_prompt_submit:1:{hi}", path.replace('\\', "/")),
        ] {
            let entry = state.get(&key).unwrap_or_else(|| panic!("{key} in {text}"));
            assert_eq!(entry["trusted_hash"].as_str(), Some(hash.as_str()));
            assert_eq!(entry["enabled"].as_bool(), Some(true));
        }
    }
    assert_eq!(state.len(), 5, "the user's own entry and two per handler");
    assert!(
        text.starts_with(CODEX_CONFIG_FIXTURE),
        "the user's config stays as it was, above"
    );
    assert_eq!(
        table["projects"]["C:\\<work>"]["trust_level"].as_str(),
        Some("trusted")
    );

    install(Provider::Codex, &f.folder, "a3", &f.hook).expect("reinstall");
    assert_eq!(fs::read_to_string(&config).expect("read"), text);

    remove(Provider::Codex, &f.folder, "a3", &f.hook.data_dir).expect("remove");
    assert_eq!(read(&config), config_before);
    assert_eq!(read(&hooks), hooks_before);
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true)).routing,
        Setup::NotInstalled
    );
}

#[test]
fn codex_without_trust_shows_not_trusted_and_reinstall_adds_it() {
    let f = fx("codex-untrusted");
    let config = f.folder.join(CODEX_CONFIG);
    install(Provider::Codex, &f.folder, "a3", &f.hook).expect("install");
    // Someone (or Codex) drops the trust entries.
    fs::write(&config, "model = \"gpt-5\"\n").expect("write");
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true)).routing,
        Setup::NotTrusted
    );
    install(Provider::Codex, &f.folder, "a3", &f.hook).expect("reinstall");
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true)).routing,
        Setup::Installed
    );
    let text = fs::read_to_string(&config).expect("read");
    assert!(
        text.starts_with("model = \"gpt-5\"\n\n[hooks.state."),
        "{text}"
    );
    // A changed hash (another command) is not trusted either.
    let other = HookTool {
        #[cfg(debug_assertions)]
        home: None,
        exe: PathBuf::from("/opt/other/warden.exe"),
        data_dir: f.hook.data_dir.clone(),
    };
    let hooks = f.folder.join(CODEX_HOOKS);
    let changed = fs::read_to_string(&hooks)
        .expect("read")
        .replace("/opt/warden/warden.exe", &slashed(&other.exe));
    fs::write(&hooks, changed).expect("write");
    let ours_now = Want {
        tool: Some(other),
        route: true,
        status: false,
    };
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &ours_now).routing,
        Setup::NotTrusted
    );
    // And to the Warden that wrote the old command, it is not in place at all.
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true)).routing,
        Setup::NotInstalled
    );
    // Removal after outside changes takes out only Warden's parts.
    remove(Provider::Codex, &f.folder, "a3", &f.hook.data_dir).expect("remove");
    assert_eq!(
        fs::read_to_string(&config).expect("read"),
        "model = \"gpt-5\"\n"
    );
    assert!(!fs::read_to_string(&hooks).expect("read").contains("warden"));
}

#[test]
fn trust_blocks_are_found_in_either_quoting() {
    assert_eq!(
        header_key("[hooks.state.'C:\\<a>\\hooks.json:x:0:0']").as_deref(),
        Some("C:\\<a>\\hooks.json:x:0:0")
    );
    assert_eq!(
        header_key("[ hooks . state . \"C:\\\\<a>:x\" ]").as_deref(),
        Some("C:\\<a>:x")
    );
    assert_eq!(header_key("[hooks.other.'x']"), None);
    let text = "a = 1\n\n[hooks.state.'k']\nenabled = true\n\n[other]\nb = 2\n";
    assert_eq!(
        strip_trust(text, &["k".to_owned()]).as_deref(),
        Some("a = 1\n\n[other]\nb = 2\n")
    );
    assert_eq!(strip_trust(text, &["z".to_owned()]), None);
}

#[test]
fn the_backup_is_saved_before_the_file_is_written_and_kept_until_it_is_restored() {
    let f = fx("claude-backup");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, CLAUDE_SETTINGS_FIXTURE).expect("write");
    // The file cannot be replaced (read-only): the write fails, the backup stays.
    let lock = |on: bool| {
        let mut perm = fs::metadata(&path).expect("meta").permissions();
        perm.set_readonly(on);
        fs::set_permissions(&path, perm).expect("chmod");
    };
    lock(true);
    assert!(install(Provider::Claude, &f.folder, "a1", &f.hook).is_err());
    let ledger = load_ledger(&f.hook.data_dir, "a1").expect("ledger");
    assert_eq!(ledger.files.len(), 1);
    assert_eq!(
        ledger.files[0].original.as_deref(),
        Some(CLAUDE_SETTINGS_FIXTURE)
    );
    lock(false);

    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("install");
    lock(true);
    assert!(remove(Provider::Claude, &f.folder, "a1", &f.hook.data_dir).is_err());
    assert_eq!(
        load_ledger(&f.hook.data_dir, "a1")
            .expect("ledger")
            .files
            .len(),
        1,
        "kept after a failed restore"
    );
    lock(false);
    remove(Provider::Claude, &f.folder, "a1", &f.hook.data_dir).expect("remove");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        CLAUDE_SETTINGS_FIXTURE
    );
}

#[test]
fn codex_event_names_are_snake_case() {
    assert_eq!(event_label("UserPromptSubmit"), "user_prompt_submit");
    assert_eq!(event_label("PreToolUse"), "pre_tool_use");
    assert_eq!(event_label("Stop"), "stop");
}

#[test]
fn the_status_hook_runs_warden_event_in_the_background_on_every_event() {
    let f = fx("claude-status");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, CLAUDE_SETTINGS_FIXTURE).expect("write");
    sync(
        Provider::Claude,
        &f.folder,
        "a1",
        &f.hook.data_dir,
        &want(&f, false, true),
    )
    .expect("install");
    let s = setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true));
    assert_eq!(s.status, Setup::Installed);
    assert_eq!(s.routing, Setup::NotInstalled);
    let v: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
    let events = v["hooks"].as_object().expect("hooks");
    assert_eq!(events.len(), CLAUDE_STATUS_EVENTS.len());
    for event in CLAUDE_STATUS_EVENTS {
        let groups = v["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("{event}"));
        let h = &groups.last().expect("group")["hooks"][0];
        assert_eq!(h["command"], "/opt/warden/warden.exe");
        assert_eq!(
            h["args"],
            serde_json::json!(["event", "--account", "a1", "--data-dir", f.hook.data_dir])
        );
        assert_eq!(h["async"], true, "{event} does not hold up the turn");
        assert_eq!(h["timeout"], STATUS_TIMEOUT_SEC);
        assert!(groups.last().expect("group").get("matcher").is_none());
    }
    assert_eq!(
        v["hooks"][ROUTE_EVENT][0]["hooks"][0]["command"], "echo mine",
        "the user's own group keeps its place"
    );

    // A missing event (an older install, or edited by hand) reads as not installed.
    let mut partial = v.clone();
    partial["hooks"]
        .as_object_mut()
        .expect("hooks")
        .remove("SubagentStop");
    fs::write(&path, serde_json::to_string_pretty(&partial).expect("json")).expect("write");
    assert_eq!(
        setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true)).status,
        Setup::NotInstalled
    );
    sync(
        Provider::Claude,
        &f.folder,
        "a1",
        &f.hook.data_dir,
        &want(&f, false, true),
    )
    .expect("reinstall");
    assert_eq!(
        setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true)).status,
        Setup::Installed
    );

    sync(
        Provider::Claude,
        &f.folder,
        "a1",
        &f.hook.data_dir,
        &want(&f, false, false),
    )
    .expect("remove");
    assert!(!fs::read_to_string(&path).expect("read").contains("warden"));
    assert_eq!(
        setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true)).status,
        Setup::NotInstalled
    );
}

#[test]
fn either_hook_comes_and_goes_without_touching_the_other_and_both_gone_is_byte_for_byte() {
    for (first, second) in [
        ((true, false), (false, true)),
        ((false, true), (true, false)),
    ] {
        let f = fx("claude-both");
        let path = f.folder.join(CLAUDE_SETTINGS);
        fs::write(&path, CLAUDE_SETTINGS_FIXTURE).expect("write");
        let before = read(&path);
        let go = |route: bool, status: bool| {
            sync(
                Provider::Claude,
                &f.folder,
                "a1",
                &f.hook.data_dir,
                &want(&f, route, status),
            )
            .expect("sync")
        };
        go(first.0, first.1);
        go(true, true);
        let s = setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true));
        assert_eq!((s.routing, s.status), (Setup::Installed, Setup::Installed));
        go(second.0, second.1);
        let s = setup(Provider::Claude, &f.folder, "a1", &want(&f, true, true));
        let on = |b: bool| {
            if b {
                Setup::Installed
            } else {
                Setup::NotInstalled
            }
        };
        assert_eq!((s.routing, s.status), (on(second.0), on(second.1)));
        go(false, false);
        assert_eq!(read(&path), before);
        assert!(!ledger_path(&f.hook.data_dir, "a1").exists());
    }
}

#[test]
fn codex_trusts_every_status_handler_and_keeps_the_routing_trust_when_it_goes() {
    let f = fx("codex-status");
    let config = f.folder.join(CODEX_CONFIG);
    let hooks = f.folder.join(CODEX_HOOKS);
    fs::write(&config, CODEX_CONFIG_FIXTURE).expect("write");
    fs::write(&hooks, CODEX_HOOKS_FIXTURE).expect("write");
    let (config_before, hooks_before) = (read(&config), read(&hooks));
    let go = |route: bool, status: bool| {
        sync(
            Provider::Codex,
            &f.folder,
            "a3",
            &f.hook.data_dir,
            &want(&f, route, status),
        )
        .expect("sync")
    };

    go(true, true);
    let s = setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true));
    assert_eq!((s.routing, s.status), (Setup::Installed, Setup::Installed));
    let v: Value = serde_json::from_str(&fs::read_to_string(&hooks).expect("read")).expect("json");
    assert!(
        v["hooks"].get("SessionEnd").is_none(),
        "Codex has no session end"
    );
    let text = fs::read_to_string(&config).expect("read");
    let table: toml::Table = text.parse().expect("still valid TOML");
    let state = table["hooks"]["state"].as_table().expect("hooks.state");
    let path = hooks.to_string_lossy().replace('/', "\\");
    // On UserPromptSubmit the user's group is 0, routing 1 and the status hook 2; elsewhere the status hook is 0.
    let status = status_handler(Provider::Codex, &f.hook, "a3");
    for event in CODEX_STATUS_EVENTS {
        let group = if event == ROUTE_EVENT { 2 } else { 0 };
        let key = format!("{path}:{}:{group}:0", event_label(event));
        let entry = state.get(&key).unwrap_or_else(|| panic!("{key} in {text}"));
        assert_eq!(
            entry["trusted_hash"].as_str(),
            Some(codex_trusted_hash(event, &status).as_str())
        );
    }

    // Trust dropped for one status handler: the status hook is not trusted, routing still is.
    let key = format!("{path}:stop:0:0");
    let dropped = strip_trust(&text, &[key.clone(), key.replace('\\', "/")]).expect("strip");
    fs::write(&config, dropped).expect("write");
    let s = setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true));
    assert_eq!((s.routing, s.status), (Setup::Installed, Setup::NotTrusted));

    go(true, false);
    let s = setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true));
    assert_eq!(
        (s.routing, s.status),
        (Setup::Installed, Setup::NotInstalled)
    );
    assert!(
        !fs::read_to_string(&config)
            .expect("read")
            .contains(":session_start:")
    );
    assert!(
        !fs::read_to_string(&hooks)
            .expect("read")
            .contains(" event ")
    );

    go(false, true);
    go(false, false);
    // config.toml was changed by hand above, so only Warden's entries come out of it; hooks.json is as it was.
    assert_eq!(read(&hooks), hooks_before);
    assert_eq!(read(&config), config_before);
}

#[test]
fn a_hook_left_by_another_warden_reads_as_not_installed_and_outdated() {
    let f = fx("claude-stale");
    sync(
        Provider::Claude,
        &f.folder,
        "a1",
        &f.hook.data_dir,
        &want(&f, true, true),
    )
    .expect("install");
    let ours = want(&f, true, true);
    let s = setup(Provider::Claude, &f.folder, "a1", &ours);
    assert_eq!((s.routing, s.status), (Setup::Installed, Setup::Installed));

    assert_eq!(
        outdated(Provider::Claude, &f.folder, "a1", &ours),
        (false, false)
    );

    // Warden now runs another warden.exe (the desktop app after the browser version).
    let moved = Want {
        tool: Some(HookTool {
            #[cfg(debug_assertions)]
            home: None,
            exe: PathBuf::from("/opt/warden/bin/warden.exe"),
            data_dir: f.hook.data_dir.clone(),
        }),
        route: true,
        status: true,
    };
    let s = setup(Provider::Claude, &f.folder, "a1", &moved);
    assert_eq!(
        (s.routing, s.status),
        (Setup::NotInstalled, Setup::NotInstalled)
    );
    assert_eq!(
        outdated(Provider::Claude, &f.folder, "a1", &moved),
        (true, true)
    );
    // Another account's hooks are not this account's.
    let s = setup(Provider::Claude, &f.folder, "a2", &ours);
    assert_eq!(
        (s.routing, s.status),
        (Setup::NotInstalled, Setup::NotInstalled)
    );
    // Without warden.exe, nothing can be in place.
    let s = setup(Provider::Claude, &f.folder, "a1", &Want::default());
    assert_eq!(
        (s.routing, s.status),
        (Setup::NotInstalled, Setup::NotInstalled)
    );

    // Reinstall writes them for this Warden.
    sync(Provider::Claude, &f.folder, "a1", &f.hook.data_dir, &moved).expect("reinstall");
    let s = setup(Provider::Claude, &f.folder, "a1", &moved);
    assert_eq!((s.routing, s.status), (Setup::Installed, Setup::Installed));
    assert_eq!(
        outdated(Provider::Claude, &f.folder, "a1", &moved),
        (false, false)
    );
}

#[test]
fn an_earlier_builds_hooks_are_outdated_and_rewritten_with_the_users_own_untouched() {
    let f = fx("claude-old-form");
    let path = f.folder.join(CLAUDE_SETTINGS);
    // An earlier build: the routing hook as a quoted Git Bash command line, the status hook as curl.
    let data = slashed(&f.hook.data_dir);
    let old = serde_json::json!({
        "model": "opus",
        "hooks": {
            "UserPromptSubmit": [
                { "hooks": [ { "type": "command", "command": "echo mine" } ] },
                { "hooks": [
                    { "type": "command", "command": format!("'/opt/warden/warden.exe' route --account a1 --data-dir '{data}'"), "timeout": 10 },
                    { "type": "command", "command": format!("'/opt/warden/warden.exe' refresh --if-stale --account a1 --data-dir '{data}'"), "timeout": 60, "async": true },
                ] },
            ],
            "Stop": [ { "hooks": [ { "type": "command", "command": "curl.exe -q -s -m 1 --noproxy 127.0.0.1 -H \"Content-Type: application/json\" --data-binary \"@-\" \"http://127.0.0.1:4777/api/events?account=a1\"", "timeout": 5, "async": true } ] } ],
        },
    });
    fs::write(&path, serde_json::to_string_pretty(&old).expect("json")).expect("write");
    let ours = want(&f, true, true);
    let s = setup(Provider::Claude, &f.folder, "a1", &ours);
    assert_eq!(
        (s.routing, s.status),
        (Setup::NotInstalled, Setup::NotInstalled)
    );
    assert_eq!(
        outdated(Provider::Claude, &f.folder, "a1", &ours),
        (true, true)
    );
    // Only the status hook switched on: only it counts... but the sync writes what is wanted and takes the rest out.
    sync(Provider::Claude, &f.folder, "a1", &f.hook.data_dir, &ours).expect("repair");
    let s = setup(Provider::Claude, &f.folder, "a1", &ours);
    assert_eq!((s.routing, s.status), (Setup::Installed, Setup::Installed));
    assert_eq!(
        outdated(Provider::Claude, &f.folder, "a1", &ours),
        (false, false)
    );
    let text = fs::read_to_string(&path).expect("read");
    assert!(!text.contains("curl"), "{text}");
    assert!(!text.contains("'/opt"), "{text}");
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        v["hooks"][ROUTE_EVENT][0]["hooks"][0]["command"],
        "echo mine"
    );
    assert_eq!(v["model"], "opus");
    // Changed flags count too: a refresh that is no longer async is outdated.
    let flagged = text.replacen("\"async\": true", "\"async\": false", 1);
    fs::write(&path, flagged).expect("write");
    assert!(outdated(Provider::Claude, &f.folder, "a1", &ours).0);
    // Nothing of Warden's: nothing outdated.
    let g = fx("claude-none");
    fs::write(g.folder.join(CLAUDE_SETTINGS), CLAUDE_SETTINGS_FIXTURE).expect("write");
    assert_eq!(
        outdated(Provider::Claude, &g.folder, "a1", &want(&g, true, true)),
        (false, false)
    );
}

#[test]
fn a_users_codex_hook_that_warden_shifts_keeps_its_trust() {
    let f = fx("codex-shift");
    let config = f.folder.join(CODEX_CONFIG);
    let hooks = f.folder.join(CODEX_HOOKS);
    let go = |route: bool, status: bool| {
        sync(
            Provider::Codex,
            &f.folder,
            "a3",
            &f.hook.data_dir,
            &want(&f, route, status),
        )
        .expect("sync")
    };
    go(true, false);
    // The user adds their own hook after Warden's group, and Codex trusts it there (index 1).
    let mut v: Value =
        serde_json::from_str(&fs::read_to_string(&hooks).expect("read")).expect("json");
    let mine = serde_json::json!({ "matcher": "x", "hooks": [ { "type": "command", "command": "mine.cmd", "timeout": 7 } ] });
    v["hooks"][ROUTE_EVENT]
        .as_array_mut()
        .expect("groups")
        .push(mine);
    fs::write(&hooks, serde_json::to_string_pretty(&v).expect("json")).expect("write");
    let path = hooks.to_string_lossy().replace('/', "\\");
    let key = |g: usize| format!("{path}:user_prompt_submit:{g}:0");
    let mut text = fs::read_to_string(&config).expect("read");
    text.push_str(&format!(
        "\n[hooks.state.'{}']\nenabled = true\ntrusted_hash = \"sha256:mine\"\n",
        key(1)
    ));
    fs::write(&config, &text).expect("write");

    // Reinstall moves Warden's group after the user's: the user's hook is now at index 0, and its entry with it.
    go(true, false);
    let v: Value = serde_json::from_str(&fs::read_to_string(&hooks).expect("read")).expect("json");
    assert_eq!(
        v["hooks"][ROUTE_EVENT][0]["hooks"][0]["command"],
        "mine.cmd"
    );
    let table: toml::Table = fs::read_to_string(&config)
        .expect("read")
        .parse()
        .expect("toml");
    let state = table["hooks"]["state"].as_table().expect("state");
    assert_eq!(
        state[&key(0)]["trusted_hash"].as_str(),
        Some("sha256:mine"),
        "{state:?}"
    );
    // Warden's own entry is at index 1 now, for its own hash: the user's entry was not left there.
    let warden = &handlers(Provider::Codex, &f.hook, "a3")[0];
    assert_eq!(
        state[&key(1)]["trusted_hash"].as_str(),
        Some(codex_trusted_hash(ROUTE_EVENT, warden).as_str())
    );
    assert_eq!(
        setup(Provider::Codex, &f.folder, "a3", &want(&f, true, true)).routing,
        Setup::Installed
    );

    // Removing Warden's hooks leaves the user's at index 0, still trusted; nothing new is trusted.
    go(false, false);
    let after = fs::read_to_string(&config).expect("read");
    let table: toml::Table = after.parse().expect("toml");
    let state = table["hooks"]["state"].as_table().expect("state");
    assert_eq!(state[&key(0)]["trusted_hash"].as_str(), Some("sha256:mine"));
    assert_eq!(state.len(), 1, "{after}");
}

#[test]
fn trust_moves_only_for_the_same_unchanged_hook() {
    let file = Path::new("C:/<a>/hooks.json");
    let mine = serde_json::json!({ "type": "command", "command": "mine.cmd" });
    let ours = serde_json::json!({ "type": "command", "command": "warden.exe route --account a1 --data-dir d" });
    let before =
        serde_json::json!({ "hooks": { "Stop": [ { "hooks": [ours] }, { "hooks": [mine] } ] } });
    let after = serde_json::json!({ "hooks": { "Stop": [ { "hooks": [mine] } ] } });
    let moves = trust_moves(file, &before, &after);
    assert_eq!(
        moves,
        vec![(
            trust_keys(file, "Stop", 1, 0),
            trust_keys(file, "Stop", 0, 0)
        )]
    );
    // Changed by the user in between: not the same hook, nothing moves.
    let changed = serde_json::json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "other.cmd" } ] } ] } });
    assert!(trust_moves(file, &before, &changed).is_empty());
    // Moving the entry renames its header; an entry left at the target goes, others stay.
    let text = "a = 1\n\n[hooks.state.'C:\\<a>\\hooks.json:stop:0:0']\nenabled = true\ntrusted_hash = \"old\"\n\n[hooks.state.'C:\\<a>\\hooks.json:stop:1:0']\nenabled = true\ntrusted_hash = \"mine\"\n\n[other]\nb = 2\n";
    assert_eq!(
        move_trust(text, &moves).as_deref(),
        Some(
            "a = 1\n\n[hooks.state.'C:\\<a>\\hooks.json:stop:0:0']\nenabled = true\ntrusted_hash = \"mine\"\n\n[other]\nb = 2\n"
        )
    );
    // No entry to move: nothing changes.
    assert_eq!(move_trust("a = 1\n", &moves), None);
}

#[test]
fn claude_shell_is_found_as_claude_code_finds_git_bash() {
    let dir = Dir::new("git-bash");
    let folder = dir.path().join("account");
    fs::create_dir_all(&folder).expect("mkdir");
    let touch = |p: &Path| {
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(p, "").expect("write");
    };
    let none = crate::detect::Env::default();
    assert_eq!(claude_shell(&folder, &none), ClaudeShell::PowerShell);

    // Git installed in Program Files.
    let pf = dir.path().join("Program Files");
    let env = crate::detect::Env {
        program_files: vec![dir.path().join("Program Files (x86)"), pf.clone()],
        ..Default::default()
    };
    assert_eq!(claude_shell(&folder, &env), ClaudeShell::PowerShell);
    touch(&pf.join("Git/bin/bash.exe"));
    assert_eq!(claude_shell(&folder, &env), ClaudeShell::Bash);

    // Git elsewhere: `bin\bash.exe` beside the folder of the first `git` on PATH.
    let tools = dir.path().join("Tools/Git");
    let env = crate::detect::Env {
        path: vec![dir.path().join("empty"), tools.join("cmd")],
        ..Default::default()
    };
    touch(&tools.join("cmd/git.exe"));
    assert_eq!(claude_shell(&folder, &env), ClaudeShell::PowerShell);
    touch(&tools.join("bin/bash.exe"));
    assert_eq!(claude_shell(&folder, &env), ClaudeShell::Bash);
    // The same Git as `warden route` sees it, run by Git Bash: Git's own copy in `<Git>\ucrt64\bin` first on PATH.
    touch(&tools.join("ucrt64/bin/git.exe"));
    let env = crate::detect::Env {
        path: vec![tools.join("ucrt64/bin"), tools.join("cmd")],
        ..Default::default()
    };
    assert_eq!(claude_shell(&folder, &env), ClaudeShell::Bash);
    // Only the first `git` counts, as with `where git`.
    let other = dir.path().join("Other");
    touch(&other.join("cmd/git.exe"));
    let env = crate::detect::Env {
        path: vec![other.join("cmd"), tools.join("cmd")],
        ..Default::default()
    };
    assert_eq!(claude_shell(&folder, &env), ClaudeShell::PowerShell);

    // CLAUDE_CODE_GIT_BASH_PATH: used when it names an existing bash or sh, from the environment or settings.json.
    let custom = dir.path().join("My Bash/usr/bin/bash.exe");
    let named = |p: &Path| crate::detect::Env {
        claude_git_bash: Some(p.to_owned()),
        ..Default::default()
    };
    assert_eq!(
        claude_shell(&folder, &named(&custom)),
        ClaudeShell::PowerShell
    );
    touch(&custom);
    assert_eq!(claude_shell(&folder, &named(&custom)), ClaudeShell::Bash);
    let not_bash = dir.path().join("My Bash/zsh.exe");
    touch(&not_bash);
    assert_eq!(
        claude_shell(&folder, &named(&not_bash)),
        ClaudeShell::PowerShell
    );
    let settings = folder.join(CLAUDE_SETTINGS);
    let with_env =
        |p: &Path| serde_json::json!({ "env": { "CLAUDE_CODE_GIT_BASH_PATH": p } }).to_string();
    fs::write(&settings, with_env(&custom)).expect("write");
    assert_eq!(claude_shell(&folder, &none), ClaudeShell::Bash);
    // The account's settings come before the environment.
    fs::write(&settings, with_env(&not_bash)).expect("write");
    assert_eq!(
        claude_shell(&folder, &named(&custom)),
        ClaudeShell::PowerShell
    );
    // A settings file that is not JSON is no setting.
    fs::write(&settings, "{ not json").expect("write");
    assert_eq!(claude_shell(&folder, &named(&custom)), ClaudeShell::Bash);
}

#[test]
fn a_file_changed_since_it_was_read_is_never_overwritten() {
    let f = fx("hooks-race");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, "{\"model\":\"opus\"}\n").expect("write");
    // Warden read the file before the CLI wrote it again: the CLI's version stays.
    let err = replace(&path, Some("{}\n"), Some(b"{\"hooks\":{}}\n")).expect_err("refused");
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    let err = replace(&path, Some("{}\n"), None).expect_err("refused");
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert_eq!(read(&path), Some(b"{\"model\":\"opus\"}\n".to_vec()));
    // Unchanged since it was read: replaced, or deleted.
    replace(&path, Some("{\"model\":\"opus\"}\n"), Some(b"{}\n")).expect("replaced");
    assert_eq!(read(&path), Some(b"{}\n".to_vec()));
    replace(&path, Some("{}\n"), None).expect("deleted");
    assert_eq!(read(&path), None);
}

#[test]
fn an_empty_or_missing_json_file_reads_as_an_empty_object_and_a_non_object_is_refused() {
    let empty = Value::Object(Map::new());
    assert_eq!(parse_json(None).expect("missing"), empty);
    assert_eq!(parse_json(Some("")).expect("empty"), empty);
    assert_eq!(parse_json(Some(" \r\n\t")).expect("blank"), empty);
    assert_eq!(parse_json(Some("\u{feff}")).expect("only a BOM"), empty);
    assert_eq!(
        parse_json(Some("\u{feff}{\"a\": 1}")).expect("BOM")["a"],
        Value::from(1)
    );
    for bad in ["[]", "1", "\"text\"", "null", "{"] {
        let e = parse_json(Some(bad)).expect_err(bad);
        assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{bad}");
    }
}

#[test]
fn an_empty_settings_file_takes_the_hook_and_a_byte_order_mark_is_read_past() {
    let f = fx("claude-empty");
    let path = f.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, "").expect("write");
    install(Provider::Claude, &f.folder, "a1", &f.hook).expect("install into an empty file");
    let v: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
    assert!(v["hooks"][ROUTE_EVENT].is_array());

    let g = fx("claude-bom");
    let path = g.folder.join(CLAUDE_SETTINGS);
    fs::write(&path, "\u{feff}{\"model\": \"opus\"}\n").expect("write");
    install(Provider::Claude, &g.folder, "a1", &g.hook).expect("install past a BOM");
    let text = fs::read_to_string(&path).expect("read");
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).expect("json");
    assert_eq!(v["model"], "opus");
    assert!(v["hooks"][ROUTE_EVENT].is_array());
}

#[test]
fn a_json_array_or_hooks_in_another_shape_is_refused_and_left_alone() {
    for (tag, text) in [
        ("array", "[1, 2]\n"),
        ("hooks-list", "{\"hooks\": []}\n"),
        ("event-object", "{\"hooks\": {\"UserPromptSubmit\": {}}}\n"),
    ] {
        let f = fx(&format!("claude-shape-{tag}"));
        let path = f.folder.join(CLAUDE_SETTINGS);
        fs::write(&path, text).expect("write");
        let e = install(Provider::Claude, &f.folder, "a1", &f.hook).expect_err(tag);
        assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{tag}");
        assert_eq!(fs::read_to_string(&path).expect("read"), text, "{tag}");
    }
}

#[test]
fn installing_into_a_missing_folder_is_refused_and_creates_nothing() {
    let f = fx("claude-no-folder");
    let gone = f.folder.join("not-there");
    let e = install(Provider::Claude, &gone, "a1", &f.hook).expect_err("no folder");
    assert_eq!(e.kind(), io::ErrorKind::NotFound);
    assert!(!gone.exists());
}

#[test]
fn stripping_takes_out_only_wardens_handlers_and_the_lists_they_leave_empty() {
    let tool = fx("strip").hook;
    let as_json = |h: &Handler| serde_json::json!({ "type": "command", "command": h.command, "args": h.args });
    let route = as_json(&handlers(Provider::Claude, &tool, "a1")[0]);
    let status = as_json(&status_handler(Provider::Claude, &tool, "a1"));
    let codex_route = handlers(Provider::Codex, &tool, "a1")[0].command.clone();
    let mut v = serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [
                { "hooks": [ route, { "type": "command", "command": "echo mine" } ] },
                { "hooks": [ { "type": "command", "command": codex_route } ] },
            ],
            "Stop": [ { "hooks": [ status ] } ],
            "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "echo warden" } ] } ],
        },
        "model": "opus",
    });
    assert!(strip_json(&mut v));
    assert_eq!(
        v,
        serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": "echo mine" } ] } ],
                "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "echo warden" } ] } ],
            },
            "model": "opus",
        })
    );
    // Nothing of Warden's: nothing changes, and an empty event list the user wrote is kept.
    let mut user = serde_json::json!({ "hooks": { "Stop": [] }, "model": "opus" });
    let before = user.clone();
    assert!(!strip_json(&mut user));
    assert_eq!(user, before);

    // Only Warden's: the whole "hooks" key goes.
    let mut only = serde_json::json!({ "hooks": { "Stop": [ { "hooks": [ { "command": OLD_CURL } ] } ] }, "x": 1 });
    assert!(strip_json(&mut only));
    assert_eq!(only, serde_json::json!({ "x": 1 }));
}

#[test]
fn a_command_counts_as_wardens_only_with_its_marks() {
    assert!(is_ours(
        &status_handler(Provider::Codex, &fx("marks").hook, "a1").line()
    ));
    // An earlier build's curl status hook.
    assert!(is_ours(OLD_CURL));
    assert!(is_ours(&OLD_CURL.replacen("curl.exe ", "curl ", 1)));
    assert!(!is_ours(&OLD_CURL.replace("127.0.0.1", "localhost")));
    assert!(!is_ours("echo route --account a1"));
    assert!(is_ours(
        "\"C:/Users/<name>/Warden/warden.exe\" route --account a1"
    ));
    assert!(is_ours("WARDEN refresh --if-stale 60"));
    assert!(!is_ours("warden route"));
}

#[test]
fn json_is_written_with_the_files_own_line_endings_and_final_newline() {
    let v = serde_json::json!({ "a": 1 });
    assert_eq!(to_text(&v, None).expect("new"), "{\n  \"a\": 1\n}\n");
    assert_eq!(
        to_text(&v, Some("{}")).expect("no newline"),
        "{\n  \"a\": 1\n}"
    );
    assert_eq!(
        to_text(&v, Some("{}\r\n")).expect("crlf"),
        "{\r\n  \"a\": 1\r\n}\r\n"
    );
    assert_eq!(
        to_text(&v, Some("{\r\n}")).expect("crlf, no newline"),
        "{\r\n  \"a\": 1\r\n}"
    );
}

#[test]
fn trust_entries_are_appended_after_one_blank_line_in_the_files_line_endings() {
    let entry = |k: &str| vec![(k.to_owned(), "sha256:ab".to_owned())];
    assert_eq!(
        append_trust("", &entry("k")),
        "[hooks.state.'k']\nenabled = true\ntrusted_hash = \"sha256:ab\"\n"
    );
    let block = "\n[hooks.state.'k']\nenabled = true\ntrusted_hash = \"sha256:ab\"\n";
    assert_eq!(
        append_trust("a = 1", &entry("k")),
        format!("a = 1\n{block}")
    );
    assert_eq!(
        append_trust("a = 1\n", &entry("k")),
        format!("a = 1\n{block}")
    );
    assert_eq!(
        append_trust("a = 1\n\n", &entry("k")),
        format!("a = 1\n{block}")
    );
    assert_eq!(
        append_trust("a = 1\r\n", &entry("k")),
        "a = 1\r\n\r\n[hooks.state.'k']\r\nenabled = true\r\ntrusted_hash = \"sha256:ab\"\r\n"
    );
    let two = append_trust("", &[("x".into(), "h1".into()), ("y".into(), "h2".into())]);
    assert!(two.contains("trusted_hash = \"h1\"\n\n[hooks.state.'y']"));
    assert!(parses(&two));
}

#[test]
fn a_key_with_a_quote_is_written_double_quoted_and_read_back() {
    for key in [
        "C:\\<a>\\hooks.json:stop:0:0",
        "it's:stop:1:0",
        "a\"b\\c'd:x:0:0",
    ] {
        let text = append_trust("", &[(key.to_owned(), "h".to_owned())]);
        assert!(parses(&text), "{text}");
        let header = text.lines().next().expect("header");
        assert_eq!(header_key(header).as_deref(), Some(key), "{header}");
        assert!(trusted(&text, &[key.to_owned()], "h"));
        assert_eq!(strip_trust(&text, &[key.to_owned()]).as_deref(), Some(""));
    }
}

#[test]
fn trust_counts_only_with_the_same_hash_and_not_switched_off() {
    let k = vec!["k".to_owned()];
    let on = "[hooks.state.'k']\ntrusted_hash = \"h\"\n";
    assert!(trusted(on, &k, "h"));
    assert!(trusted(&format!("{on}enabled = true\n"), &k, "h"));
    assert!(!trusted(&format!("{on}enabled = false\n"), &k, "h"));
    assert!(!trusted(on, &k, "other"));
    assert!(!trusted(on, &["z".to_owned()], "h"));
    assert!(!trusted(
        "[hooks.state.'k'\ntrusted_hash = \"h\"\n",
        &k,
        "h"
    ));
    assert!(!trusted("", &k, "h"));
    // Any of the key's spellings will do.
    assert!(trusted(on, &["z".to_owned(), "k".to_owned()], "h"));
}

#[test]
fn trust_keys_name_both_spellings_of_a_windows_path_once_each() {
    assert_eq!(
        trust_keys(
            Path::new("C:\\Users\\<name>\\hooks.json"),
            "UserPromptSubmit",
            2,
            1
        ),
        vec![
            "C:\\Users\\<name>\\hooks.json:user_prompt_submit:2:1".to_owned(),
            "C:/Users/<name>/hooks.json:user_prompt_submit:2:1".to_owned(),
        ]
    );
    assert_eq!(event_label("Stop"), "stop");
    assert_eq!(event_label("PreToolUse"), "pre_tool_use");
}

#[test]
fn an_already_broken_config_toml_keeps_its_text_through_install_and_removal() {
    let f = fx("codex-broken-config");
    let config = f.folder.join(CODEX_CONFIG);
    fs::write(&config, "model = \n").expect("write");
    install(Provider::Codex, &f.folder, "a1", &f.hook).expect("install");
    assert!(
        fs::read_to_string(&config)
            .expect("read")
            .starts_with("model = \n")
    );
    sync(
        Provider::Codex,
        &f.folder,
        "a1",
        &f.hook.data_dir,
        &Want::default(),
    )
    .expect("remove");
    assert_eq!(fs::read_to_string(&config).expect("read"), "model = \n");
}
