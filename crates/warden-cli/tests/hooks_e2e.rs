//! The installed hooks, end to end: the real server installs both hooks in fixture account folders (one Claude, one
//! Codex), and this test plays the CLIs' part. It reads the hook files back and runs every handler in them the way
//! each CLI runs it on Windows, with the event's JSON on standard input:
//!
//! - Claude Code: the exec form (`command` and `args`), the program started directly with its arguments and no shell
//!   (Claude Code's hooks docs: "Exec form ... spawns it directly with `args` as the argument vector").
//! - Codex: `cmd.exe /C "<command>"` (`%COMSPEC%`), in the session's folder, without a window (Codex's hook runner,
//!   `codex-rs/hooks/src/engine/command_runner.rs`).
//!
//! The hooks run the built `warden` binary, copied to a folder with a space in its name as an installed app is, with a
//! data folder whose name has a space, a quote and a `$`. Then a session runs inside a stand-in CLI process (a copy
//! of `cmd.exe` named `claude.exe` or `codex.exe`, reading the hook commands from its standard input), so the core
//! finds the session's process up the hook's parent chain and ends the session when that process exits. A few
//! seconds, so it runs with the other tests whenever Rust changes.

#![cfg(windows)]

#[path = "../../warden-server/tests/common/mod.rs"]
mod common;

use common::{Fixture, call, write};
use serde_json::{Value, json};
use std::io::Write as _;
use std::net::SocketAddr;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use warden_core::routing;
use warden_core::store::{self, READINGS_FILE, Readings, SETTINGS_FILE, Settings};
use warden_core::testdir::Dir;
use warden_core::time::now_ms;

/// Codex starts its hooks without a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cli {
    Claude,
    Codex,
}

struct Ran {
    ok: bool,
    stdout: String,
    took: Duration,
}

/// One handler as written in a hook file.
struct Handler {
    command: String,
    args: Option<Vec<String>>,
    timeout: Duration,
    is_async: bool,
}

impl Handler {
    fn line(&self) -> String {
        match &self.args {
            Some(a) => format!("{} {}", self.command, a.join(" ")),
            None => self.command.clone(),
        }
    }
}

fn shell() -> PathBuf {
    std::env::var_os("COMSPEC").map_or_else(|| PathBuf::from("cmd.exe"), PathBuf::from)
}

/// Runs one handler as the CLI does, giving up (and failing) after the handler's timeout.
fn run(cli: Cli, h: &Handler, input: &str, cwd: &Path) -> Ran {
    let mut cmd = match (cli, &h.args) {
        (Cli::Claude, Some(args)) => {
            let mut c = Command::new(&h.command);
            c.args(args);
            c
        }
        (Cli::Claude, None) => {
            // The user's own hook in the shell form (`exit 0`), which reads the same in any shell; Warden's are never
            // in it.
            assert!(!h.command.contains("warden"), "{}", h.command);
            let mut c = Command::new(shell());
            c.arg("/C").arg(&h.command);
            c
        }
        (Cli::Codex, _) => {
            assert!(h.args.is_none(), "Codex has no exec form");
            let mut c = Command::new(shell());
            c.arg("/C").raw_arg(format!("\"{}\"", h.command));
            c.creation_flags(CREATE_NO_WINDOW);
            c
        }
    };
    cmd.current_dir(cwd)
        .env("USERPROFILE", cwd)
        .env("HOME", cwd)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = cmd.spawn().expect("the hook starts");
    // A hook that does not read its input may close it first; that is not a failure.
    let _ = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes());
    loop {
        if child.try_wait().expect("wait").is_some() {
            break;
        }
        if start.elapsed() > h.timeout {
            let _ = child.kill();
            panic!(
                "{cli:?}: the hook ran past its {:?} timeout: {}",
                h.timeout,
                h.line()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let took = start.elapsed();
    let out = child.wait_with_output().expect("output");
    Ran {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        took,
    }
}

/// Every handler on an event in a Claude `settings.json` or Codex `hooks.json`, in the order the CLI runs them.
fn handlers(hook_file: &Path, event: &str) -> Vec<Handler> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(hook_file).expect("hook file"))
        .expect("JSON");
    let mut out = Vec::new();
    for group in v["hooks"][event].as_array().into_iter().flatten() {
        for h in group["hooks"].as_array().into_iter().flatten() {
            assert_eq!(h["type"], "command");
            out.push(Handler {
                command: h["command"].as_str().expect("command").to_owned(),
                args: h["args"].as_array().map(|a| {
                    a.iter()
                        .map(|x| x.as_str().expect("a string argument").to_owned())
                        .collect()
                }),
                timeout: Duration::from_secs(h["timeout"].as_u64().unwrap_or(600)),
                is_async: h["async"] == true,
            });
        }
    }
    out
}

