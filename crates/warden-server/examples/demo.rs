//! The demo server: the real local server on a throwaway data folder and home folder, with a stand-in for
//! Anthropic's usage endpoint that answers from files. `npm run demo` (checks/demo.ts) builds and starts it, makes the
//! made-up accounts and feeds it activity. An example, so it is never part of a normal start or a release.
//!
//! ```text
//! demo --port <port> --data-dir <folder> --home <folder> --usage-dir <folder> --ui-dir <folder> [--route-exe <file>]
//!      [--real-memory-every-ms <ms>]
//! ```
//!
//! The stand-in endpoint answers a token's request with `<usage-dir>/<token>.json` (as Anthropic's body, its limit
//! resets block only when the request asks for it), or 401 when there is no such file; the stand-in for ChatGPT's
//! backend answers a Codex account's banked resets with `<usage-dir>/<token>.resets.json` (as the backend's body), or
//! none. Using a limit reset in the app answers success after about a second (Claude's claim and Codex's redeem alike),
//! and the stand-ins remember it until the demo stops: a used reset no longer comes back in their answers, and starting
//! the demo again brings it back. Nothing leaves this computer, and no program is started: the CLIs on the demo's PATH
//! (the home's `bin`) are stand-ins the driver writes, and the sign-in only prints.
//!
//! Memory comes from made-up processes (each hooked session a made-up CLI of its own with three processes under it,
//! one beside the demo for its window), and a few made-up past days are written to the data folder at start: no real
//! process is read. Only the budgets pass
//! `--real-memory-every-ms`, to measure what sampling this computer costs with a stand-in tree they start themselves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use warden_core::claude::UsageEndpoint;
use warden_core::detect::Env;
use warden_core::memory;
use warden_core::secret::Secret;
use warden_core::time::now_ms;
use warden_server::{Config, start};

/// How long the stand-ins take to answer a use of a reset, so the app's busy state shows.
const USE_TAKES: Duration = Duration::from_millis(1000);

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Between 0 and 1, going up and down once every `period` (in the same unit as `t`).
fn wave(t: f64, period: f64, phase: f64) -> f64 {
    ((t / period + phase) * std::f64::consts::TAU).sin() * 0.5 + 0.5
}

/// Stands in for this computer's processes, so the demo reads no real one: each session's CLI gets three made-up
/// processes under it and Warden one made-up window process, each holding memory that rises and falls over a few
/// minutes; the computer has 32 GB.
struct MadeUpMemory {
    started: std::time::Instant,
}

impl MadeUpMemory {
    fn seconds(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// A made-up process id under a real one, apart from the small ids Windows gives.
    fn child(root: u32, k: u32) -> u32 {
        0x8000_0000 | (root.wrapping_mul(8).wrapping_add(k) & 0x7FFF_FFFF)
    }

    fn private(&self, pid: u32) -> u64 {
        let base = 40 + u64::from(pid.wrapping_mul(2_654_435_761) >> 24);
        let phase = f64::from(pid % 97) / 97.0;
        let swing = wave(self.seconds(), 240.0, phase) * 200.0;
        (base + swing as u64) * MIB
    }
}

impl warden_server::procs::MemorySource for MadeUpMemory {
    /// Every made-up session gets its own made-up CLI: the demo driver sends all their events from one process.
    fn stand_in_cli(
        &self,
        provider: warden_core::model::Provider,
        session_id: &str,
    ) -> Option<u32> {
        let mut hash: u32 = 2_166_136_261;
        for b in format!("{provider:?}/{session_id}").bytes() {
            hash = (hash ^ u32::from(b)).wrapping_mul(16_777_619);
        }
        Some(0x4000_0000 | (hash & 0x3FFF_FFFF))
    }

    fn computer(&self) -> Option<(u64, u64)> {
        let used = 14 * GIB + (wave(self.seconds(), 600.0, 0.0) * 4.0 * GIB as f64) as u64;
        Some((32 * GIB, 32 * GIB - used))
    }

