//! Live sessions, from the status hook's events: each session's state (working, waiting for you, idle, done), what
//! it is doing now, its subagents, the account and project it belongs to, and two hours of activity per account for
//! the Overview's sparklines.
//!
//! Only state, tool names and subagent types are kept, and each session's own name ([`crate::titles`]). An
//! event's prompt, tool input and every other field are skipped while it is parsed, never stored; the transcript
//! path is used only to read the session's title. Memory is capped: at most [`MAX_SESSIONS`] sessions, finished ones
//! dropped after [`DONE_KEPT_MS`], and [`BINS`] five-minute bins of activity per account.

use crate::model::Provider;
use crate::pipe::HookEvent;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Sessions kept at most; the oldest finished, then the longest quiet, make room.
pub const MAX_SESSIONS: usize = 200;
/// How long a finished session stays in the table.
pub const DONE_KEPT_MS: i64 = 10 * 60_000;
/// A session with no event for this long is taken to be gone (its CLI ended without saying so). Only for sessions
/// whose CLI process Warden could not find to watch: a watched session ends when its process exits.
pub const QUIET_GONE_MS: i64 = 24 * 60 * 60_000;
/// How recently a session must have been written to for a CLI's own session list to add it, and how long such a
/// session stays after its last write while no hook event has taken it over.
pub const LISTED_KEPT_MS: i64 = 30 * 60_000;
/// One sparkline bin: five minutes.
pub const BIN_MS: i64 = 5 * 60_000;
/// Two hours of bins.
pub const BINS: usize = 24;
/// How long after a turn stops (or a session ends, or asks for permission) a tool event is taken to be a late
/// delivery from before it. The status hook runs in the background, so its events can arrive out of order;
/// `warden event` gives up after a second, so nothing is later than that and a bit. Also how long a session's
/// events may still arrive after its process exited.
pub const LATE_MS: i64 = 2_000;
/// The longest tool or subagent name kept.
const MAX_NAME: usize = 64;
/// Claude's question tool: while it is open (from its start to its end), the session waits for you.
pub const QUESTION_TOOL: &str = "AskUserQuestion";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Working,
    Waiting,
    Idle,
    Done,
}

/// Where a session is known from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The status hook reports it live.
    Hook,
    /// Seen in the CLI's own session list (no status hook reported it); its state is not known.
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Subagent {
    #[serde(skip)]
    id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
}

/// The project a session works in, worked out from its folder.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    /// The repository's (or folder's) name.
    pub project: Option<String>,
    /// The session's name: its own (Claude's title, Codex's thread name), else its branch in words, else its
    /// folder's name.
    pub name: Option<String>,
}

impl Place {
    /// The place named by the session's own title, when it has one.
    pub fn titled(&self, title: Option<&str>) -> Place {
        match title {
            Some(title) => Place {
                name: Some(title.to_owned()),
                ..self.clone()
            },
            None => self.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub parent_id: Option<String>,
    pub provider: Provider,
    /// The account whose folder it runs in. It counts for that account (its now chip and sparkline) unless
    /// `signed_in_as` is set.
    pub account_id: String,
    /// The email the folder was signed in as when the session started, when that is not the account's own: the
    /// session is shown with it, counts for no account and gets no routing.
    pub signed_in_as: Option<String>,
    pub state: SessionState,
    pub doing: String,
    /// Where it works, named by its own title when it has one.
    #[serde(flatten)]
    pub place: Place,
    /// Where it works, worked out from its folder alone.
    #[serde(skip)]
    folder_place: Place,
    /// Its own name, when it has one.
    #[serde(skip)]
    title: Option<String>,
    /// Claude's question tool is open.
    #[serde(skip)]
    asking: bool,
    pub started_at: i64,
    pub last_event_at: i64,
    pub ended_at: Option<i64>,
    pub subagents: Vec<Subagent>,
    pub source: Source,
    /// The tool in use, for the "doing now" text only; never shown on its own.
    #[serde(skip)]
    tool: Option<String>,
    /// When the turn last stopped, the session ended or it asked for permission or a question, until the next prompt.
    #[serde(skip)]
    settled_at: Option<i64>,
    /// The process id of the session's CLI while Warden watches it for its exit.
    #[serde(skip)]
    process: Option<u32>,
    /// Whether Warden looked for the session's CLI process since the session (re)started.
    #[serde(skip)]
    process_looked: bool,
}

/// What a hook event says, with everything else left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Start,
    Prompt,
    ToolStart(Option<String>),
    ToolEnd(Option<String>),
    /// Asks for permission to use a tool.
    Permission(Option<String>),
    /// Asks the user a question.
    Question,
    /// The turn ended.
    Stop,
    SubagentStart {
        id: Option<String>,
        kind: String,
    },
    SubagentStop {
        id: Option<String>,
        kind: String,
    },
    End,
    /// Known to be from a live session, but changes nothing shown.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub session_id: String,
    pub kind: EventKind,
    pub cwd: Option<PathBuf>,
    /// Claude's transcript of the session: a path only, read for the session's title alone.
    pub transcript: Option<PathBuf>,
}

/// Keeps a name short and printable (tool and subagent names come from the hook, MCP tools from anywhere).
pub(crate) fn clean_name(s: &str) -> Option<String> {
    let s: String = s
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// Reads one hook event from a hook's raw input, keeping only what [`HookEvent`] keeps. `Err` for input that is not
/// a hook event.
pub fn parse_event(body: impl Read) -> Result<Event, String> {
    let hook = HookEvent::from_hook_input(body).ok_or("not a hook event")?;
    event_from(&hook)
}

/// What a hook event from Claude or Codex says. `Err` for one with no session id or event name.
pub fn event_from(wire: &HookEvent) -> Result<Event, String> {
    let session_id = wire
        .session_id
        .as_deref()
        .and_then(clean_name)
        .ok_or("no session id")?;
    let name = wire.hook_event_name.as_deref().ok_or("no event name")?;
    let tool = wire.tool_name.as_deref().and_then(clean_name);
    let agent = || {
        (
            wire.agent_id.as_deref().and_then(clean_name),
            wire.agent_type
                .as_deref()
                .and_then(clean_name)
                .unwrap_or_else(|| "subagent".into()),
        )
    };
    let kind = match name {
        "SessionStart" => EventKind::Start,
        "UserPromptSubmit" => EventKind::Prompt,
        "PreToolUse" => EventKind::ToolStart(tool),
        "PostToolUse" | "PostToolUseFailure" | "PostToolBatch" => EventKind::ToolEnd(tool),
        "PermissionRequest" => EventKind::Permission(tool),
        "Notification" => match wire.notification_type.as_deref() {
            Some("permission_prompt") => EventKind::Permission(None),
            Some("elicitation_dialog" | "agent_needs_input") => EventKind::Question,
            Some("idle_prompt") => EventKind::Stop,
            _ => EventKind::Other,
        },
        "Elicitation" => EventKind::Question,
        "Stop" | "StopFailure" | "Interrupt" => EventKind::Stop,
        "SubagentStart" => {
            let (id, kind) = agent();
            EventKind::SubagentStart { id, kind }
        }
        "SubagentStop" => {
            let (id, kind) = agent();
            EventKind::SubagentStop { id, kind }
        }
        "SessionEnd" => EventKind::End,
        _ => EventKind::Other,
    };
    Ok(Event {
        session_id,
        kind,
        cwd: wire
            .cwd
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(PathBuf::from),
        transcript: wire
            .transcript_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
    })
}

enum ToolGroup {
    Edit,
    Read,
    Command,
    Web,
    Agents,
    Plan,
    Mcp(String),
    Other,
}

fn group(tool: &str) -> ToolGroup {
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "apply_patch" => ToolGroup::Edit,
        "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" | "view_image" | "read_file"
        | "list_dir" => ToolGroup::Read,
        "Bash" | "PowerShell" | "shell" | "local_shell" | "exec_command" | "unified_exec"
        | "write_stdin" => ToolGroup::Command,
        "WebFetch" | "WebSearch" | "web_search" => ToolGroup::Web,
        "Task" | "Agent" | "spawn_agent" => ToolGroup::Agents,
        "TodoWrite" | "update_plan" => ToolGroup::Plan,
        t => match t.strip_prefix("mcp__").and_then(|r| r.split("__").next()) {
            Some(server) if !server.is_empty() => ToolGroup::Mcp(server.replace('_', " ")),
            _ => ToolGroup::Other,
        },
    }
}

/// "Doing now" while working with a tool: "Editing files", "Running commands".
pub fn doing_with(tool: Option<&str>) -> String {
    let Some(tool) = tool else {
        return "Thinking".into();
    };
    match group(tool) {
        ToolGroup::Edit => "Editing files".into(),
        ToolGroup::Read => "Reading files".into(),
        ToolGroup::Command => "Running commands".into(),
        ToolGroup::Web => "Searching the web".into(),
        ToolGroup::Agents => "Running subagents".into(),
        ToolGroup::Plan => "Planning".into(),
        ToolGroup::Mcp(server) => format!("Using {server}"),
        ToolGroup::Other => format!("Using {tool}"),
    }
}

/// "Doing now" while it waits for permission: "Asks to run a command".
pub fn asks_for(tool: Option<&str>) -> String {
    let Some(tool) = tool else {
        return "Asks for permission".into();
    };
    match group(tool) {
        ToolGroup::Edit => "Asks to edit files".into(),
        ToolGroup::Read => "Asks to read files".into(),
        ToolGroup::Command => "Asks to run a command".into(),
        ToolGroup::Web => "Asks to use the web".into(),
        ToolGroup::Agents => "Asks to start a subagent".into(),
        ToolGroup::Plan => "Asks to update its plan".into(),
        ToolGroup::Mcp(server) => format!("Asks to use {server}"),
        ToolGroup::Other => format!("Asks to use {tool}"),
    }
}

pub const IDLE_TEXT: &str = "Waiting for your next message";
pub const QUESTION_TEXT: &str = "Asks you a question";
pub const DONE_TEXT: &str = "Ended";
pub const LISTED_TEXT: &str = "Seen in the session list, no status hook";

/// One five-minute bin of an account's activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bin {
    /// The bin's start (ms since the epoch, a multiple of [`BIN_MS`]).
    pub at: i64,
    /// The most sessions working at once during the bin.
    pub working: u32,
    /// Sessions working at the bin's last change; later bins with no events carry it on.
    pub last: u32,
    /// A session started waiting for you during the bin.
    pub waited: bool,
}