fn is_route(h: &Handler) -> bool {
    h.line().contains(" route --account ")
}

fn is_refresh(h: &Handler) -> bool {
    h.line().contains(" refresh --if-stale ")
}

fn is_status(h: &Handler) -> bool {
    h.line().contains(" event --account ")
}

/// The event as the CLI sends it on stdin (the fields each CLI's hooks docs list).
fn event(cli: Cli, session: &str, cwd: &Path, name: &str, extra: Value) -> String {
    let mut e = match cli {
        Cli::Claude => json!({
            "session_id": session,
            "transcript_path": cwd.join("transcript.jsonl"),
            "cwd": cwd,
            "permission_mode": "default",
            "hook_event_name": name,
        }),
        Cli::Codex => json!({
            "session_id": session,
            "transcript_path": cwd.join("rollout.jsonl"),
            "cwd": cwd,
            "hook_event_name": name,
            "model": "gpt-5",
            "turn_id": "turn-1",
            "permission_mode": "default",
        }),
    };
    if let (Some(e), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        e.extend(x.clone());
    }
    e.to_string()
}

/// Runs every handler on an event; returns what the routing hook printed (the only output that reaches the AI).
fn fire(cli: Cli, hook_file: &Path, cwd: &Path, name: &str, input: &str) -> String {
    let list = handlers(hook_file, name);
    assert!(!list.is_empty(), "{cli:?}: nothing installed on {name}");
    let mut printed = String::new();
    for h in list {
        // Only the routing hook's answer is waited for; the CLI runs the status hook and the refresh in the
        // background, so they never hold up the turn.
        if is_status(&h) || is_refresh(&h) {
            assert!(h.is_async, "{cli:?} {name}: `{}` is not async", h.line());
        } else if is_route(&h) {
            assert!(
                !h.is_async,
                "{cli:?}: the routing hook's answer would be lost"
            );
        }
        let ran = run(cli, &h, input, cwd);
        assert!(ran.ok, "{cli:?} {name}: `{}` failed", h.line());
        if is_route(&h) {
            printed.push_str(&ran.stdout);
        } else {
            assert_eq!(ran.stdout, "", "{cli:?} {name}: `{}` printed", h.line());
        }
    }
    printed
}

fn state(addr: SocketAddr) -> Value {
    call(addr, "GET", "/api/state", None).json()
}