    fn processes(
        &self,
        roots: &[u32],
        own: u32,
        pick: &mut dyn FnMut(&[memory::Proc]) -> Vec<u32>,
    ) -> Vec<memory::Proc> {
        let proc = |pid, parent| memory::Proc {
            pid,
            parent,
            started: if parent.is_some() { 2 } else { 1 },
            private: None,
        };
        let mut list = Vec::new();
        for &root in roots {
            list.push(proc(root, None));
            for k in 0..3 {
                list.push(proc(MadeUpMemory::child(root, k), Some(root)));
            }
        }
        list.push(proc(own, None));
        list.push(proc(MadeUpMemory::child(own, 0), Some(own)));
        let wanted: std::collections::HashSet<u32> = pick(&list).into_iter().collect();
        for p in &mut list {
            if wanted.contains(&p.pid) {
                p.private = Some(self.private(p.pid));
            }
        }
        list
    }
}

/// A few made-up past days of memory (yesterday, the day before, and four days ago), so the Memory page has days to
/// show. Each a working day from 08:00 to 17:30 with Warden closed over lunch. A day already there is left as it is.
fn made_up_days(data_dir: &std::path::Path) {
    use memory::{Day, Kind, Minute, Series, SeriesInfo};
    let today = warden_core::time::parse_date(&memory::today(now_ms())).unwrap_or(0);
    let series = |n: u16,
                  kind: Kind,
                  provider,
                  account: Option<(&str, &str)>,
                  session: Option<(&str, &str)>| Series {
        series: n,
        info: SeriesInfo {
            kind,
            provider,
            account_id: account.map(|a| a.0.to_owned()),
            account: account.map(|a| a.1.to_owned()),
            session_id: session.map(|s| s.0.to_owned()),
            parent_session_id: (n == 3).then(|| "demo-past-main-1".to_owned()),
            name: session.map(|s| s.1.to_owned()),
            project: session.map(|_| "Warden".to_owned()),
        },
    };
    let claude = Some(warden_core::model::Provider::Claude);
    let codex = Some(warden_core::model::Provider::Codex);
    let all = [
        series(
            1,
            Kind::Session,
            claude,
            Some(("main", "Main")),
            Some(("demo-past-main-1", "Fix the login page")),
        ),
        series(
            2,
            Kind::Session,
            claude,
            Some(("main", "Main")),
            Some(("demo-past-main-2", "Memory monitoring")),
        ),
        series(
            3,
            Kind::Session,
            claude,
            Some(("a2", "Second")),
            Some(("demo-past-run-1", "Review the API")),
        ),
        series(
            4,
            Kind::Session,
            claude,
            Some(("a1", "Work")),
            Some(("demo-past-work-1", "Payroll export")),
        ),
        series(
            5,
            Kind::Session,
            codex,
            Some(("a3", "Codex")),
            Some(("demo-past-codex-1", "Tidy the checks")),
        ),
        series(
            6,
            Kind::Left,
            claude,
            Some(("a1", "Work")),
            Some(("demo-past-work-1", "Payroll export")),
        ),
        series(7, Kind::Warden, None, None, None),
    ];
    let dir = data_dir.join(memory::DIR);
    for back in [1, 2, 4] {
        let mut minutes = Vec::new();
        for m in (8 * 60)..(17 * 60 + 30) {
            if (12 * 60..12 * 60 + 40).contains(&m) {
                continue;
            }
            let t = f64::from(m) + f64::from(back) * 37.0;
            // Each series runs part of the day, its memory going up and down.
            let runs: [(u16, u16, u16, f64); 7] = [
                (1, 8 * 60, 11 * 60, 900.0),
                (2, 13 * 60, 17 * 60 + 30, 1400.0),
                (3, 9 * 60 + 30, 10 * 60 + 45, 700.0),
                (4, 10 * 60, 16 * 60, 600.0),
                (5, 14 * 60, 15 * 60 + 20, 500.0),
                (6, 16 * 60, 17 * 60 + 30, 350.0),
                (7, 8 * 60, 17 * 60 + 30, 12.0),
            ];
            let mut s = Vec::new();
            for (n, from, to, peak) in runs {
                if (from..to).contains(&m) {
                    let mib = peak
                        * (0.35 + 0.65 * wave(t, 47.0 + f64::from(n) * 11.0, f64::from(n) / 7.0));
                    s.push((n, (mib * 1024.0) as u32));
                }
            }
            let sum = s.iter().filter(|(n, _)| *n != 7).map(|(_, kib)| kib).sum();
            let used = 13.0 + 3.0 * wave(t, 180.0, 0.2) + f64::from(sum) / 1024.0 / 1024.0;
            minutes.push(Minute {
                m,
                total: 32 * 1024 * 1024,
                used: (used * 1024.0 * 1024.0) as u32,
                sum,
                s,
            });
        }
        let used: std::collections::HashSet<u16> = minutes
            .iter()
            .flat_map(|m| m.s.iter().map(|(n, _)| *n))
            .collect();
        let day = Day {
            series: all
                .iter()
                .filter(|s| used.contains(&s.series))
                .cloned()
                .collect(),
            minutes,
        };
        let date = warden_core::time::date_text(today - i64::from(back));
        if let Err(e) = memory::write_day(&dir, &date, &day)
            && e.kind() != std::io::ErrorKind::AlreadyExists
        {
            eprintln!("demo: could not write the made-up memory day {date}: {e}");
        }
    }
}

/// The answer file's name for a token: its plain characters only.
fn file_name(token: &Secret) -> String {
    token
        .expose()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

/// Stands in for Anthropic's usage endpoint: the answer for each token is a file the demo driver writes. Each claimed
/// limit reset is remembered per token and grant, and taken off that grant's resets left in later answers.
struct DemoUsageEndpoint {
    dir: PathBuf,
    claimed: Mutex<HashMap<(String, String), u64>>,
}

/// Stands in for ChatGPT's backend: a Codex account's banked resets are a file the demo driver writes. Each redeemed
/// credit is remembered per token and shows as redeemed in later answers.
struct DemoResetsEndpoint {
    dir: PathBuf,
    redeemed: Mutex<Vec<(String, String)>>,
}

impl warden_core::codex_resets::ResetsEndpoint for DemoResetsEndpoint {
    fn fetch(
        &self,
        token: &Secret,
        _account_id: &str,
    ) -> Result<Vec<warden_core::codex_resets::Reset>, String> {
        let name = file_name(token);
        let body = std::fs::read_to_string(self.dir.join(format!("{name}.resets.json")))
            .unwrap_or_else(|_| r#"{"credits": []}"#.to_owned());
        // Redeemed in this run: no longer available.
        let redeemed = self.redeemed.lock().expect("lock").clone();
        let body = match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(mut answer) => {
                if let Some(credits) = answer["credits"].as_array_mut() {
                    for credit in credits {
                        let id = credit["id"].as_str().unwrap_or_default().to_owned();
                        if redeemed.contains(&(name.clone(), id)) {
                            credit["status"] = "redeemed".into();
                        }
                    }
                }
                answer.to_string()
            }
            Err(_) => body,
        };
        warden_core::codex_resets::read_response(200, body.as_bytes())
    }

    fn redeem(
        &self,
        token: &Secret,
        _account_id: &str,
        credit_id: &str,
        request_id: &str,
    ) -> Result<(), warden_core::codex_resets::UseRefused> {
        warden_core::codex_resets::check_redeem_ids(credit_id, request_id)?;
        std::thread::sleep(USE_TAKES);
        self.redeemed
            .lock()
            .expect("lock")
            .push((file_name(token), credit_id.to_owned()));
        warden_core::codex_resets::read_redeem_response(
            200,
            r#"{"code": "reset", "windows_reset": 2}"#.as_bytes(),
        )
    }
}

struct DemoSignInServer;
impl warden_core::mainlogin::SignInServer for DemoSignInServer {
    fn renew(
        &self,
        token: &Secret,
    ) -> Result<warden_core::mainlogin::Renewed, warden_core::mainlogin::RenewalError> {
        Ok(warden_core::mainlogin::Renewed {
            access_token: token
                .expose()
                .trim_end_matches("-refresh")
                .to_owned()
                .into(),
            refresh_token: None,
            expires_in: 28_800,
        })
    }
}

impl UsageEndpoint for DemoUsageEndpoint {
    fn fetch(
        &self,
        token: &Secret,
        resets: bool,
    ) -> Result<warden_core::model::Usage, warden_core::model::UsageError> {
        let name = file_name(token);
        match std::fs::read_to_string(self.dir.join(format!("{name}.json"))) {
            Ok(body) => {
                // As the endpoint: the limit resets block only when the request asks for it, each grant's resets
                // left less those claimed in this run.
                let claimed = self.claimed.lock().expect("lock").clone();
                let body = match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(serde_json::Value::Object(mut answer)) if !resets => {
                        answer.remove("cedar_ember");
                        serde_json::Value::Object(answer).to_string()
                    }
                    Ok(mut answer) => {
                        if let Some(grants) = answer["cedar_ember"]["grants"].as_array_mut() {
                            for grant in grants {
                                let id = grant["id"].as_str().unwrap_or_default().to_owned();
                                let used = claimed.get(&(name.clone(), id)).copied().unwrap_or(0);
                                let left = grant["resets_left"].as_u64().unwrap_or(0);
                                grant["resets_left"] = left.saturating_sub(used).into();
                            }
                        }
                        answer.to_string()
                    }
                    Err(_) => body,
                };
                warden_core::claude::read_response(200, body.as_bytes())
            }
            Err(_) => warden_core::claude::read_response(401, std::io::empty()),
        }
    }

    fn use_reset(
        &self,
        token: &Secret,
        organization_id: &str,
        grant_id: &str,
        request_id: &str,
    ) -> Result<(), warden_core::codex_resets::UseRefused> {
        warden_core::claude::check_claim_ids(organization_id, grant_id, request_id)?;
        std::thread::sleep(USE_TAKES);
        *self
            .claimed
            .lock()
            .expect("lock")
            .entry((file_name(token), grant_id.to_owned()))
            .or_default() += 1;
        warden_core::claude::read_claim_response(
            200,
            r#"{"result": "reset", "reason": null, "resets_left": 0, "cleared": ["five_hour", "seven_day"]}"#
                .as_bytes(),
        )
    }
}

struct Args {
    port: u16,
    data_dir: PathBuf,
    home: PathBuf,
    usage_dir: PathBuf,
    ui_dir: PathBuf,
    route_exe: Option<PathBuf>,
    /// Sample this computer's memory at this interval (ms) in place of the made-up processes: the budgets only.
    real_memory_every_ms: Option<u64>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut port = None;
    let mut real_memory_every_ms = None;
    let (mut data_dir, mut home, mut usage_dir, mut ui_dir, mut route_exe) =
        (None, None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let value = it.next().cloned().ok_or(format!("{a} needs a value"))?;
        match a.as_str() {
            "--port" => port = Some(value.parse().map_err(|_| "--port needs a number")?),
            "--data-dir" => data_dir = Some(PathBuf::from(value)),
            "--home" => home = Some(PathBuf::from(value)),
            "--usage-dir" => usage_dir = Some(PathBuf::from(value)),
            "--ui-dir" => ui_dir = Some(PathBuf::from(value)),
            "--route-exe" => route_exe = Some(PathBuf::from(value)),
            "--real-memory-every-ms" => {
                real_memory_every_ms = Some(
                    value
                        .parse()
                        .map_err(|_| "--real-memory-every-ms needs a number")?,
                );
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let need = |v: Option<PathBuf>, name: &str| v.ok_or(format!("{name} is needed"));
    Ok(Args {
        port: port.ok_or("--port is needed")?,
        data_dir: need(data_dir, "--data-dir")?,
        home: need(home, "--home")?,
        usage_dir: need(usage_dir, "--usage-dir")?,
        ui_dir: need(ui_dir, "--ui-dir")?,
        route_exe,
        real_memory_every_ms,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("demo: {e}");
            return ExitCode::from(2);
        }
    };
    // Made-up processes and past days, unless the budgets ask for this computer's own memory.
    let memory: Arc<dyn warden_server::procs::MemorySource> = match args.real_memory_every_ms {
        Some(ms) => Arc::new(warden_server::procs::SystemMemory::every(
            Duration::from_millis(ms),
        )),
        None => {
            made_up_days(&args.data_dir);
            Arc::new(MadeUpMemory {
                started: std::time::Instant::now(),
            })
        }
    };
    let config = Config {
        sign_in_server: Arc::new(DemoSignInServer),
        data_dir: args.data_dir,
        ui_dir: Some(args.ui_dir),
        // Only the throwaway home: the real account folders, CLIs and desktop apps are never seen.
        env: Env {
            #[cfg(debug_assertions)]
            no_hooks: false,
            home: Some(args.home.clone()),
            local_app_data: Some(args.home.join("AppData").join("Local")),
            path: vec![args.home.join("bin")],
            claude_config_dir: None,
            codex_home: None,
            claude_git_bash: None,
            program_files: Vec::new(),
        },
        resets: Arc::new(DemoResetsEndpoint {
            dir: args.usage_dir.clone(),
            redeemed: Mutex::new(Vec::new()),
        }),
        endpoint: Arc::new(DemoUsageEndpoint {
            dir: args.usage_dir,
            claimed: Mutex::new(HashMap::new()),
        }),
        sign_in: Arc::new(|provider, _cli, folder, _done| {
            eprintln!(
                "demo: the official {provider:?} sign-in would open for {}",
                folder.display()
            );
            Ok(())
        }),
        log: Arc::new(|line| eprintln!("{} warden: {line}", now_ms())),
        route_exe: args.route_exe,
        desktop: None,
        memory,
    };
    match start(config, args.port) {
        Ok(running) => {
            // checks/demo.ts reads this line for the token.
            eprintln!("demo: open {}", running.url());
            running.wait();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("demo: could not start the server: {e}");
            ExitCode::FAILURE
        }
    }
}