/// Two hours of one account's activity, kept as the bins that saw a change (at most [`BINS`] + 1, so the one
/// before the window can still carry its count in).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Activity(VecDeque<Bin>);

impl Activity {
    fn record(&mut self, now: i64, working: u32, waited: bool) {
        let at = now - now.rem_euclid(BIN_MS);
        match self.0.back_mut() {
            Some(b) if b.at == at => {
                b.working = b.working.max(working);
                b.last = working;
                b.waited |= waited;
            }
            Some(b) if b.at > at => {} // The clock went back: keep what is there.
            prev => {
                let carried = prev.map_or(0, |b| b.last);
                if carried == working && !waited && self.0.back().is_some() {
                    return; // Nothing new; later bins carry the count on.
                }
                self.0.push_back(Bin {
                    at,
                    working: working.max(carried),
                    last: working,
                    waited,
                });
                while self.0.len() > BINS + 1 {
                    self.0.pop_front();
                }
            }
        }
    }

    pub fn bins(&self) -> impl Iterator<Item = &Bin> {
        self.0.iter()
    }
}

/// Which main session started each delegated run, kept in the data folder (`session-parents.json`) so the sessions tree
/// is whole again after Warden restarts, before each run's next hook event. Session ids only: at most
/// [`MAX_PARENT_LINKS`], each forgotten after [`PARENT_LINK_MS`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentLinks {
    links: Vec<ParentLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentLink {
    provider: Provider,
    id: String,
    parent: String,
    at: i64,
}

pub const PARENT_LINKS_FILE: &str = "session-parents.json";
/// Parent links kept at most, the oldest dropped first.
pub const MAX_PARENT_LINKS: usize = MAX_SESSIONS;
/// How long a parent link is kept: as long as a quiet session is still shown.
pub const PARENT_LINK_MS: i64 = QUIET_GONE_MS;

impl ParentLinks {
    /// The main session that started this run, if known and learned within [`PARENT_LINK_MS`].
    pub fn get(&self, provider: Provider, id: &str, now: i64) -> Option<&str> {
        self.links
            .iter()
            .rev()
            .find(|l| l.provider == provider && l.id == id && now - l.at <= PARENT_LINK_MS)
            .map(|l| l.parent.as_str())
    }

    fn remember(&mut self, provider: Provider, id: &str, parent: &str, now: i64) {
        self.links
            .retain(|l| !(l.provider == provider && l.id == id));
        self.links.push(ParentLink {
            provider,
            id: id.to_owned(),
            parent: parent.to_owned(),
            at: now,
        });
        self.prune(now);
    }

    /// Drops old links and any whose ids are not session ids (a hand-edited file), keeping the newest few.
    fn prune(&mut self, now: i64) {
        self.links.retain(|l| {
            now - l.at <= PARENT_LINK_MS
                && crate::pipe::valid_session_id(&l.id)
                && crate::pipe::valid_session_id(&l.parent)
        });
        let extra = self.links.len().saturating_sub(MAX_PARENT_LINKS);
        self.links.drain(..extra);
    }
}

/// Every session Warden knows, and each account's activity.
#[derive(Debug, Default)]
pub struct Sessions {
    list: Vec<Session>,
    activity: HashMap<String, Activity>,
    /// Sessions' own names, also for sessions not seen yet (Codex names a session in its index, maybe before its
    /// first event reaches Warden). At most [`MAX_NAMES`], the oldest given dropped first.
    names: HashMap<(Provider, String), String>,
    named_order: VecDeque<(Provider, String)>,
    /// Which main session started each delegated run, kept across restarts.
    parents: ParentLinks,
    /// Parent ids looked up by id since the start, so one that cannot be found is not looked up on every event. At
    /// most [`MAX_LOOKED_UP`], the oldest dropped first.
    looked_up: VecDeque<String>,
    /// The main sessions those lookups found, so one dropped while no run under it was shown comes back with the
    /// run without another lookup. At most [`MAX_LOOKED_UP`], the oldest found dropped first; one the hook reports
    /// is forgotten, as its own events say what it is from then on.
    found: VecDeque<Found>,
}

/// A main session found by its id ([`Sessions::add_found`]), as it was added.
#[derive(Debug, Clone)]
struct Found {
    provider: Provider,
    account_id: String,
    signed_in_as: Option<String>,
    id: String,
    last_at: i64,
    place: Place,
}

/// Sessions' own names kept at most.
pub const MAX_NAMES: usize = 2 * MAX_SESSIONS;
/// Parent ids remembered as looked up, and main sessions remembered as found, at most.
pub const MAX_LOOKED_UP: usize = 2 * MAX_SESSIONS;

impl Sessions {
    /// The parent links saved before a restart: a session that comes back sits under its main session again.
    pub fn restore_parents(&mut self, mut links: ParentLinks, now: i64) {
        links.prune(now);
        self.parents = links;
    }

    /// The parent links to save, after one changed.
    pub fn parent_links(&self) -> &ParentLinks {
        &self.parents
    }

    fn known_parent(&self, provider: Provider, id: &str, now: i64) -> Option<String> {
        self.parents.get(provider, id, now).map(str::to_owned)
    }

    pub fn get(&self, provider: Provider, id: &str) -> Option<&Session> {
        self.list
            .iter()
            .find(|s| s.provider == provider && s.id == id)
    }

    /// Whether an event for this session would start a new one (so its account and project are worked out first).
    pub fn is_new(&self, provider: Provider, id: &str) -> bool {
        self.get(provider, id)
            .is_none_or(|s| s.source == Source::List)
    }

    /// The sessions to show at `now`: finished ones only for a while, long-quiet ones not at all.
    pub fn shown(&self, now: i64) -> impl Iterator<Item = &Session> {
        self.list.iter().filter(move |s| self.kept(s, now))
    }

    /// Whether a session is still shown at `now`: by the usual rule ([`keep`]), or a listed main session while one
    /// of the delegated runs under it is shown, however long ago it was last written to.
    fn kept(&self, s: &Session, now: i64) -> bool {
        keep(s, now) || self.holds_a_run(s, now)
    }

    /// A listed main session (not ended, signed in as main) with a run under it that is shown.
    fn holds_a_run(&self, s: &Session, now: i64) -> bool {
        listed_main(s)
            && self.list.iter().any(|c| {
                c.account_id != "main"
                    && c.parent_id.as_deref() == Some(s.id.as_str())
                    && keep(c, now)
            })
    }

    /// Drops the sessions no longer shown. Returns whether any went.
    fn drop_gone(&mut self, now: i64) -> bool {
        let gone: Vec<bool> = self.list.iter().map(|s| !self.kept(s, now)).collect();
        if !gone.contains(&true) {
            return false;
        }
        let mut gone = gone.into_iter();
        self.list.retain(|_| !gone.next().unwrap_or(false));
        true
    }

    /// The main sessions that shown runs name as their parent (through a saved link) but that are not known, each
    /// given once per start: Warden looks them up by id in the accounts' own session files.
    pub fn parents_to_find(&mut self, now: i64) -> Vec<String> {
        let named: Vec<String> = self
            .shown(now)
            .filter(|s| s.account_id != "main")
            .filter_map(|s| s.parent_id.clone())
            .collect();
        named.into_iter().filter(|p| self.take_lookup(p)).collect()
    }

    /// The main session a shown run the hook reports names as its parent (`named`, from its event, or else its saved
    /// link), when that session is not known and was not looked up yet since the start.
    pub fn parent_to_find(
        &mut self,
        provider: Provider,
        id: &str,
        named: Option<&str>,
        now: i64,
    ) -> Option<String> {
        let parent = self.named_parent(provider, id, named, now)?;
        self.take_lookup(&parent).then_some(parent)
    }

    /// The main session a shown run names as its parent, as for [`Sessions::parent_to_find`].
    fn named_parent(
        &self,
        provider: Provider,
        id: &str,
        named: Option<&str>,
        now: i64,
    ) -> Option<String> {
        let s = self.get(provider, id)?;
        if s.account_id == "main" || !keep(s, now) {
            return None;
        }
        named
            .map(str::to_owned)
            .or_else(|| s.parent_id.clone())
            .or_else(|| self.known_parent(provider, id, now))
            .filter(|p| p != id)
    }

    /// Adds a main session found by its id for a run's parent link, as a listed session ([`Sessions::add_listed_as`]),
    /// and remembers it, so it can come back without another lookup ([`Sessions::restore_found_parent`]). Returns
    /// whether it was added.
    pub fn add_found(
        &mut self,
        provider: Provider,
        account_id: &str,
        signed_in_as: Option<&str>,
        (id, last_at, place): (String, i64, Place),
        now: i64,
    ) -> bool {
        self.found.retain(|f| f.id != id);
        self.found.push_back(Found {
            provider,
            account_id: account_id.to_owned(),
            signed_in_as: signed_in_as.map(str::to_owned),
            id: id.clone(),
            last_at,
            place: place.clone(),
        });
        while self.found.len() > MAX_LOOKED_UP {
            self.found.pop_front();
        }
        self.add_listed_as(
            provider,
            account_id,
            signed_in_as,
            vec![(id, last_at, place)],
            now,
        )
    }

    /// Brings back the main session a shown run names as its parent (as for [`Sessions::parent_to_find`]) when it is
    /// not known but was found earlier since the start: dropped while no run under it was shown, it returns with the
    /// run, from what was found then, without another lookup. Returns whether it came back.
    pub fn restore_found_parent(
        &mut self,
        provider: Provider,
        id: &str,
        named: Option<&str>,
        now: i64,
    ) -> bool {
        let Some(parent) = self.named_parent(provider, id, named, now) else {
            return false;
        };
        if self.list.iter().any(|s| s.id == parent) {
            return false;
        }
        let Some(f) = self.found.iter().find(|f| f.id == parent).cloned() else {
            return false;
        };
        self.add_listed_as(
            f.provider,
            &f.account_id,
            f.signed_in_as.as_deref(),
            vec![(f.id, f.last_at, f.place)],
            now,
        )
    }

