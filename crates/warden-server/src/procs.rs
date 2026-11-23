//! The sessions' processes: which CLI process a hook ran in, and Windows telling Warden when it exits.
//!
//! A hook's `warden.exe` connects to the pipe; Windows tells the core its process id. Its parent chain leads to the
//! session's CLI (`claude.exe`, or `node.exe` for an npm install; `codex.exe`, with `cmd.exe` between for Codex's
//! hooks). Warden then holds a handle on that process and a small thread waits on it (no polling): when it exits,
//! the session ends. Nothing is started and nothing is read beyond the process list.
//!
//! Memory samples ([`MemorySource`]) read the same list with each process's private memory, and keep the memory of
//! the sessions' processes and Warden's own: never another process's command line, environment, executable path or
//! working folder.

use std::collections::{HashMap, HashSet};
use std::io;
use std::time::Duration;
use sysinfo::{MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use warden_core::memory;
use warden_core::model::Provider;

#[derive(Clone, Copy)]
struct Ancestor {
    parent: Option<u32>,
    started: u64,
}

fn parent_in_chain(
    cli: u32,
    main: &[(u32, String)],
    processes: &HashMap<u32, Ancestor>,
) -> Option<String> {
    let mut seen = HashSet::from([cli]);
    let mut at = cli;
    let mut parents = HashSet::new();
    let mut complete = false;
    for _ in 0..64 {
        let Some(child) = processes.get(&at) else {
            complete = true;
            break;
        };
        let Some(parent) = child.parent else {
            complete = true;
            break;
        };
        if !seen.insert(parent) {
            return None;
        }
        let Some(ancestor) = processes.get(&parent) else {
            complete = true;
            break;
        };
        if ancestor.started > child.started {
            return None;
        }
        if parents.is_empty() {
            for (_, id) in main.iter().filter(|(pid, _)| *pid == parent) {
                parents.insert(id.clone());
            }
        }
        at = parent;
    }
    if complete && parents.len() == 1 {
        parents.into_iter().next()
    } else {
        None
    }
}

/// Uses the existing process-list source only, walking above the delegated CLI to a live main session.
pub fn parent_session(cli: u32, main: &[(u32, String)]) -> Option<String> {
    if main.is_empty() {
        return None;
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let processes = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            (
                pid.as_u32(),
                Ancestor {
                    parent: p.parent().map(|p| p.as_u32()),
                    started: p.start_time(),
                },
            )
        })
        .collect();
    parent_in_chain(cli, main, &processes)
}

/// Where memory samples come from: this computer ([`SystemMemory`]), or the demo's made-up processes.
pub trait MemorySource: Send + Sync {
    /// The computer's total and available memory, in bytes.
    fn computer(&self) -> Option<(u64, u64)>;
    /// One snapshot of the processes: each one's id, parent and start time, then the private memory of those
    /// `pick` chooses from that list. `roots` are the sessions' CLIs and `own` Warden's process (a made-up source
    /// makes its processes up from them; this computer's has them all).
    fn processes(
        &self,
        roots: &[u32],
        own: u32,
        pick: &mut dyn FnMut(&[memory::Proc]) -> Vec<u32>,
    ) -> Vec<memory::Proc>;
    /// How long between two samples.
    fn every(&self) -> Duration {
        Duration::from_millis(memory::SAMPLE_EVERY_MS)
    }
    /// A made-up CLI process for a session, in place of the one found above its hook (the demo, whose made-up
    /// sessions all send their events from one process). This computer's source has none.
    fn stand_in_cli(&self, _provider: Provider, _session_id: &str) -> Option<u32> {
        None
    }
}

/// This computer's memory, through `sysinfo`: for other processes only their parents, start times and private
/// memory (Windows' private bytes, what `sysinfo` calls virtual memory there), never anything else. A fresh
/// `System` each time, so no handle on another process is held between samples.
pub struct SystemMemory {
    every: Duration,
}

impl SystemMemory {
    /// Sampled every [`memory::SAMPLE_EVERY_MS`].
    pub fn new() -> SystemMemory {
        SystemMemory::every(Duration::from_millis(memory::SAMPLE_EVERY_MS))
    }

    /// Sampled at another interval (the tests, the budgets).
    pub fn every(every: Duration) -> SystemMemory {
        SystemMemory { every }
    }
}

impl Default for SystemMemory {
    fn default() -> SystemMemory {
        SystemMemory::new()
    }
}

impl MemorySource for SystemMemory {
    fn computer(&self) -> Option<(u64, u64)> {
        let mut sys = System::new();
        sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        (sys.total_memory() > 0).then(|| (sys.total_memory(), sys.available_memory()))
    }

    fn processes(
        &self,
        _roots: &[u32],
        _own: u32,
        pick: &mut dyn FnMut(&[memory::Proc]) -> Vec<u32>,
    ) -> Vec<memory::Proc> {
        // One snapshot (Windows' process list), each process's memory read with its start time through the handle
        // opened for that: a second pass for the chosen ones alone would take a whole second snapshot, which costs
        // more than the memory reads it saves. Only the chosen processes' memory is kept.
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        let mut memory: HashMap<u32, u64> = HashMap::new();
        let mut list: Vec<memory::Proc> = sys
            .processes()
            .iter()
            .map(|(pid, p)| {
                memory.insert(pid.as_u32(), p.virtual_memory());
                memory::Proc {
                    pid: pid.as_u32(),
                    parent: p.parent().map(Pid::as_u32),
                    started: p.start_time(),
                    private: None,
                }
            })
            .collect();
        let wanted: HashSet<u32> = pick(&list).into_iter().collect();
        for p in &mut list {
            if wanted.contains(&p.pid) {
                p.private = memory.get(&p.pid).copied();
            }
        }
        list
    }

