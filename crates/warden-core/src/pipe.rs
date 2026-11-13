//! The per-user connection between the hooks and the core: a named pipe only the current Windows user can open.
//!
//! The core creates the pipe with a security descriptor that lets only the user it runs as connect (no one else, and
//! no one else can add an instance of it), under a name made fresh on every start, and writes that name and its own
//! process id to [`CONNECTION_FILE`] in its data folder (in the user's profile, which other users cannot read). The
//! hook's `warden event` reads that file, connects, checks that the process at the other end is the one the file
//! names (so a pipe someone else made under the same name later is never sent anything), and sends one
//! [`Message`]: the account and the hook event, cut down to [`HookEvent`]'s fields. The core answers with one byte
//! once it has taken the event, so `warden.exe` (whose parent chain leads to the session's CLI) is still running while
//! the core looks at it. Nothing is listening on a network port.
//!
//! The fields kept are the shared contract with the sessions code: the hook event's name, the session id, the working
//! folder, the transcript path (a path only; the file is never sent), tool names, subagent ids and types, and the
//! notification kind. Prompts, tool inputs, tool outputs and messages never leave the hook: they are
//! skipped while `warden event` parses its input, and the core refuses a message with any other field.

use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The file in the data folder naming the core's pipe and the core's process id, while it runs.
pub const CONNECTION_FILE: &str = "pipe.json";
/// The largest message the core reads.
pub const MAX_MESSAGE: u64 = 64 * 1024;
/// The most of a hook's input `warden event` reads (a huge tool input is skipped as it is read, never kept).
pub const MAX_HOOK_INPUT: u64 = 64 * 1024 * 1024;
/// How long `warden event` waits for the core, all told, before giving up (and exiting 0 all the same).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(1);
/// The longest id or name kept (session ids, tool and subagent names and types, the event's name).
const MAX_NAME: usize = 200;
/// The longest path kept (the working folder, the transcript path).
const MAX_PATH: usize = 1024;

/// A hook event as it travels to the core: only these fields, each as the CLI sent it (trimmed to a sane length).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookEvent {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "parent_id"
    )]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_event_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// A path only: the transcript itself is never read by the hook nor sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Claude's `Notification` kind (`permission_prompt`, `idle_prompt`, ...), never its message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
}

/// Only these session ids may be put in a delegation command or accepted as a parent link.
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parent_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| {
        v.as_str()
            .filter(|s| valid_session_id(s))
            .map(str::to_owned)
    }))
}

/// The same fields read from a hook's raw input, where every other field (the prompt, the tool input and output,
/// messages) is skipped by the parser without being kept.
#[derive(Deserialize)]
struct RawHook {
    hook_event_name: Option<serde_json::Value>,
    session_id: Option<serde_json::Value>,
    cwd: Option<serde_json::Value>,
    transcript_path: Option<serde_json::Value>,
    tool_name: Option<serde_json::Value>,
    agent_id: Option<serde_json::Value>,
    agent_type: Option<serde_json::Value>,
    notification_type: Option<serde_json::Value>,
}

/// A string field, without control characters, cut to `max` characters; anything else is dropped.
fn text(v: Option<serde_json::Value>, max: usize) -> Option<String> {
    let s: String = v?
        .as_str()?
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

impl HookEvent {
    /// Reads a hook's input (the JSON the CLI writes to the hook's standard input) and keeps only the contract's
    /// fields. `None` for input that is not a JSON object.
    pub fn from_hook_input(input: impl Read) -> Option<HookEvent> {
        let reader = io::BufReader::new(input.take(MAX_HOOK_INPUT));
        let raw: RawHook = serde_json::from_reader(reader).ok()?;
        Some(HookEvent {
            // This field comes only from warden event's environment, never from the CLI's raw input.
            parent_session_id: None,
            hook_event_name: text(raw.hook_event_name, MAX_NAME),
            session_id: text(raw.session_id, MAX_NAME),
            cwd: text(raw.cwd, MAX_PATH),
            transcript_path: text(raw.transcript_path, MAX_PATH),
            tool_name: text(raw.tool_name, MAX_NAME),
            agent_id: text(raw.agent_id, MAX_NAME),
            agent_type: text(raw.agent_type, MAX_NAME),
            notification_type: text(raw.notification_type, MAX_NAME),
        })
    }
}

/// One message from a hook to the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// The account whose hook sent it (`--account`).
    pub account: String,
    pub event: HookEvent,
}