    /// Notes that `parent` is looked up now, if it is a session id that is not known and was not looked up before.
    fn take_lookup(&mut self, parent: &str) -> bool {
        if !crate::pipe::valid_session_id(parent)
            || self.list.iter().any(|s| s.id == parent)
            || self.looked_up.iter().any(|p| p == parent)
        {
            return false;
        }
        self.looked_up.push_back(parent.to_owned());
        while self.looked_up.len() > MAX_LOOKED_UP {
            self.looked_up.pop_front();
        }
        true
    }

    /// A snapshot clears links whose main session is no longer shown, even with no new hook event.
    pub fn shown_with_parents(&self, now: i64) -> Vec<Session> {
        self.shown(now)
            .map(|s| {
                let mut s = s.clone();
                if !s
                    .parent_id
                    .as_deref()
                    .is_some_and(|id| self.valid_parent(id, now))
                {
                    s.parent_id = None;
                }
                s
            })
            .collect()
    }

    fn valid_parent(&self, id: &str, now: i64) -> bool {
        crate::pipe::valid_session_id(id)
            && self
                .shown(now)
                .any(|s| s.id == id && s.account_id == "main" && s.signed_in_as.is_none())
    }

    /// A listed main session (not ended, signed in as main), shown or not.
    fn is_listed_main(&self, id: &str) -> bool {
        self.list.iter().any(|s| s.id == id && listed_main(s))
    }

    /// Only main sessions may parent delegated runs; a late event without a link preserves a known parent.
    pub fn set_parent(
        &mut self,
        provider: Provider,
        id: &str,
        parent: Option<&str>,
        now: i64,
    ) -> bool {
        // A listed main session older than its window is shown again once a shown run sits under it.
        let child_kept = self.get(provider, id).is_some_and(|s| keep(s, now));
        let parent = parent.filter(|p| {
            *p != id && (self.valid_parent(p, now) || child_kept && self.is_listed_main(p))
        });
        let Some(s) = self
            .list
            .iter_mut()
            .find(|s| s.provider == provider && s.id == id)
        else {
            return false;
        };
        if s.account_id == "main" {
            return false;
        }
        if let Some(parent) = parent
            && s.parent_id.as_deref() != Some(parent)
        {
            s.parent_id = Some(parent.to_owned());
            self.parents.remember(provider, id, parent, now);
            return true;
        }
        false
    }

    /// Live main sessions with a known process, for the process-chain fallback (never finished parents).
    pub fn main_processes(&self) -> Vec<(u32, String)> {
        self.list
            .iter()
            .filter(|s| s.account_id == "main" && s.signed_in_as.is_none() && s.ended_at.is_none())
            .filter_map(|s| s.process.map(|p| (p, s.id.clone())))
            .collect()
    }

    /// Sessions not ended whose CLI process is known, each with it: what memory is sampled for.
    pub fn with_processes(&self) -> impl Iterator<Item = (&Session, u32)> {
        self.list
            .iter()
            .filter(|s| s.ended_at.is_none())
            .filter_map(|s| s.process.map(|p| (s, p)))
    }

    pub fn activity(&self) -> &HashMap<String, Activity> {
        &self.activity
    }

    fn working(&self, account_id: &str) -> u32 {
        let n = self
            .list
            .iter()
            .filter(|s| {
                s.account_id == account_id
                    && s.signed_in_as.is_none()
                    && s.state == SessionState::Working
            })
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// Applies one event from a folder signed in as its account. See [`Sessions::apply_as`].
    pub fn apply(
        &mut self,
        provider: Provider,
        account_id: &str,
        place: impl FnOnce() -> Place,
        event: &Event,
        now: i64,
    ) -> bool {
        self.apply_as(provider, account_id, None, place, event, now)
    }

    /// Applies one event from `account_id`'s folder. A session not seen before, or starting again (resumed), takes
    /// `signed_in_as`: the folder's sign-in when it is not the account's own, so the session counts for no account.
    /// A new one works in `place`. Returns whether anything shown changed.
    pub fn apply_as(
        &mut self,
        provider: Provider,
        account_id: &str,
        signed_in_as: Option<&str>,
        place: impl FnOnce() -> Place,
        event: &Event,
        now: i64,
    ) -> bool {
        let mut changed = self.drop_gone(now);
        let i = match self
            .list
            .iter()
            .position(|s| s.provider == provider && s.id == event.session_id)
        {
            Some(i) if self.list[i].source == Source::Hook => i,
            found => {
                if event.kind == EventKind::End && found.is_none() {
                    return false; // The end of a session never seen: nothing to show.
                }
                if let Some(i) = found {
                    self.list.remove(i); // A listed session the hook now reports.
                }
                // A main session found for a run's parent: its own events say what it is from now on.
                self.found
                    .retain(|f| !(f.provider == provider && f.id == event.session_id));
                self.make_room();
                changed = true;
                let title = self
                    .names
                    .get(&(provider, event.session_id.clone()))
                    .cloned();
                let folder_place = place();
                let parent_id = self.known_parent(provider, &event.session_id, now);
                self.list.push(Session {
                    id: event.session_id.clone(),
                    parent_id,
                    provider,
                    account_id: account_id.to_owned(),
                    signed_in_as: signed_in_as.map(str::to_owned),
                    state: SessionState::Idle,
                    doing: IDLE_TEXT.into(),
                    place: folder_place.titled(title.as_deref()),
                    folder_place,
                    title,
                    asking: false,
                    started_at: now,
                    last_event_at: now,
                    ended_at: None,
                    subagents: Vec::new(),
                    source: Source::Hook,
                    tool: None,
                    settled_at: None,
                    process: None,
                    process_looked: false,
                });
                self.list.len() - 1
            }
        };
        let s = &mut self.list[i];
        let before = (
            s.state,
            s.doing.clone(),
            s.subagents.len(),
            s.account_id.clone(),
            s.signed_in_as.clone(),
        );
        let was_waiting = s.state == SessionState::Waiting;
        s.last_event_at = now;
        // A tool event just after the turn stopped (or the session ended) is from before it: a new turn starts with a
        // prompt. A tool start just after a permission request is the tool that asks.
        let just_settled = s.settled_at.is_some_and(|t| now - t <= LATE_MS);
        let late = just_settled
            && match &event.kind {
                EventKind::ToolStart(_) => s.state != SessionState::Working,
                EventKind::ToolEnd(_) | EventKind::SubagentStart { .. } => {
                    matches!(s.state, SessionState::Idle | SessionState::Done)
                }
                _ => false,
            };
        match &event.kind {
            _ if late => {}
            EventKind::Start => {
                // A resumed session counts for whoever is signed in now, and may run in a new process.
                account_id.clone_into(&mut s.account_id);
                s.signed_in_as = signed_in_as.map(str::to_owned);
                // It may run in another process now (resumed): looked for again, so the process it runs in now is
                // the one watched, and an earlier process's exit no longer ends it.
                s.process_looked = false;
                s.process = None;
                if s.state == SessionState::Done {
                    s.state = SessionState::Idle;
                    s.ended_at = None;
                }
            }
            EventKind::Prompt => {
                s.state = SessionState::Working;
                s.tool = None;
                s.settled_at = None;
                s.asking = false;
            }
            EventKind::ToolStart(tool) if tool.as_deref() == Some(QUESTION_TOOL) => {
                s.asking = true;
            }
            EventKind::ToolStart(tool) => {
                s.state = SessionState::Working;
                s.tool = tool.clone();
            }
            EventKind::ToolEnd(tool) => {
                if tool.as_deref() == Some(QUESTION_TOOL) {
                    s.asking = false;
                }
                if s.state != SessionState::Done {
                    s.state = SessionState::Working;
                    s.tool = None; // Back to thinking until the next tool.
                }
            }
            EventKind::Permission(tool) => {
                s.state = SessionState::Waiting;
                s.settled_at = Some(now);
                s.asking |= tool.as_deref() == Some(QUESTION_TOOL);
                s.doing = asks_for(tool.as_deref().or(s.tool.as_deref()));
            }
            EventKind::Question => {
                s.state = SessionState::Waiting;
                s.settled_at = Some(now);
                s.doing = QUESTION_TEXT.into();
            }
            EventKind::Stop => {
                s.state = SessionState::Idle;
                s.settled_at = Some(now);
                s.tool = None;
                s.asking = false;
                s.subagents.clear();
            }
            EventKind::SubagentStart { id, kind } => {
                if s.state != SessionState::Waiting {
                    s.state = SessionState::Working;
                }
                if s.subagents.len() < 64 {
                    s.subagents.push(Subagent {
                        id: id.clone(),
                        kind: kind.clone(),
                    });
                }
            }
            EventKind::SubagentStop { id, kind } => {
                let at = match id {
                    Some(id) => s.subagents.iter().position(|a| a.id.as_ref() == Some(id)),
                    None => None,
                }
                .or_else(|| s.subagents.iter().position(|a| &a.kind == kind));
                if let Some(at) = at {
                    s.subagents.remove(at);
                }
            }
            EventKind::End => {
                s.state = SessionState::Done;
                s.settled_at = Some(now);
                s.ended_at = Some(now);
                s.tool = None;
                s.asking = false;
                s.subagents.clear();
            }
            EventKind::Other => {}
        }
        // While the question is open the session waits for you, whatever its other tools and subagents do (a
        // permission request of theirs shows until it is answered).
        let other_asks =
            matches!(&event.kind, EventKind::Permission(Some(t)) if t != QUESTION_TOOL);
        if s.asking && !other_asks {
            s.state = SessionState::Waiting;
            s.doing = QUESTION_TEXT.into();
        }
        match s.state {
            SessionState::Working => s.doing = doing_with(s.tool.as_deref()),
            SessionState::Idle => s.doing = IDLE_TEXT.into(),
            SessionState::Done => s.doing = DONE_TEXT.into(),
            SessionState::Waiting => {}
        }
        // A session signed in as someone else is shown but not counted, so it marks no wait either.
        let started_waiting =
            s.state == SessionState::Waiting && !was_waiting && s.signed_in_as.is_none();
        changed |= before
            != (
                s.state,
                s.doing.clone(),
                s.subagents.len(),
                s.account_id.clone(),
                s.signed_in_as.clone(),
            );
        let account = s.account_id.clone();
        // A resumed session that moved account leaves the old account's count too.
        if before.3 != account {
            let working = self.working(&before.3);
            self.activity
                .entry(before.3)
                .or_default()
                .record(now, working, false);
        }
        let working = self.working(&account);
        self.activity
            .entry(account)
            .or_default()
            .record(now, working, started_waiting);
        changed
    }

    /// Adds listed sessions from a folder signed in as its account. See [`Sessions::add_listed_as`].
    pub fn add_listed(
        &mut self,
        provider: Provider,
        account_id: &str,
        listed: Vec<(String, i64, Place)>,
        now: i64,
    ) -> bool {
        self.add_listed_as(provider, account_id, None, listed, now)
    }

    /// Adds sessions seen in a CLI's own session list, for an account no hook reports; `signed_in_as` as for
    /// [`Sessions::apply_as`]. Sessions already known are left alone. Returns whether any was added.
    pub fn add_listed_as(
        &mut self,
        provider: Provider,
        account_id: &str,
        signed_in_as: Option<&str>,
        listed: Vec<(String, i64, Place)>,
        now: i64,
    ) -> bool {
        self.drop_gone(now);
        let mut added = false;
        for (id, last_at, place) in listed {
            if self.get(provider, &id).is_some() {
                continue;
            }
            self.make_room();
            let title = self.names.get(&(provider, id.clone())).cloned();
            let parent_id = self.known_parent(provider, &id, now);
            self.list.push(Session {
                parent_id,
                id,
                provider,
                account_id: account_id.to_owned(),
                signed_in_as: signed_in_as.map(str::to_owned),
                state: SessionState::Idle,
                doing: LISTED_TEXT.into(),
                place: place.titled(title.as_deref()),
                folder_place: place,
                title,
                asking: false,
                started_at: last_at,
                last_event_at: last_at,
                ended_at: None,
                subagents: Vec::new(),
                source: Source::List,
                tool: None,
                settled_at: None,
                process: None,
                process_looked: false,
            });
            added = true;
        }
        added
    }

    /// The account's folder is its own again (the new sign-in accepted, or the old one back): its sessions shown as
    /// signed in as someone else count for it from now on. Returns whether any did.
    pub fn settle_account(&mut self, account_id: &str, now: i64) -> bool {
        let mut settled = false;
        for s in &mut self.list {
            if s.account_id == account_id && s.signed_in_as.take().is_some() {
                settled = true;
            }
        }
        if settled {
            let working = self.working(account_id);
            self.activity
                .entry(account_id.to_owned())
                .or_default()
                .record(now, working, false);
        }
        settled
    }

    /// A changed folder sign-in counts for no account, including sessions that were already running.
    pub fn changed_account(&mut self, account_id: &str, email: &str, now: i64) -> bool {
        let mut changed = false;
        for session in &mut self.list {
            if session.account_id == account_id && session.signed_in_as.as_deref() != Some(email) {
                session.signed_in_as = Some(email.to_owned());
                changed = true;
            }
        }
        if changed {
            self.activity
                .entry(account_id.to_owned())
                .or_default()
                .record(now, 0, false);
        }
        changed
    }

    /// Gives sessions their own names (a Claude title read from a transcript, Codex's thread names), including
    /// sessions not seen yet. Returns whether anything shown changed.
    pub fn name(&mut self, provider: Provider, names: Vec<(String, String)>) -> bool {
        let mut changed = false;
        for (id, title) in names {
            let key = (provider, id);
            if self.names.insert(key.clone(), title.clone()).is_none() {
                self.named_order.push_back(key.clone());
                while self.named_order.len() > MAX_NAMES {
                    if let Some(old) = self.named_order.pop_front() {
                        self.names.remove(&old);
                    }
                }
            }
            if let Some(s) = self
                .list
                .iter_mut()
                .find(|s| s.provider == provider && s.id == key.1)
                && s.title.as_deref() != Some(title.as_str())
            {
                s.place = s.folder_place.titled(Some(&title));
                s.title = Some(title);
                changed = true;
            }
        }
        changed
    }

    /// Whether Warden should look for this session's CLI process now: it reports through the hook and was not
    /// looked for since it (re)started.
    pub fn needs_process(&self, provider: Provider, id: &str) -> bool {
        self.get(provider, id)
            .is_some_and(|s| s.source == Source::Hook && !s.process_looked)
    }

    /// Notes the session's CLI process (`None`: not found), which Warden then watches for its exit.
    pub fn set_process(&mut self, provider: Provider, id: &str, pid: Option<u32>) {
        if let Some(s) = self
            .list
            .iter_mut()
            .find(|s| s.provider == provider && s.id == id)
        {
            s.process_looked = true;
            s.process = pid;
        }
    }

    /// A watched CLI process exited (a moment ago, so late events had their chance). Its sessions end: a Codex one,
    /// which never reports its end, becomes done; a Claude one that did not report its end crashed or was killed, so
    /// it goes at once. Returns whether anything shown changed.
    pub fn process_exited(&mut self, pid: u32, now: i64) -> bool {
        let mut changed = false;
        let mut accounts = Vec::new();
        self.list.retain_mut(|s| {
            if s.process != Some(pid) {
                return true;
            }
            s.process = None;
            if s.state == SessionState::Done {
                return true;
            }
            changed = true;
            accounts.push(s.account_id.clone());
            match s.provider {
                Provider::Claude => false,
                Provider::Codex => {
                    s.state = SessionState::Done;
                    s.doing = DONE_TEXT.into();
                    s.settled_at = Some(now);
                    s.ended_at = Some(now);
                    s.tool = None;
                    s.subagents.clear();
                    true
                }
            }
        });
        for account in accounts {
            let working = self.working(&account);
            self.activity
                .entry(account)
                .or_default()
                .record(now, working, false);
        }
        changed
    }

    /// Forgets an account's sessions and activity (the account was removed).
    pub fn forget_account(&mut self, account_id: &str) {
        self.list.retain(|s| s.account_id != account_id);
        self.activity.remove(account_id);
    }

    fn make_room(&mut self) {
        while self.list.len() >= MAX_SESSIONS {
            let victim = self
                .list
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| (s.state != SessionState::Done, s.last_event_at))
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.list.remove(i);
                }
                None => break,
            }
        }
    }
}