fn find_session(addr: SocketAddr, id: &str) -> Option<Value> {
    state(addr)["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .find(|s| s["id"] == id)
        .cloned()
}

fn session(addr: SocketAddr, id: &str) -> Value {
    find_session(addr, id).unwrap_or_else(|| panic!("session {id} not in the state"))
}

fn account(addr: SocketAddr, id: &str) -> Value {
    state(addr)["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["id"] == id)
        .expect("account")
        .clone()
}

fn add(addr: SocketAddr, provider: &str, folder: &Path, label: &str) -> String {
    let body = json!({ "provider": provider, "folder": folder, "label": label }).to_string();
    let reply = call(addr, "POST", "/api/accounts", Some(&body));
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()["accounts"]
        .as_array()
        .expect("accounts")
        .last()
        .expect("added")["id"]
        .as_str()
        .expect("id")
        .to_owned()
}

fn set_hooks(addr: SocketAddr, id: &str, on: bool) {
    let body = json!({ "enabled": on }).to_string();
    let reply = call(addr, "PATCH", &format!("/api/accounts/{id}"), Some(&body));
    assert_eq!(reply.status, 200, "{}", reply.body);
}

/// One session's life through the installed hooks, checking the server's state after each event.
fn play_session(cli: Cli, addr: SocketAddr, account_id: &str, hook_file: &Path, cwd: &Path) {
    let sid = format!("{cli:?}-session").to_lowercase();
    let fire = |name: &str, extra: Value| {
        fire(
            cli,
            hook_file,
            cwd,
            name,
            &event(cli, &sid, cwd, name, extra),
        )
    };
    let start = match cli {
        Cli::Claude => json!({ "source": "startup", "model": "claude-opus" }),
        Cli::Codex => json!({ "source": "startup" }),
    };
    assert_eq!(fire("SessionStart", start), "");
    let s = session(addr, &sid);
    assert_eq!(s["state"], "idle");
    assert_eq!(s["accountId"], account_id);
    assert_eq!(s["source"], "hook");
    assert_eq!(s["project"], "shop");

    let told = fire("UserPromptSubmit", json!({ "prompt": "fix the build" }));
    assert_eq!(told, "", "Warden accounts get no routing advice");
    assert_eq!(session(addr, &sid)["state"], "working");

    let tool = match cli {
        Cli::Claude => "Edit",
        Cli::Codex => "apply_patch",
    };
    let tool_event =
        json!({ "tool_name": tool, "tool_use_id": "t1", "tool_input": { "secret": "x" } });
    assert_eq!(fire("PreToolUse", tool_event.clone()), "");
    let s = session(addr, &sid);
    assert_eq!(s["state"], "working");
    assert_eq!(s["doing"], "Editing files");

    let shell_tool = match cli {
        Cli::Claude => "Bash",
        Cli::Codex => "shell",
    };
    assert_eq!(
        fire(
            "PermissionRequest",
            json!({ "tool_name": shell_tool, "tool_input": { "command": "npm ci" } })
        ),
        ""
    );
    assert_eq!(session(addr, &sid)["state"], "waiting");
    assert_eq!(fire("PostToolUse", tool_event), "");
    assert_eq!(session(addr, &sid)["state"], "working");

    let agent = json!({ "agent_id": "agent-1", "agent_type": "Explore" });
    assert_eq!(fire("SubagentStart", agent.clone()), "");
    assert_eq!(
        session(addr, &sid)["subagents"],
        json!([{ "type": "Explore" }])
    );
    let mut stopped = agent;
    stopped["stop_hook_active"] = json!(false);
    stopped["last_assistant_message"] = json!("found it");
    assert_eq!(fire("SubagentStop", stopped), "");
    assert_eq!(session(addr, &sid)["subagents"], json!([]));

    let stop = json!({ "stop_hook_active": false, "last_assistant_message": "done" });
    assert_eq!(fire("Stop", stop.clone()), "");
    assert_eq!(session(addr, &sid)["state"], "idle");

    // Nothing changed since the session was told: the routing hook stays silent.
    let again = fire("UserPromptSubmit", json!({ "prompt": "and the tests" }));
    assert_eq!(again, "", "{cli:?}: the routing hook spoke again");
    assert_eq!(session(addr, &sid)["state"], "working");
    assert_eq!(fire("Stop", stop), "");
    assert_eq!(session(addr, &sid)["state"], "idle");

    if cli == Cli::Claude {
        assert_eq!(fire("SessionEnd", json!({ "reason": "exit" })), "");
        assert_eq!(session(addr, &sid)["state"], "done");
    }
}

/// With the server stopped, every hook still succeeds at once and prints nothing but the routing guidance (from the
/// saved readings; the background refresh finds them fresh), so a turn is never held up.
fn play_server_stopped(cli: Cli, hook_file: &Path, cwd: &Path, data_dir: &Path) {
    let sid = format!("{cli:?}-after").to_lowercase();
    for name in ["SessionStart", "UserPromptSubmit", "Stop"] {
        let input = event(cli, &sid, cwd, name, json!({ "prompt": "hi" }));
        for h in handlers(hook_file, name) {
            if is_refresh(&h) {
                // Fresh readings: the refresh reads nothing, so the real usage endpoint is never called.
                let settings: Settings = store::load(data_dir, SETTINGS_FILE).expect("settings");
                let readings: Readings = store::load(data_dir, READINGS_FILE).expect("readings");
                assert!(
                    !routing::readings_are_stale(&settings, &readings, now_ms()),
                    "the readings went stale; the refresh would call the real endpoint"
                );
            }
            let ran = run(cli, &h, &input, cwd);
            println!("{cli:?} {name}, server stopped: {:?}", ran.took);
            assert!(
                ran.ok,
                "{cli:?} {name}: `{}` failed without the server",
                h.line()
            );
            assert_eq!(ran.stdout, "", "Warden accounts get no advice");
            assert!(
                ran.took < Duration::from_secs(1),
                "{cli:?} {name}: `{}` took {:?} with the server stopped",
                h.line(),
                ran.took
            );
        }
    }
}

/// A stand-in for the CLI's process: a copy of `cmd.exe` under the CLI's program name, reading commands from its
/// standard input, so the hooks it runs have it as their parent, as under the real CLI. It exits when its input
/// closes.
struct FakeCli {
    child: Child,
    dir: PathBuf,
    n: usize,
}

impl FakeCli {
    fn start(home: &Path, cli: Cli) -> FakeCli {
        let dir = home.join(format!("fake {cli:?}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let name = match cli {
            Cli::Claude => "claude.exe",
            Cli::Codex => "codex.exe",
        };
        let exe = dir.join(name);
        std::fs::copy(shell(), &exe).expect("copy cmd.exe");
        let child = Command::new(&exe)
            .env("USERPROFILE", home)
            .env("HOME", home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .args(["/Q", "/K"])
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("the stand-in CLI starts");
        FakeCli { child, dir, n: 0 }
    }

    /// Runs the status hook's handler on an event inside the stand-in, as the CLI would, and waits until the
    /// session shows the event (the core has it once `warden event` exits).
    fn send(&mut self, cli: Cli, h: &Handler, input: &str) {
        self.n += 1;
        let file = self.dir.join(format!("event-{}.json", self.n));
        std::fs::write(&file, input).expect("write");
        let done = self.dir.join(format!("done-{}", self.n));
        let program = match cli {
            Cli::Claude => {
                let quoted: Vec<String> = h
                    .args
                    .as_ref()
                    .expect("args")
                    .iter()
                    .map(|a| format!("\"{a}\""))
                    .collect();
                format!("\"{}\" {}", h.command, quoted.join(" "))
            }
            Cli::Codex => h.command.clone(),
        };
        let line = format!(
            "{program} < \"{}\" & echo.> \"{}\"\r\n",
            file.display(),
            done.display()
        );
        let stdin = self.child.stdin.as_mut().expect("stdin");
        stdin.write_all(line.as_bytes()).expect("send");
        stdin.flush().expect("flush");
        common::eventually("the hook to run in the stand-in CLI", || done.exists());
    }

    /// The CLI ends: its input closes and it exits.
    fn exit(mut self) {
        drop(self.child.stdin.take());
        self.child.wait().expect("exits");
    }

    /// The CLI is killed.
    fn kill(mut self) {
        self.child.kill().expect("killed");
        self.child.wait().expect("exits");
    }
}

/// Sessions end when their CLI's process exits: a Codex session (Codex never says it ended) becomes done, and a
/// Claude session that never reported its end (crashed or killed) goes.
fn play_process_exit(addr: SocketAddr, home: &Path, files: &[(Cli, PathBuf)], cwd: &Path) {
    for (cli, hook_file) in files {
        let cli = *cli;
        let sid = format!("{cli:?}-in-a-process").to_lowercase();
        let mut fake = FakeCli::start(home, cli);
        let status = handlers(hook_file, "SessionStart")
            .into_iter()
            .find(is_status)
            .expect("the status hook");
        let input = event(cli, &sid, cwd, "SessionStart", json!({}));
        fake.send(cli, &status, &input);
        common::eventually("the session", || find_session(addr, &sid).is_some());
        let prompt = handlers(hook_file, "UserPromptSubmit")
            .into_iter()
            .find(is_status)
            .expect("the status hook");
        fake.send(
            cli,
            &prompt,
            &event(
                cli,
                &sid,
                cwd,
                "UserPromptSubmit",
                json!({ "prompt": "go" }),
            ),
        );
        common::eventually("the session to work", || {
            find_session(addr, &sid).is_some_and(|s| s["state"] == "working")
        });
        // The CLI is still running, so nothing ends; then it goes.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(session(addr, &sid)["state"], "working");
        match cli {
            Cli::Codex => {
                fake.exit();
                common::eventually("the Codex session to be done", || {
                    find_session(addr, &sid).is_some_and(|s| s["state"] == "done")
                });
            }
            Cli::Claude => {
                fake.kill();
                common::eventually("the crashed Claude session to go", || {
                    find_session(addr, &sid).is_none()
                });
            }
        }
    }
}

#[test]
fn installed_hooks_run_as_the_clis_run_them() {
    let mut f = Fixture::new();
    // A data folder with a space, a quote and a dollar sign, which no shell sees now.
    f.data = Dir::new("Warden's $data");
    // The hooks' tool where an installed app has it: a folder with a space.
    let app_dir = f.home.path().join("Warden App");
    std::fs::create_dir_all(&app_dir).expect("mkdir");
    f.route_exe = app_dir.join("warden.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_warden"), &f.route_exe).expect("copy warden.exe");

    let claude = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let codex = f.codex_folder(".codex-work", "bob@acme.example", "t2");
    // The users' own files, with their own hooks, which must survive and come back byte for byte.
    let claude_settings = claude.join("settings.json");
    let codex_hooks = codex.join("hooks.json");
    let codex_config = codex.join("config.toml");
    let originals = [
        (
            claude_settings.clone(),
            "{\r\n  \"model\": \"opus\",\r\n  \"hooks\": {\r\n    \"Stop\": [\r\n      { \"hooks\": [ { \"type\": \"command\", \"command\": \"exit 0\" } ] }\r\n    ]\r\n  }\r\n}\r\n",
        ),
        (
            codex_hooks.clone(),
            "{\n  \"hooks\": {\n    \"Stop\": [{ \"hooks\": [{ \"type\": \"command\", \"command\": \"exit 0\" }] }]\n  }\n}",
        ),
        (
            codex_config.clone(),
            "# my settings\nmodel = \"gpt-5\"\n\n[tui]\nanimations = false\n",
        ),
    ];
    for (path, text) in &originals {
        write(path, text);
    }
    let project = f.home.path().join("shop");
    write(&project.join(".git/HEAD"), "ref: refs/heads/main\n");

    let server = f.start();
    let addr = server.addr;
    let claude_id = add(addr, "claude", &claude, "Work");
    let codex_id = add(addr, "codex", &codex, "Codex");
    // The first usage reading is in, so `warden route` has something to tell.
    common::eventually("the first usage reading", || {
        !account(addr, &claude_id)["usage"].is_null()
    });
    for id in [&claude_id, &codex_id] {
        set_hooks(addr, id, true);
        let hooks = &account(addr, id)["hooks"];
        assert_eq!(*hooks, "installed", "{id}");
    }
    // Claude's hooks run warden.exe itself, with no shell around it.
    for h in handlers(&claude_settings, "UserPromptSubmit")
        .iter()
        .filter(|h| h.line().contains("warden"))
    {
        assert_eq!(Path::new(&h.command), f.route_exe, "{}", h.line());
    }

    play_session(Cli::Claude, addr, &claude_id, &claude_settings, &project);
    play_session(Cli::Codex, addr, &codex_id, &codex_hooks, &project);
    play_process_exit(
        addr,
        f.home.path(),
        &[
            (Cli::Codex, codex_hooks.clone()),
            (Cli::Claude, claude_settings.clone()),
        ],
        &project,
    );

    server.stop();
    play_server_stopped(Cli::Claude, &claude_settings, &project, f.data.path());
    play_server_stopped(Cli::Codex, &codex_hooks, &project, f.data.path());

    // Started again: the hooks are still this Warden's, so nothing is rewritten.
    let server = f.start();
    let addr = server.addr;
    assert!(state(addr).get("hooksUpdated").is_none());
    for id in [&claude_id, &codex_id] {
        let hooks = &account(addr, id)["hooks"];
        assert_eq!(*hooks, "installed", "{id}");
    }
    // Removed through the API: every file back as it was, byte for byte.
    for id in [&claude_id, &codex_id] {
        set_hooks(addr, id, false);
    }
    for (path, text) in &originals {
        assert_eq!(
            std::fs::read_to_string(path).expect("read"),
            *text,
            "{} is not as it was",
            path.display()
        );
    }
    server.stop();
}