impl Message {
    /// Reads one message as the core receives it: a JSON line of at most [`MAX_MESSAGE`] bytes, with exactly the
    /// fields above. `Err` for anything else.
    pub fn read(from: impl Read) -> Result<Message, String> {
        let mut line = Vec::new();
        for b in io::BufReader::new(from.take(MAX_MESSAGE)).bytes() {
            match b {
                Ok(b'\n') => break,
                Ok(b) => line.push(b),
                Err(_) => return Err("the message could not be read".into()),
            }
        }
        serde_json::from_slice(&line).map_err(|_| "not a hook message".to_owned())
    }

    /// The message as it is sent: one JSON line.
    pub fn to_line(&self) -> Vec<u8> {
        let mut line = serde_json::to_vec(self).unwrap_or_default();
        line.push(b'\n');
        line
    }
}

/// What [`CONNECTION_FILE`] holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    /// The pipe's full name, `\\.\pipe\warden-...`.
    pub pipe: String,
    /// The core's process id: the process at the other end of the pipe must be this one.
    pub pid: u32,
}

impl ConnectionInfo {
    pub fn load(data_dir: &Path) -> Option<ConnectionInfo> {
        let bytes = std::fs::read(data_dir.join(CONNECTION_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

/// The security descriptor (in Windows' string form) that lets only `user_sid` use the pipe: a protected DACL with
/// one entry, so nothing is inherited and nobody else (not even another user's process asking for read access, nor
/// one wanting to add an instance of the pipe) gets in.
pub fn user_only_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;{user_sid})")
}

/// A fresh pipe name: the core's process id and 64 random bits, so it is never one a stale or squatted pipe has.
fn fresh_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    format!(
        r"\\.\pipe\warden-{}-{:016x}",
        std::process::id(),
        h.finish()
    )
}

#[cfg(windows)]
mod imp {
    use super::*;
    use interprocess::ConnectWaitMode;
    use interprocess::os::windows::named_pipe::{
        DuplexPipeStream, PipeListener, PipeListenerOptions, pipe_mode,
    };
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use std::io::Write;

    type Stream = DuplexPipeStream<pipe_mode::Bytes>;

    /// The core's end: the pipe, and the connection file naming it (taken away when the listener goes).
    pub struct Listener {
        listener: PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>,
        pipe: String,
        file: PathBuf,
    }

    /// One hook's connection, as the core sees it.
    pub struct Connection {
        stream: Stream,
    }

    impl Listener {
        /// Creates the pipe for `user_sid` alone and writes the connection file into `data_dir`. The first
        /// instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so this fails rather than share a name someone
        /// else made first.
        pub fn create(data_dir: &Path, user_sid: &str) -> io::Result<Listener> {
            let sddl = widestring::U16CString::from_str(user_only_sddl(user_sid))
                .map_err(io::Error::other)?;
            let sd = SecurityDescriptor::deserialize(&sddl)?;
            let pipe = fresh_name();
            let listener = PipeListenerOptions::new()
                .path(pipe.as_str())
                .security_descriptor(Some(sd))
                .create_duplex::<pipe_mode::Bytes>()?;
            let file = data_dir.join(CONNECTION_FILE);
            let info = ConnectionInfo {
                pipe: pipe.clone(),
                pid: std::process::id(),
            };
            crate::store::write_atomic(
                &file,
                &serde_json::to_vec(&info).map_err(io::Error::other)?,
            )?;
            Ok(Listener {
                listener,
                pipe,
                file,
            })
        }

        /// Waits for the next hook to connect.
        pub fn accept(&self) -> io::Result<Connection> {
            self.listener.accept().map(|stream| Connection { stream })
        }