/// A main session seen only in its CLI's list, not ended, signed in as main: one that may hold delegated runs.
fn listed_main(s: &Session) -> bool {
    s.source == Source::List
        && s.ended_at.is_none()
        && s.account_id == "main"
        && s.signed_in_as.is_none()
}

fn keep(s: &Session, now: i64) -> bool {
    match s.ended_at {
        Some(ended) => now - ended < DONE_KEPT_MS,
        None if s.source == Source::List => now - s.last_event_at < LISTED_KEPT_MS,
        None => s.process.is_some() || now - s.last_event_at < QUIET_GONE_MS,
    }
}

#[cfg(test)]
mod parent_tests {
    use super::*;

    fn event(id: &str, kind: EventKind) -> Event {
        Event {
            session_id: id.into(),
            kind,
            cwd: None,
            transcript: None,
        }
    }

    #[test]
    fn a_restart_keeps_each_run_under_its_main_session() {
        let parent_of = |sessions: &Sessions, id: &str, now: i64| {
            sessions
                .shown_with_parents(now)
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.parent_id.clone())
        };
        let mut before = Sessions::default();
        before.apply(
            Provider::Claude,
            "main",
            Place::default,
            &event("parent", EventKind::Start),
            0,
        );
        before.apply(
            Provider::Codex,
            "a2",
            Place::default,
            &event("child", EventKind::Start),
            0,
        );
        assert!(before.set_parent(Provider::Codex, "child", Some("parent"), 1));
        // What is saved and read back after the restart.
        let saved: ParentLinks =
            serde_json::from_str(&serde_json::to_string(before.parent_links()).expect("json"))
                .expect("links");
        assert_eq!(saved.get(Provider::Codex, "child", 1), Some("parent"));
        assert_eq!(
            saved.get(Provider::Claude, "child", 1),
            None,
            "per provider"
        );