    fn every(&self) -> Duration {
        self.every
    }
}

/// How far up a hook's parent chain the CLI is looked for (`warden.exe`, a shell, the CLI, its launcher).
const MAX_DEPTH: usize = 6;
/// The waiting threads do nothing but wait: a small stack each.
const WAITER_STACK: usize = 64 * 1024;

/// The current user's security identifier (`S-1-5-21-...`), for the pipe's security descriptor.
pub fn current_user_sid() -> io::Result<String> {
    let pid = sysinfo::get_current_pid().map_err(io::Error::other)?;
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );
    sys.process(pid)
        .and_then(|p| p.user_id())
        .map(|uid| (**uid).to_string())
        .filter(|s| s.starts_with("S-"))
        .ok_or_else(|| io::Error::other("the current user's SID could not be read"))
}

/// The program names a provider's CLI runs as.
fn cli_names(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &["claude.exe", "node.exe"],
        Provider::Codex => &["codex.exe"],
    }
}

/// The session's CLI process: the nearest ancestor of the hook's process (`client`, while it still runs) that is the
/// provider's CLI. Each step up must have started before the process below it, so a parent id Windows has since
/// given to another program is never followed.
pub fn cli_process(client: u32, provider: Provider) -> Option<u32> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut at = sys.process(Pid::from_u32(client))?;
    for _ in 0..MAX_DEPTH {
        let parent = sys.process(at.parent()?)?;
        if parent.start_time() > at.start_time() {
            return None;
        }
        let name = parent.name().to_string_lossy().to_ascii_lowercase();
        if cli_names(provider).contains(&name.as_str()) {
            return Some(parent.pid().as_u32());
        }
        at = parent;
    }
    None
}

/// Calls `exited` once the process ends (after `grace`, so its last events, sent in the background, arrive first).
/// The handle is taken now, so the process watched is this one even if Windows reuses its id later. `Err` when the
/// process is already gone or cannot be opened.
pub fn watch(pid: u32, grace: Duration, exited: impl FnOnce() + Send + 'static) -> io::Result<()> {
    let id = i32::try_from(pid).map_err(io::Error::other)?;
    let mut handle = waitpid_any::WaitHandle::open(id)?;
    std::thread::Builder::new()
        .name(format!("watch {pid}"))
        .stack_size(WAITER_STACK)
        .spawn(move || {
            if handle.wait().is_ok() {
                std::thread::sleep(grace);
                exited();
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegated_process_chains_find_only_unambiguous_live_main_sessions() {
        let mut processes = HashMap::from([
            (
                10,
                Ancestor {
                    parent: None,
                    started: 1,
                },
            ),
            (
                20,
                Ancestor {
                    parent: Some(10),
                    started: 2,
                },
            ),
            (
                30,
                Ancestor {
                    parent: Some(20),
                    started: 3,
                },
            ),
            (
                40,
                Ancestor {
                    parent: Some(30),
                    started: 4,
                },
            ),
        ]);
        assert_eq!(
            parent_in_chain(40, &[(10, "main-session".into())], &processes).as_deref(),
            Some("main-session")
        );
        assert_eq!(parent_in_chain(40, &[], &processes), None);
        assert_eq!(
            parent_in_chain(40, &[(10, "one".into()), (20, "two".into())], &processes).as_deref(),
            Some("two")
        );
        assert_eq!(
            parent_in_chain(40, &[(10, "one".into()), (10, "two".into())], &processes),
            None
        );
        processes.get_mut(&20).expect("parent").started = 5;
        assert_eq!(parent_in_chain(40, &[(10, "one".into())], &processes), None);
        processes.get_mut(&20).expect("parent").started = 2;
        processes.get_mut(&10).expect("parent").parent = Some(40);
        assert_eq!(parent_in_chain(40, &[(10, "one".into())], &processes), None);
    }

    #[test]
    fn reads_the_users_sid() {
        let sid = current_user_sid().expect("the SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
    }

    #[test]
    fn a_process_with_no_cli_above_it_has_none() {
        // This test runs under cargo, not under Claude or Codex... unless an AI runs the tests, so only the
        // answer's shape is checked here; the end-to-end test plays real chains.
        let me = std::process::id();
        if let Some(cli) = cli_process(me, Provider::Codex) {
            assert_ne!(cli, me);
        }
        assert_eq!(cli_process(u32::MAX - 3, Provider::Claude), None);
    }

    #[test]
    fn watching_takes_a_running_process() {
        let (tx, rx) = std::sync::mpsc::channel();
        // Watching a process that does not exist fails at once.
        assert!(watch(u32::MAX - 3, Duration::ZERO, || {}).is_err());
        // This process outlives the test: nothing is reported.
        watch(std::process::id(), Duration::ZERO, move || {
            let _ = tx.send(());
        })
        .expect("watch");
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    }
}