        /// Connects once and hangs up, so a thread waiting in [`accept`](Self::accept) returns (to stop).
        pub fn wake(&self) {
            let _ = Stream::connect_by_path_with_wait_mode(
                self.pipe.as_str(),
                ConnectWaitMode::Timeout(Duration::from_millis(200)),
            );
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Only our own file: a newer core may have written its own since.
            if ConnectionInfo::load(self.file.parent().unwrap_or(Path::new(".")))
                .is_some_and(|i| i.pipe == self.pipe)
            {
                let _ = std::fs::remove_file(&self.file);
            }
        }
    }

    impl Connection {
        /// The process id of the program that connected (the hook's `warden.exe`), as Windows reports it.
        pub fn client_pid(&self) -> Option<u32> {
            self.stream.client_process_id().ok()
        }

        /// Reads the hook's message.
        pub fn read(&mut self) -> Result<Message, String> {
            Message::read(&mut self.stream)
        }

        /// Tells the hook its message was taken, so it can exit.
        pub fn answer(mut self) {
            let _ = self.stream.write_all(b"1");
            let _ = self.stream.flush();
        }
    }

    /// Sends one message to the core whose connection file is in `data_dir`, and waits for its answer. Fails when
    /// no core runs there, or the process at the other end is not the one the file names.
    pub fn send(data_dir: &Path, message: &Message) -> io::Result<()> {
        let info = ConnectionInfo::load(data_dir)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Warden is not running"))?;
        let mut stream = Stream::connect_by_path_with_wait_mode(
            info.pipe.as_str(),
            ConnectWaitMode::Timeout(SEND_TIMEOUT),
        )?;
        if stream.server_process_id()? != info.pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the pipe is not Warden's",
            ));
        }
        stream.write_all(&message.to_line())?;
        stream.flush()?;
        let mut answer = [0_u8; 1];
        stream.read_exact(&mut answer)?;
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Warden's pipe is Windows only",
        ))
    }

    pub struct Listener;
    pub struct Connection;

    impl Listener {
        pub fn create(_data_dir: &Path, _user_sid: &str) -> io::Result<Listener> {
            unsupported()
        }
        pub fn accept(&self) -> io::Result<Connection> {
            unsupported()
        }
        pub fn wake(&self) {}
    }

    impl Connection {
        pub fn client_pid(&self) -> Option<u32> {
            None
        }
        pub fn read(&mut self) -> Result<Message, String> {
            Err("unsupported".into())
        }
        pub fn answer(self) {}
    }

    pub fn send(_data_dir: &Path, _message: &Message) -> io::Result<()> {
        unsupported()
    }
}

pub use imp::{Connection, Listener, send};