        // After the restart: both come back from the CLIs' session lists, the run under its main session at once.
        let mut after = Sessions::default();
        after.restore_parents(saved.clone(), 2);
        after.add_listed(
            Provider::Claude,
            "main",
            vec![("parent".into(), 2, Place::default())],
            2,
        );
        after.add_listed(
            Provider::Codex,
            "a2",
            vec![("child".into(), 2, Place::default())],
            2,
        );
        assert_eq!(parent_of(&after, "child", 3).as_deref(), Some("parent"));
        // Or the run's next hook event arrives first: the same.
        let mut hooked = Sessions::default();
        hooked.restore_parents(saved.clone(), 2);
        hooked.apply(
            Provider::Claude,
            "main",
            Place::default,
            &event("parent", EventKind::Start),
            2,
        );
        hooked.apply(
            Provider::Codex,
            "a2",
            Place::default,
            &event("child", EventKind::Prompt),
            2,
        );
        assert_eq!(parent_of(&hooked, "child", 3).as_deref(), Some("parent"));
        // A link older than a day is not used, even while Warden keeps running (no restart to prune it).
        let mut running = Sessions::default();
        running.restore_parents(saved.clone(), 2);
        let late = 1 + PARENT_LINK_MS + 1;
        running.add_listed(
            Provider::Claude,
            "main",
            vec![("parent".into(), late, Place::default())],
            late,
        );
        running.add_listed(
            Provider::Codex,
            "a2",
            vec![("child".into(), late, Place::default())],
            late,
        );
        assert_eq!(parent_of(&running, "child", late), None);
        // Old links, and links that are not session ids, are forgotten.
        let mut stale = Sessions::default();
        stale.restore_parents(saved, 1 + PARENT_LINK_MS + 1);
        assert_eq!(
            stale
                .parent_links()
                .get(Provider::Codex, "child", 1 + PARENT_LINK_MS + 1),
            None
        );
        let bad: ParentLinks = serde_json::from_str(
            r#"{"links":[{"provider":"codex","id":"child","parent":"bad id","at":5}]}"#,
        )
        .expect("links");
        let mut cleaned = Sessions::default();
        cleaned.restore_parents(bad, 5);
        assert_eq!(
            cleaned.parent_links().get(Provider::Codex, "child", 5),
            None
        );
    }

    /// Links saved before a restart: `child` (a Codex run on a2) under main's `parent`.
    fn saved_link(now: i64) -> ParentLinks {
        let mut links = ParentLinks::default();
        links.remember(Provider::Codex, "child", "parent", now);
        links
    }

    fn shown_parent(sessions: &Sessions, id: &str, now: i64) -> Option<String> {
        sessions
            .shown_with_parents(now)
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.parent_id.clone())
    }

    #[test]
    fn an_idle_main_session_is_found_by_id_once_and_stays_while_a_run_under_it_is_shown() {
        let hour = 60 * 60_000;
        let now = 10 * hour;
        let mut s = Sessions::default();
        s.restore_parents(saved_link(now - hour), now);
        // The run was written to a minute ago; its main session an hour ago, so it is not in main's list.
        s.add_listed(
            Provider::Codex,
            "a2",
            vec![("child".into(), now - 60_000, Place::default())],
            now,
        );
        assert_eq!(shown_parent(&s, "child", now), None);
        assert_eq!(s.parents_to_find(now), vec!["parent".to_owned()]);
        // Asked once per start: not found, it is not looked up again, at start or on an event.
        assert!(s.parents_to_find(now).is_empty());
        assert_eq!(
            s.parent_to_find(Provider::Codex, "child", Some("parent"), now),
            None
        );

        // Found: added as a listed session, older than the lists' window, and the run sits under it.
        assert!(s.add_listed(
            Provider::Claude,
            "main",
            vec![("parent".into(), now - hour, Place::default())],
            now,
        ));
        let parent = s.get(Provider::Claude, "parent").expect("added").clone();
        assert_eq!(
            (parent.state, parent.doing.as_str(), parent.source),
            (SessionState::Idle, LISTED_TEXT, Source::List)
        );
        assert_eq!(parent.last_event_at, now - hour);
        assert_eq!(shown_parent(&s, "child", now).as_deref(), Some("parent"));
        assert_eq!(s.shown(now).count(), 2);
        // Another account's listing keeps it while the run is shown.
        s.add_listed(Provider::Codex, "a3", Vec::new(), now + LISTED_KEPT_MS / 2);
        assert!(s.get(Provider::Claude, "parent").is_some());
        // Once the run is no longer shown, the usual rule drops it too.
        let later = now - 60_000 + LISTED_KEPT_MS;
        assert_eq!(s.shown(later).count(), 0);
        s.add_listed(Provider::Codex, "a3", Vec::new(), later);
        assert!(s.get(Provider::Claude, "parent").is_none());
        // A hook event for a found parent takes it over.
        let mut t = Sessions::default();
        t.restore_parents(saved_link(now - hour), now);
        t.add_listed(
            Provider::Codex,
            "a2",
            vec![("child".into(), now - 60_000, Place::default())],
            now,
        );
        t.add_listed(
            Provider::Claude,
            "main",
            vec![("parent".into(), now - hour, Place::default())],
            now,
        );
        t.apply(
            Provider::Claude,
            "main",
            Place::default,
            &event("parent", EventKind::Prompt),
            now,
        );
        let parent = t.get(Provider::Claude, "parent").expect("hooked");
        assert_eq!(
            (parent.source, parent.state),
            (Source::Hook, SessionState::Working)
        );
        assert_eq!(shown_parent(&t, "child", now).as_deref(), Some("parent"));
    }

    #[test]
    fn a_found_main_session_comes_back_with_its_run_without_another_lookup() {
        let hour = 60 * 60_000;
        let now = 10 * hour;
        let mut s = Sessions::default();
        s.restore_parents(saved_link(now - hour), now);
        // At start: the run written to a minute ago, its main session found by id, an hour idle.
        s.add_listed(
            Provider::Codex,
            "a2",
            vec![("child".into(), now - 60_000, Place::default())],
            now,
        );
        assert_eq!(s.parents_to_find(now), vec!["parent".to_owned()]);
        let place = Place {
            project: Some("shop".into()),
            name: None,
        };
        assert!(s.add_found(
            Provider::Claude,
            "main",
            None,
            ("parent".into(), now - hour, place.clone()),
            now,
        ));
        assert_eq!(shown_parent(&s, "child", now).as_deref(), Some("parent"));
        // Nothing to bring back while it is known.
        assert!(!s.restore_found_parent(Provider::Codex, "child", None, now));

        // 30 minutes without a hook event: the listed run goes, and its main session with it.
        let later = now + LISTED_KEPT_MS;
        assert_eq!(s.shown(later).count(), 0);
        // The run's next event drops both before it is added again, from the hook.
        s.apply(
            Provider::Codex,
            "a2",
            Place::default,
            &event("child", EventKind::Prompt),
            later,
        );
        assert!(s.get(Provider::Claude, "parent").is_none());
        // It was looked up already this start, so it is not looked up again...
        assert_eq!(
            s.parent_to_find(Provider::Codex, "child", None, later),
            None
        );
        // ...but comes back from what was found, and the run sits under it again.
        assert!(s.restore_found_parent(Provider::Codex, "child", None, later));
        let parent = s.get(Provider::Claude, "parent").expect("back").clone();
        assert_eq!(
            (
                parent.account_id.as_str(),
                parent.source,
                parent.last_event_at,
                parent.folder_place
            ),
            ("main", Source::List, now - hour, place)
        );
        assert_eq!(shown_parent(&s, "child", later).as_deref(), Some("parent"));
        assert!(!s.restore_found_parent(Provider::Codex, "child", None, later));

        // One the hook has reported since is not brought back from the lookup: its own events say what it is.
        s.apply(
            Provider::Claude,
            "main",
            Place::default,
            &event("parent", EventKind::Prompt),
            later,
        );
        s.apply(
            Provider::Claude,
            "main",
            Place::default,
            &event("parent", EventKind::End),
            later,
        );
        let gone = later + DONE_KEPT_MS + 1;
        s.apply(
            Provider::Codex,
            "a2",
            Place::default,
            &event("child", EventKind::Prompt),
            gone,
        );
        assert!(s.get(Provider::Claude, "parent").is_none());
        assert!(!s.restore_found_parent(Provider::Codex, "child", None, gone));
    }

    #[test]
    fn found_main_sessions_are_remembered_only_up_to_a_bound() {
        let mut s = Sessions::default();
        for n in 0..=MAX_LOOKED_UP {
            s.add_found(
                Provider::Claude,
                "main",
                None,
                (format!("p{n}"), 0, Place::default()),
                0,
            );
        }
        assert_eq!(s.found.len(), MAX_LOOKED_UP);
        assert!(s.found.iter().all(|f| f.id != "p0"));
    }

    #[test]
    fn a_run_reported_later_finds_its_idle_main_session() {
        let hour = 60 * 60_000;
        let now = 10 * hour;
        let mut s = Sessions::default();
        // A run reported by the hook after the start, naming main's `parent` in its event.
        s.apply(
            Provider::Claude,
            "a2",
            Place::default,
            &event("child", EventKind::Start),
            now,
        );
        // Not known, so it cannot be its parent yet.
        assert!(!s.set_parent(Provider::Claude, "child", Some("parent"), now));
        assert_eq!(
            s.parent_to_find(Provider::Claude, "child", Some("parent"), now)
                .as_deref(),
            Some("parent")
        );
        assert_eq!(
            s.parent_to_find(Provider::Claude, "child", Some("parent"), now),
            None
        );
        s.add_listed(
            Provider::Claude,
            "main",
            vec![("parent".into(), now - hour, Place::default())],
            now,
        );
        // Shown again once the run sits under it.
        assert!(s.set_parent(Provider::Claude, "child", Some("parent"), now));
        assert_eq!(shown_parent(&s, "child", now).as_deref(), Some("parent"));
        assert_eq!(
            s.parent_links().get(Provider::Claude, "child", now),
            Some("parent")
        );
        // Nothing to look up for a main session, a known parent, itself or something that is not a session id.
        s.apply(
            Provider::Claude,
            "main",
            Place::default,
            &event("m2", EventKind::Start),
            now,
        );
        assert_eq!(
            s.parent_to_find(Provider::Claude, "m2", Some("other"), now),
            None
        );
        assert_eq!(
            s.parent_to_find(Provider::Claude, "child", Some("child"), now),
            None
        );
        assert_eq!(
            s.parent_to_find(Provider::Claude, "child", Some("m2"), now),
            None
        );
        assert_eq!(
            s.parent_to_find(Provider::Claude, "child", Some("bad id"), now),
            None
        );
        // A run's saved link counts when its event names none.
        let mut t = Sessions::default();
        t.restore_parents(saved_link(now - hour), now);
        t.apply(
            Provider::Codex,
            "a2",
            Place::default,
            &event("child", EventKind::Prompt),
            now,
        );
        assert_eq!(
            t.parent_to_find(Provider::Codex, "child", None, now)
                .as_deref(),
            Some("parent")
        );
    }

    #[test]
    fn parent_links_keep_only_the_newest_few() {
        let mut links = ParentLinks::default();
        for i in 0..MAX_PARENT_LINKS + 5 {
            links.remember(Provider::Codex, &format!("run-{i}"), "parent", 10);
        }
        assert_eq!(links.links.len(), MAX_PARENT_LINKS);
        assert_eq!(
            links.get(Provider::Codex, "run-0", 10),
            None,
            "the oldest went first"
        );
        assert_eq!(
            links.get(
                Provider::Codex,
                &format!("run-{}", MAX_PARENT_LINKS + 4),
                10
            ),
            Some("parent")
        );
        // Learning a run's parent again replaces its link rather than adding one.
        links.remember(Provider::Codex, "run-200", "other", 11);
        assert_eq!(links.links.len(), MAX_PARENT_LINKS);
        assert_eq!(links.get(Provider::Codex, "run-200", 11), Some("other"));
    }

    #[test]
    fn both_providers_link_only_to_main_and_keep_done_parents_until_they_leave() {
        for provider in [Provider::Claude, Provider::Codex] {
            let mut sessions = Sessions::default();
            sessions.apply(
                Provider::Claude,
                "main",
                Place::default,
                &event("parent", EventKind::Start),
                0,
            );
            sessions.apply(
                provider,
                "a1",
                Place::default,
                &event("child", EventKind::Start),
                0,
            );
            sessions.apply(
                provider,
                "a2",
                Place::default,
                &event("other", EventKind::Start),
                0,
            );
            for bad in ["unknown", "child", "other", "bad id"] {
                assert!(!sessions.set_parent(provider, "child", Some(bad), 1));
            }
            assert!(sessions.set_parent(provider, "child", Some("parent"), 1));
            assert!(!sessions.set_parent(Provider::Claude, "parent", Some("child"), 1));
            assert_eq!(
                sessions
                    .shown_with_parents(1)
                    .iter()
                    .find(|s| s.id == "child")
                    .expect("child")
                    .parent_id
                    .as_deref(),
                Some("parent")
            );
            sessions.apply(
                Provider::Claude,
                "main",
                Place::default,
                &event("parent", EventKind::End),
                2,
            );
            assert_eq!(
                sessions
                    .shown_with_parents(3)
                    .iter()
                    .find(|s| s.id == "child")
                    .expect("child")
                    .parent_id
                    .as_deref(),
                Some("parent")
            );
            assert_eq!(
                sessions
                    .shown_with_parents(2 + DONE_KEPT_MS)
                    .iter()
                    .find(|s| s.id == "child")
                    .expect("child")
                    .parent_id,
                None
            );
        }
    }
}

