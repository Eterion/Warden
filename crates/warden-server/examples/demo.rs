//! The demo server: the real local server on a throwaway data folder and home folder, with a stand-in for
//! Anthropic's usage endpoint that answers from files. `npm run demo` (checks/demo.ts) builds and starts it, makes the
//! made-up accounts and feeds it activity. An example, so it is never part of a normal start or a release.
//!
//! ```text
//! demo --port <port> --data-dir <folder> --home <folder> --usage-dir <folder> --ui-dir <folder> [--route-exe <file>]
//! ```
//!
//! The stand-in endpoint answers a token's request with `<usage-dir>/<token>.json` (as Anthropic's body, its limit
//! resets block only when the request asks for it), or 401 when there is no such file; the stand-in for ChatGPT's
//! backend answers a Codex account's banked resets with `<usage-dir>/<token>.resets.json` (as the backend's body), or
//! none. Using a limit reset in the app answers success after about a second (Claude's claim and Codex's redeem alike),
//! and the stand-ins remember it until the demo stops: a used reset no longer comes back in their answers, and starting
//! the demo again brings it back. Nothing leaves this computer, and no program is started: the CLIs on the demo's PATH
//! (the home's `bin`) are stand-ins the driver writes, and the sign-in only prints.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use warden_core::claude::UsageEndpoint;
use warden_core::detect::Env;
use warden_core::secret::Secret;
use warden_core::time::now_ms;
use warden_server::{Config, start};

/// How long the stand-ins take to answer a use of a reset, so the app's busy state shows.
const USE_TAKES: Duration = Duration::from_millis(1000);

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
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut port = None;
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