/// Sends a message on a thread of its own and gives up after [`SEND_TIMEOUT`], however the core behaves. What
/// `warden event` does; tests use it too.
pub fn send_with_timeout(data_dir: &Path, message: Message) -> io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let dir = data_dir.to_owned();
    std::thread::spawn(move || {
        let _ = tx.send(send(&dir, &message));
    });
    rx.recv_timeout(SEND_TIMEOUT).unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Warden did not answer",
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_contracts_fields_are_kept_from_a_hook_input() {
        let input = serde_json::json!({
            "session_id": "s1",
            "hook_event_name": "PreToolUse",
            "cwd": "%USERPROFILE%/work/shop",
            "transcript_path": "%USERPROFILE%/work/t.jsonl",
            "tool_name": "Bash",
            "tool_input": { "command": "secret" },
            "prompt": "secret prompt",
            "tool_response": "secret output",
            "message": "secret message",
            "last_assistant_message": "secret reply",
            "agent_id": "a1",
            "agent_type": "Explore",
            "notification_type": "permission_prompt",
            "permission_mode": "default",
        })
        .to_string();
        let e = HookEvent::from_hook_input(input.as_bytes()).expect("an event");
        let sent = String::from_utf8(
            Message {
                account: "a1".into(),
                event: e.clone(),
            }
            .to_line(),
        )
        .expect("utf-8");
        assert!(!sent.contains("secret"), "{sent}");
        assert_eq!(e.session_id.as_deref(), Some("s1"));
        assert_eq!(
            e.transcript_path.as_deref(),
            Some("%USERPROFILE%/work/t.jsonl")
        );
        assert_eq!(e.notification_type.as_deref(), Some("permission_prompt"));
        assert_eq!(e.agent_type.as_deref(), Some("Explore"));
    }

    #[test]
    fn odd_values_are_dropped_or_cut_and_non_objects_refused() {
        let long = "x".repeat(5000);
        let input = serde_json::json!({ "session_id": 5, "tool_name": "Ba\u{7}sh", "cwd": long })
            .to_string();
        let e = HookEvent::from_hook_input(input.as_bytes()).expect("an event");
        assert_eq!(e.session_id, None);
        assert_eq!(e.tool_name.as_deref(), Some("Bash"));
        assert_eq!(e.cwd.map(|c| c.len()), Some(MAX_PATH));
        assert_eq!(HookEvent::from_hook_input(&b"[1]"[..]), None);
        assert_eq!(HookEvent::from_hook_input(&b"not json"[..]), None);
    }

    #[test]
    fn the_core_refuses_a_message_with_any_other_field() {
        let ok = br#"{"account":"a1","event":{"session_id":"s1","hook_event_name":"Stop"}}"#;
        assert_eq!(Message::read(&ok[..]).expect("a message").account, "a1");
        let extra = br#"{"account":"a1","event":{"session_id":"s1","prompt":"hi"}}"#;
        assert!(Message::read(&extra[..]).is_err());
        let top = br#"{"account":"a1","event":{},"tool_input":{}}"#;
        assert!(Message::read(&top[..]).is_err());
        // One line only: what follows the line break is not read.
        let two = b"{\"account\":\"a1\",\"event\":{}}\n{\"x\":1}";
        assert!(Message::read(&two[..]).is_ok());
    }

    #[test]
    fn only_a_valid_parent_id_is_accepted_and_raw_input_cannot_supply_it() {
        for id in ["", "bad id", "../x", "$(cmd)", "é", &"a".repeat(129)] {
            assert!(!valid_session_id(id));
            let wire =
                serde_json::json!({"account":"a1","event":{"parent_session_id":id}}).to_string();
            assert_eq!(
                Message::read(wire.as_bytes())
                    .expect("message")
                    .event
                    .parent_session_id,
                None
            );
        }
        for id in ["abc-123_X", &"a".repeat(128)] {
            assert!(valid_session_id(id));
            let wire =
                serde_json::json!({"account":"a1","event":{"parent_session_id":id}}).to_string();
            assert_eq!(
                Message::read(wire.as_bytes())
                    .expect("message")
                    .event
                    .parent_session_id
                    .as_deref(),
                Some(id)
            );
            let input = serde_json::json!({"parent_session_id":id}).to_string();
            assert_eq!(
                HookEvent::from_hook_input(input.as_bytes())
                    .expect("input")
                    .parent_session_id,
                None
            );
        }
    }

    #[test]
    fn the_pipe_is_for_the_user_alone() {
        assert_eq!(
            user_only_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)"
        );
        let a = fresh_name();
        assert!(a.starts_with(r"\\.\pipe\warden-"));
        assert_ne!(a, fresh_name());
    }

    #[test]
    fn sending_with_no_core_fails_at_once() {
        let dir = crate::testdir::Dir::new("pipe-none");
        let started = std::time::Instant::now();
        let m = Message {
            account: "a1".into(),
            event: HookEvent::default(),
        };
        assert!(send_with_timeout(dir.path(), m.clone()).is_err());
        // A stale file naming a pipe that is gone.
        std::fs::write(
            dir.path().join(CONNECTION_FILE),
            r#"{"pipe":"\\\\.\\pipe\\warden-0-gone","pid":1}"#,
        )
        .expect("write");
        assert!(send_with_timeout(dir.path(), m).is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