/// The project and name of a session working in `cwd`: the repository's name (the main repository's, for a
/// worktree), and the branch in words or the folder's name. Reads only `.git` and `HEAD`, a few bytes each.
pub fn place_of(cwd: &Path) -> Place {
    let folder_name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let mut repo: Option<(PathBuf, PathBuf)> = None; // (where .git is, the git folder)
    // Only folders on this computer's drives are looked into: a network path (from an event, or a `.git` file's
    // `gitdir`) would make Windows connect to that server, and sign in to it as the user.
    let ancestors = cwd.ancestors().take(if on_a_drive(cwd) { 32 } else { 0 });
    for dir in ancestors {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            repo = Some((dir.to_owned(), dot_git));
            break;
        }
        if dot_git.is_file() {
            // A worktree: ".git" names its git folder, `<main>/.git/worktrees/<name>`.
            let Some(text) = read_small(&dot_git) else {
                break;
            };
            let Some(git_dir) = text.trim().strip_prefix("gitdir:").map(str::trim) else {
                break;
            };
            let git_dir = dir.join(git_dir);
            if on_a_drive(&git_dir) {
                repo = Some((dir.to_owned(), git_dir));
            }
            break;
        }
    }
    let Some((root, git_dir)) = repo else {
        let name = folder_name(cwd);
        return Place {
            project: name.clone(),
            name,
        };
    };
    // For a worktree, the project is the main repository: `<main>/.git/worktrees/<name>` → `<main>`.
    let main_root = git_dir
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "worktrees"))
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map_or_else(|| root.clone(), Path::to_path_buf);
    let branch = read_small(&git_dir.join("HEAD")).and_then(|h| {
        h.trim()
            .strip_prefix("ref: refs/heads/")
            .map(|b| b.trim().to_owned())
    });
    let raw = branch.or_else(|| folder_name(&root));
    Place {
        project: folder_name(&main_root),
        name: raw.as_deref().map(words),
    }
}

/// Whether a path is a full path on one of this computer's drives (`C:\<folder>`, `\\?\C:\<folder>`): not a network
/// path (`\\<server>\<share>`, `\\?\UNC\<server>\<share>`), a device (`\\.\<name>`) or a relative path. Elsewhere
/// than Windows, any full path.
pub fn on_a_drive(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    match parts.next() {
        Some(Component::Prefix(p)) => {
            matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
                && parts.next() == Some(Component::RootDir)
        }
        Some(Component::RootDir) => !cfg!(windows),
        _ => false,
    }
}

fn read_small(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(1024)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// A branch or folder name in words: "feature/performance-admin" → "Performance admin".
fn words(raw: &str) -> String {
    let last = raw.rsplit('/').next().unwrap_or(raw);
    let spaced: Vec<&str> = last
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    let joined = spaced.join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => raw.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;

    const T0: i64 = 1_794_053_400_000; // a multiple of BIN_MS

    fn ev(json: &str) -> Event {
        parse_event(json.as_bytes()).expect("a hook event")
    }

    fn apply(s: &mut Sessions, json: &str, now: i64) -> bool {
        let e = ev(json);
        s.apply(Provider::Claude, "a1", Place::default, &e, now)
    }

    #[test]
    fn parses_claude_and_codex_events_without_keeping_prompts_or_tool_inputs() {
        let e = ev(
            r#"{"session_id":"s1","transcript_path":"t.jsonl","cwd":"%USERPROFILE%","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"npm ci --secret"}}"#,
        );
        assert_eq!(e.session_id, "s1");
        assert_eq!(e.kind, EventKind::ToolStart(Some("Bash".into())));
        assert!(!format!("{e:?}").contains("npm ci"));
        let e = ev(
            r#"{"session_id":"s1","hook_event_name":"UserPromptSubmit","prompt":"my private prompt"}"#,
        );
        assert_eq!(e.kind, EventKind::Prompt);
        assert!(!format!("{e:?}").contains("private"));
        let e = ev(
            r#"{"session_id":"s1","hook_event_name":"SubagentStart","agent_id":"x","agent_type":"Explore"}"#,
        );
        assert_eq!(
            e.kind,
            EventKind::SubagentStart {
                id: Some("x".into()),
                kind: "Explore".into()
            }
        );
        // Codex's events carry the same fields.
        let e = ev(
            r#"{"session_id":"c1","cwd":"/w","hook_event_name":"PreToolUse","model":"gpt","tool_name":"shell"}"#,
        );
        assert_eq!(e.kind, EventKind::ToolStart(Some("shell".into())));
        assert!(parse_event(&b"{}"[..]).is_err());
        assert!(parse_event(&b"not json"[..]).is_err());
    }

    #[test]
    fn sessions_end_when_their_process_exits() {
        let mut s = Sessions::default();
        let start =
            |id: &str| format!(r#"{{"session_id":"{id}","hook_event_name":"SessionStart"}}"#);
        let prompt =
            |id: &str| format!(r#"{{"session_id":"{id}","hook_event_name":"UserPromptSubmit"}}"#);
        // A Codex session (Codex never says it ended) and a Claude one, each in its own process.
        let codex = ev(&start("x1"));
        s.apply(Provider::Codex, "a2", Place::default, &codex, T0);
        apply(&mut s, &start("c1"), T0);
        apply(&mut s, &prompt("c1"), T0 + 1);
        assert!(s.needs_process(Provider::Codex, "x1"));
        s.set_process(Provider::Codex, "x1", Some(10));
        s.set_process(Provider::Claude, "c1", Some(20));
        assert!(!s.needs_process(Provider::Claude, "c1"), "looked for once");
        // Watched sessions are never dropped for being quiet.
        assert_eq!(s.shown(T0 + QUIET_GONE_MS + 1).count(), 2);
        // Another process exiting changes nothing.
        assert!(!s.process_exited(99, T0 + 5));
        // Codex's process exits: done, shown for a while.
        assert!(s.process_exited(10, T0 + 10));
        let x = s.get(Provider::Codex, "x1").expect("kept");
        assert_eq!((x.state, x.ended_at), (SessionState::Done, Some(T0 + 10)));
        // Claude's exits without having said so (crashed or killed): it goes at once, and stops counting as working.
        assert!(s.process_exited(20, T0 + 20));
        assert!(s.get(Provider::Claude, "c1").is_none());
        let last = s.activity()["a1"].bins().last().copied().expect("bin");
        assert_eq!(last.last, 0);

        // A Claude session that ended properly stays done when its process then exits.
        apply(&mut s, &start("c2"), T0 + 30);
        s.set_process(Provider::Claude, "c2", Some(21));
        apply(
            &mut s,
            r#"{"session_id":"c2","hook_event_name":"SessionEnd"}"#,
            T0 + 31,
        );
        assert!(!s.process_exited(21, T0 + 32));
        assert_eq!(
            s.get(Provider::Claude, "c2").expect("kept").state,
            SessionState::Done
        );
        // Resumed later in a new process: looked for again.
        apply(&mut s, &start("c2"), T0 + 40);
        assert!(s.needs_process(Provider::Claude, "c2"));
        // Resumed elsewhere while its old process is still winding down: the old one's exit no longer ends it.
        s.set_process(Provider::Claude, "c2", Some(30));
        apply(&mut s, &start("c2"), T0 + 41);
        s.set_process(Provider::Claude, "c2", Some(31));
        assert!(!s.process_exited(30, T0 + 42));
        assert_eq!(
            s.get(Provider::Claude, "c2").expect("live").state,
            SessionState::Idle
        );
        // A session whose process was not found keeps the old 24-hour rule.
        s.set_process(Provider::Claude, "c2", None);
        assert_eq!(
            s.shown(T0 + 41 + QUIET_GONE_MS)
                .filter(|x| x.id == "c2")
                .count(),
            0
        );
    }

    #[test]
    fn a_session_moves_through_its_states() {
        let mut s = Sessions::default();
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SessionStart"}"#,
            T0,
        );
        let get = |s: &Sessions| s.get(Provider::Claude, "s").cloned().expect("known");
        assert_eq!(get(&s).state, SessionState::Idle);
        assert_eq!(get(&s).doing, IDLE_TEXT);
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"UserPromptSubmit","prompt":"hi"}"#,
            T0 + 1,
        );
        assert_eq!(get(&s).state, SessionState::Working);
        assert_eq!(get(&s).doing, "Thinking");
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"Edit"}"#,
            T0 + 2,
        );
        assert_eq!(get(&s).doing, "Editing files");
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"PermissionRequest","tool_name":"Bash"}"#,
            T0 + 3,
        );
        assert_eq!(get(&s).state, SessionState::Waiting);
        assert_eq!(get(&s).doing, "Asks to run a command");
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"PostToolUse","tool_name":"Bash"}"#,
            T0 + 4,
        );
        assert_eq!(get(&s).state, SessionState::Working);
        assert_eq!(get(&s).doing, "Thinking", "the tool is done");
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SubagentStart","agent_id":"1","agent_type":"Explore"}"#,
            T0 + 5,
        );
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SubagentStart","agent_id":"2","agent_type":"general-purpose"}"#,
            T0 + 6,
        );
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SubagentStop","agent_id":"1","agent_type":"Explore"}"#,
            T0 + 7,
        );
        assert_eq!(get(&s).subagents.len(), 1);
        assert_eq!(get(&s).subagents[0].kind, "general-purpose");
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"Stop"}"#,
            T0 + 8,
        );
        assert_eq!(get(&s).state, SessionState::Idle);
        assert!(get(&s).subagents.is_empty());
        apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SessionEnd","reason":"exit"}"#,
            T0 + 9,
        );
        assert_eq!(get(&s).state, SessionState::Done);
        assert_eq!(s.shown(T0 + 10).count(), 1);
        assert_eq!(s.shown(T0 + 9 + DONE_KEPT_MS).count(), 0);
    }

    #[test]
    fn late_tool_events_from_before_a_stop_do_not_bring_the_session_back() {
        let mut s = Sessions::default();
        let get = |s: &Sessions| s.get(Provider::Claude, "s").cloned().expect("known");
        let ev = |name: &str| {
            format!(r#"{{"session_id":"s","hook_event_name":"{name}","tool_name":"Bash"}}"#)
        };
        apply(&mut s, &ev("UserPromptSubmit"), T0);
        apply(&mut s, &ev("Stop"), T0 + 10);
        // Sent before the stop, arriving after it.
        apply(&mut s, &ev("PostToolUse"), T0 + 20);
        apply(&mut s, &ev("PreToolUse"), T0 + 30);
        assert_eq!(get(&s).state, SessionState::Idle);
        // A tool start sent before the permission request it led to.
        apply(&mut s, &ev("UserPromptSubmit"), T0 + 5_000);
        apply(&mut s, &ev("PermissionRequest"), T0 + 5_010);
        apply(&mut s, &ev("PreToolUse"), T0 + 5_020);
        assert_eq!(get(&s).state, SessionState::Waiting);
        // Once the tool ran, it works again; and well after a stop, tool events count.
        apply(&mut s, &ev("PostToolUse"), T0 + 6_000);
        assert_eq!(get(&s).state, SessionState::Working);
        apply(&mut s, &ev("Stop"), T0 + 7_000);
        apply(&mut s, &ev("PreToolUse"), T0 + 7_000 + LATE_MS + 1);
        assert_eq!(get(&s).state, SessionState::Working);
        // After the session ended, nothing late brings it back.
        apply(&mut s, &ev("SessionEnd"), T0 + 20_000);
        apply(&mut s, &ev("PreToolUse"), T0 + 20_100);
        assert_eq!(get(&s).state, SessionState::Done);
    }

    #[test]
    fn a_new_session_is_a_change_and_a_resumed_one_counts_for_who_is_signed_in() {
        let mut s = Sessions::default();
        let start = ev(r#"{"session_id":"r","hook_event_name":"SessionStart"}"#);
        assert!(s.apply(Provider::Claude, "a1", Place::default, &start, T0));
        assert!(!s.apply(Provider::Claude, "a1", Place::default, &start, T0 + 1));
        assert!(s.apply(Provider::Claude, "a2", Place::default, &start, T0 + 2));
        assert_eq!(
            s.get(Provider::Claude, "r").expect("known").account_id,
            "a2"
        );
    }

    #[test]
    fn a_session_signed_in_as_someone_else_is_shown_but_not_counted() {
        let mut s = Sessions::default();
        let prompt = ev(r#"{"session_id":"x","hook_event_name":"UserPromptSubmit"}"#);
        let ask = ev(r#"{"session_id":"x","hook_event_name":"PermissionRequest"}"#);
        let other = Some("someone@example.com");
        assert!(s.apply_as(Provider::Claude, "a1", other, Place::default, &prompt, T0));
        let shown = s.get(Provider::Claude, "x").expect("shown");
        assert_eq!(shown.signed_in_as.as_deref(), other);
        assert_eq!(shown.state, SessionState::Working);
        s.apply_as(Provider::Claude, "a1", other, Place::default, &ask, T0 + 1);
        assert!(
            s.activity()["a1"]
                .bins()
                .all(|b| b.working == 0 && !b.waited),
            "neither its work nor its wait counts for the account"
        );
        // Resumed once the folder is the account's own again, it counts.
        let start = ev(r#"{"session_id":"x","hook_event_name":"SessionStart"}"#);
        s.apply(Provider::Claude, "a1", Place::default, &start, T0 + 2);
        s.apply(Provider::Claude, "a1", Place::default, &prompt, T0 + 3);
        assert_eq!(
            s.get(Provider::Claude, "x").expect("shown").signed_in_as,
            None
        );
        assert_eq!(s.activity()["a1"].bins().last().expect("a bin").last, 1);
    }

    #[test]
    fn a_session_joined_midway_starts_from_its_event() {
        let mut s = Sessions::default();
        assert!(apply(
            &mut s,
            r#"{"session_id":"late","hook_event_name":"PreToolUse","tool_name":"mcp__azure_devops__wit"}"#,
            T0
        ));
        let late = s.get(Provider::Claude, "late").expect("known");
        assert_eq!(late.doing, "Using azure devops");
        // The end of a session never seen adds nothing.
        assert!(!apply(
            &mut s,
            r#"{"session_id":"gone","hook_event_name":"SessionEnd"}"#,
            T0
        ));
        assert!(s.get(Provider::Claude, "gone").is_none());
    }

    #[test]
    fn listed_sessions_fill_in_until_the_hook_reports_them() {
        let mut s = Sessions::default();
        let listed = |ids: &[&str]| {
            ids.iter()
                .map(|id| ((*id).to_owned(), T0 - 60_000, Place::default()))
                .collect::<Vec<_>>()
        };
        assert!(s.add_listed(Provider::Codex, "c1", listed(&["x", "y"]), T0));
        let x = s.get(Provider::Codex, "x").expect("listed");
        assert_eq!(x.source, Source::List);
        assert_eq!(x.doing, LISTED_TEXT);
        assert_eq!(x.started_at, T0 - 60_000);
        // Listed again: nothing new. The same id under the other provider is another session.
        assert!(!s.add_listed(Provider::Codex, "c1", listed(&["x", "y"]), T0));
        assert!(s.get(Provider::Claude, "x").is_none());
        // A listed session counts as new for the hook, which then takes it over.
        assert!(s.is_new(Provider::Codex, "x"));
        let e = ev(r#"{"session_id":"x","hook_event_name":"PreToolUse","tool_name":"shell"}"#);
        assert!(s.apply(Provider::Codex, "c1", Place::default, &e, T0));
        let x = s.get(Provider::Codex, "x").expect("known");
        assert_eq!(x.source, Source::Hook);
        assert_eq!(x.state, SessionState::Working);
        assert!(!s.is_new(Provider::Codex, "x"));
        assert_eq!(s.shown(T0).filter(|v| v.id == "x").count(), 1);
        // A hook session is never replaced by the list.
        assert!(!s.add_listed(Provider::Codex, "c1", listed(&["x"]), T0));
        assert_eq!(
            s.get(Provider::Codex, "x").expect("known").source,
            Source::Hook
        );
    }

    #[test]
    fn listed_sessions_go_once_older_than_their_window() {
        let mut s = Sessions::default();
        let written = T0 - 60_000;
        let listed = |ids: &[&str]| {
            ids.iter()
                .map(|id| ((*id).to_owned(), written, Place::default()))
                .collect::<Vec<_>>()
        };
        assert!(s.add_listed(Provider::Claude, "c2", listed(&["old", "taken"]), T0));
        // Inside the window both stay.
        assert_eq!(s.shown(written + LISTED_KEPT_MS - 1).count(), 2);
        // A hook event takes one over; from then on the hook's rules apply.
        let e = ev(r#"{"session_id":"taken","hook_event_name":"PreToolUse","tool_name":"Bash"}"#);
        assert!(s.apply(Provider::Claude, "c2", Place::default, &e, T0));
        // Once its last write is older than the window, the one still listed goes.
        let after = written + LISTED_KEPT_MS;
        let ids: Vec<_> = s.shown(after).map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["taken"]);
        // Pruning also forgets it, so the CLI's list can add it again if it is written to again.
        assert!(!s.add_listed(Provider::Claude, "c2", Vec::new(), after));
        assert!(s.get(Provider::Claude, "old").is_none());
        // The one the hook took over stays until the hook's own rules drop it.
        assert_eq!(s.shown(T0 + QUIET_GONE_MS - 1).count(), 1);
        assert_eq!(s.shown(T0 + QUIET_GONE_MS).count(), 0);
    }

    #[test]
    fn a_removed_account_takes_its_sessions_and_activity_with_it() {
        let mut s = Sessions::default();
        let work = ev(r#"{"session_id":"k","hook_event_name":"PreToolUse","tool_name":"Bash"}"#);
        s.apply(Provider::Claude, "keep", Place::default, &work, T0);
        let work = ev(r#"{"session_id":"g","hook_event_name":"PreToolUse","tool_name":"Bash"}"#);
        s.apply(Provider::Claude, "gone", Place::default, &work, T0);
        assert!(s.activity().contains_key("gone"));
        s.forget_account("gone");
        assert!(s.get(Provider::Claude, "g").is_none());
        assert!(!s.activity().contains_key("gone"));
        assert!(s.get(Provider::Claude, "k").is_some());
        assert!(s.activity().contains_key("keep"));
    }

    #[test]
    fn a_subagent_stops_by_its_id_or_else_by_its_type() {
        let mut s = Sessions::default();
        for json in [
            r#"{"session_id":"s","hook_event_name":"SubagentStart","agent_id":"1","agent_type":"Explore"}"#,
            r#"{"session_id":"s","hook_event_name":"SubagentStart","agent_id":"2","agent_type":"Explore"}"#,
            r#"{"session_id":"s","hook_event_name":"SubagentStart","agent_type":"Plan"}"#,
        ] {
            apply(&mut s, json, T0);
        }
        let ids = |s: &Sessions| {
            s.get(Provider::Claude, "s")
                .expect("known")
                .subagents
                .iter()
                .map(|a| (a.id.clone(), a.kind.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&s).len(), 3);
        // The second Explore stops: the first stays.
        assert!(apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SubagentStop","agent_id":"2","agent_type":"Explore"}"#,
            T0
        ));
        assert_eq!(
            ids(&s),
            vec![(Some("1".into()), "Explore".into()), (None, "Plan".into())]
        );
        // Without an id it goes by type; an unknown one changes nothing.
        assert!(apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SubagentStop","agent_type":"Plan"}"#,
            T0
        ));
        assert!(!apply(
            &mut s,
            r#"{"session_id":"s","hook_event_name":"SubagentStop","agent_id":"9","agent_type":"Review"}"#,
            T0
        ));
        assert_eq!(ids(&s), vec![(Some("1".into()), "Explore".into())]);
    }

    #[test]
    fn memory_stays_capped() {
        let mut s = Sessions::default();
        for i in 0..(MAX_SESSIONS + 50) {
            let json = format!(r#"{{"session_id":"s{i}","hook_event_name":"UserPromptSubmit"}}"#);
            apply(&mut s, &json, T0 + i as i64);
        }
        assert_eq!(s.shown(T0 + 1000).count(), MAX_SESSIONS);
        assert!(s.get(Provider::Claude, "s0").is_none());
        for i in 0..(BINS as i64 * 3) {
            let json = format!(r#"{{"session_id":"s{i}","hook_event_name":"Stop"}}"#);
            apply(&mut s, &json, T0 + i * BIN_MS);
        }
        assert!(s.activity()["a1"].bins().count() <= BINS + 1);
    }

    #[test]
    fn activity_keeps_the_most_working_per_bin_and_the_waits() {
        let mut a = Activity::default();
        a.record(T0, 1, false);
        a.record(T0 + 1000, 3, false);
        a.record(T0 + 2000, 2, true);
        a.record(T0 + 3 * BIN_MS, 2, false); // unchanged: carried on, no bin
        a.record(T0 + 4 * BIN_MS, 0, false);
        let bins: Vec<Bin> = a.bins().copied().collect();
        assert_eq!(
            bins,
            vec![
                Bin {
                    at: T0,
                    working: 3,
                    last: 2,
                    waited: true
                },
                Bin {
                    at: T0 + 4 * BIN_MS,
                    working: 2,
                    last: 0,
                    waited: false
                },
            ]
        );
    }

    #[test]
    fn works_out_the_project_and_name_from_the_folder() {
        let dir = Dir::new("place");
        let main = dir.path().join("shop");
        std::fs::create_dir_all(main.join(".git/worktrees/lane-b")).expect("dirs");
        std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").expect("head");
        std::fs::write(
            main.join(".git/worktrees/lane-b/HEAD"),
            "ref: refs/heads/feature/55389-admin-operations\n",
        )
        .expect("head");
        let wt = dir.path().join("shop-wt").join("lane-b");
        std::fs::create_dir_all(wt.join("src")).expect("dirs");
        std::fs::write(
            wt.join(".git"),
            format!(
                "gitdir: {}\n",
                main.join(".git/worktrees/lane-b").to_string_lossy()
            ),
        )
        .expect(".git");
        assert_eq!(
            place_of(&wt.join("src")),
            Place {
                project: Some("shop".into()),
                name: Some("55389 admin operations".into()),
            }
        );
        assert_eq!(
            place_of(&main),
            Place {
                project: Some("shop".into()),
                name: Some("Main".into()),
            }
        );
        let plain = dir.path().join("notes");
        std::fs::create_dir_all(&plain).expect("dirs");
        assert_eq!(place_of(&plain).project.as_deref(), Some("notes"));
    }

    #[cfg(windows)]
    #[test]
    fn only_folders_on_this_computers_drives_are_looked_into() {
        // Built from parts, so the source names no network path.
        let unc = |prefix: &str| format!(r"{prefix}{}\share\shop", "example.com");
        let remote = unc(r"\\");
        for (path, local) in [
            (r"C:\Users\<name>\shop".to_owned(), true),
            (r"\\?\C:\Users\<name>\shop".to_owned(), true),
            (remote.clone(), false),
            (remote.replace('\\', "/"), false),
            (unc(r"\\?\UNC\"), false),
            (r"\\.\pipe\shop".to_owned(), false),
            (r"\shop".to_owned(), false),
            ("C:shop".to_owned(), false),
            ("shop".to_owned(), false),
        ] {
            assert_eq!(on_a_drive(Path::new(&path)), local, "{path}");
        }
        // A network path is named by its last folder, without anything read from it.
        let place = place_of(Path::new(&remote));
        assert_eq!(place.project.as_deref(), Some("shop"));
        // A `.git` file pointing at a network path is not followed: the folder's own name is used.
        let dir = Dir::new("place-remote");
        let wt = dir.path().join("lane-c");
        std::fs::create_dir_all(&wt).expect("dirs");
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {remote}\\.git\\worktrees\\lane-c\n"),
        )
        .expect(".git");
        assert_eq!(
            place_of(&wt),
            Place {
                project: Some("lane-c".into()),
                name: Some("lane-c".into()),
            }
        );
    }

    #[test]
    fn doing_now_in_words() {
        assert_eq!(doing_with(Some("apply_patch")), "Editing files");
        assert_eq!(doing_with(Some("Grep")), "Reading files");
        assert_eq!(doing_with(Some("WebSearch")), "Searching the web");
        assert_eq!(doing_with(Some("Frobnicate")), "Using Frobnicate");
        assert_eq!(asks_for(Some("Write")), "Asks to edit files");
        assert_eq!(asks_for(None), "Asks for permission");
    }
}

#[cfg(test)]
mod named_and_asking_tests {
    use super::*;

    const T0: i64 = 1_794_053_400_000;

    fn apply(s: &mut Sessions, json: &str, now: i64) -> bool {
        let e = parse_event(json.as_bytes()).expect("a hook event");
        s.apply(Provider::Claude, "a1", Place::default, &e, now)
    }

    fn get(s: &Sessions) -> Session {
        s.get(Provider::Claude, "s").cloned().expect("known")
    }

    fn tool(name: &str, tool: &str) -> String {
        format!(r#"{{"session_id":"s","hook_event_name":"{name}","tool_name":"{tool}"}}"#)
    }

    #[test]
    fn the_transcript_path_is_read_from_the_event() {
        let e = parse_event(
            &br#"{"session_id":"s","hook_event_name":"Stop","transcript_path":"t.jsonl","last_assistant_message":"secret"}"#[..],
        )
        .expect("event");
        assert_eq!(e.transcript, Some(PathBuf::from("t.jsonl")));
        assert!(!format!("{e:?}").contains("secret"));
    }

    #[test]
    fn claude_waits_for_you_while_its_question_is_open() {
        let mut s = Sessions::default();
        apply(&mut s, &tool("UserPromptSubmit", ""), T0);
        apply(&mut s, &tool("PreToolUse", QUESTION_TOOL), T0 + 1);
        assert_eq!(get(&s).state, SessionState::Waiting);
        assert_eq!(get(&s).doing, QUESTION_TEXT);
        assert!(s.activity()["a1"].bins().any(|b| b.waited));
        // A subagent's tools keep running meanwhile: still waiting for you.
        apply(&mut s, &tool("PreToolUse", "Read"), T0 + 2);
        apply(&mut s, &tool("PostToolUse", "Read"), T0 + 3);
        assert_eq!(get(&s).doing, QUESTION_TEXT);
        // Another tool's permission request shows, then the question again once it is answered.
        apply(&mut s, &tool("PermissionRequest", "Bash"), T0 + 4);
        assert_eq!(get(&s).doing, "Asks to run a command");
        apply(&mut s, &tool("PostToolUse", "Bash"), T0 + 5_000);
        assert_eq!(get(&s).doing, QUESTION_TEXT);
        // Answered: working again.
        apply(&mut s, &tool("PostToolUse", QUESTION_TOOL), T0 + 9_000);
        assert_eq!(get(&s).state, SessionState::Working);
        assert_eq!(get(&s).doing, "Thinking");
        // The question's own permission request is the question.
        apply(
            &mut s,
            &tool("PermissionRequest", QUESTION_TOOL),
            T0 + 10_000,
        );
        assert_eq!(get(&s).doing, QUESTION_TEXT);
        // Interrupted: the next prompt ends it.
        apply(&mut s, &tool("UserPromptSubmit", ""), T0 + 20_000);
        assert_eq!(get(&s).state, SessionState::Working);
        apply(&mut s, &tool("PreToolUse", QUESTION_TOOL), T0 + 21_000);
        apply(&mut s, &tool("Stop", ""), T0 + 22_000);
        assert_eq!(get(&s).state, SessionState::Idle);
    }

    #[test]
    fn a_title_names_the_session_as_it_is() {
        let folder = Place {
            project: Some("shop".into()),
            name: Some("55389 admin operations".into()),
        };
        assert_eq!(folder.titled(None), folder);
        for title in [
            "Payroll export",
            "#55402: Fix login",
            "Fix login (55402)",
            "Release 2026.1",
        ] {
            let p = folder.titled(Some(title));
            assert_eq!(p.name.as_deref(), Some(title));
            assert_eq!(p.project.as_deref(), Some("shop"));
        }
    }

    #[test]
    fn names_reach_sessions_seen_now_and_later() {
        let mut s = Sessions::default();
        apply(&mut s, &tool("SessionStart", ""), T0);
        assert!(s.name(Provider::Claude, vec![("s".into(), "Fix login".into())]));
        assert_eq!(get(&s).place.name.as_deref(), Some("Fix login"));
        // The same name again changes nothing; another provider's session of that id is another.
        assert!(!s.name(Provider::Claude, vec![("s".into(), "Fix login".into())]));
        assert!(!s.name(Provider::Codex, vec![("s".into(), "Other".into())]));
        assert_eq!(get(&s).place.name.as_deref(), Some("Fix login"));
        // A Codex session named before its first event, and one only listed.
        s.name(
            Provider::Codex,
            vec![("c".into(), "Docs".into()), ("l".into(), "Listed".into())],
        );
        let e = parse_event(&br#"{"session_id":"c","hook_event_name":"SessionStart"}"#[..])
            .expect("event");
        s.apply(Provider::Codex, "c1", Place::default, &e, T0);
        assert_eq!(
            s.get(Provider::Codex, "c")
                .expect("known")
                .place
                .name
                .as_deref(),
            Some("Docs")
        );
        s.add_listed(
            Provider::Codex,
            "c1",
            vec![("l".into(), T0, Place::default())],
            T0,
        );
        assert_eq!(
            s.get(Provider::Codex, "l")
                .expect("listed")
                .place
                .name
                .as_deref(),
            Some("Listed")
        );
        // Names kept stay capped.
        let many = (0..MAX_NAMES + 10)
            .map(|i| (format!("n{i}"), "x".to_owned()))
            .collect();
        s.name(Provider::Codex, many);
        assert_eq!(s.names.len(), MAX_NAMES);
        assert_eq!(s.named_order.len(), MAX_NAMES);
    }
}
