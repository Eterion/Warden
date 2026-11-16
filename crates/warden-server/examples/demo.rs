//! The demo server: the real local server on a throwaway data folder and home folder, with a stand-in for
//! Anthropic's usage endpoint that answers from files. `npm run demo` (checks/demo.ts) builds and starts it, makes the
//! made-up accounts and feeds it activity. An example, so it is never part of a normal start or a release.
//!
//! ```text
//! demo --port <port> --data-dir <folder> --home <folder> --usage-dir <folder> --ui-dir <folder> [--route-exe <file>]
//! ```
//!
//! The stand-in endpoint answers a token's request with `<usage-dir>/<token>.json` (as Anthropic's body), or 401 when
//! there is no such file; the stand-in for ChatGPT's backend answers a Codex account's banked resets with
//! `<usage-dir>/<token>.resets.json` (as the backend's body), or none. Nothing leaves this computer, and no program is started: the CLIs on the demo's PATH (the
//! home's `bin`) are stand-ins the driver writes, and the sign-in only prints.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use warden_core::claude::UsageEndpoint;
use warden_core::detect::Env;
use warden_core::secret::Secret;
use warden_core::time::now_ms;
use warden_server::{Config, start};

/// Stands in for Anthropic's usage endpoint: the answer for each token is a file the demo driver writes.
struct DemoUsageEndpoint {
    dir: PathBuf,
}

/// Stands in for ChatGPT's backend: a Codex account's banked resets are a file the demo driver writes.
struct DemoResetsEndpoint {
    dir: PathBuf,
}

impl warden_core::codex_resets::ResetsEndpoint for DemoResetsEndpoint {
    fn fetch(
        &self,
        token: &Secret,
        _account_id: &str,
    ) -> Result<Vec<warden_core::codex_resets::Reset>, String> {
        let name: String = token
            .expose()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let body = std::fs::read_to_string(self.dir.join(format!("{name}.resets.json")))
            .unwrap_or_else(|_| r#"{"credits": []}"#.to_owned());
        warden_core::codex_resets::read_response(200, body.as_bytes())
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
    ) -> Result<warden_core::model::Usage, warden_core::model::UsageError> {
        let name: String = token
            .expose()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        match std::fs::read_to_string(self.dir.join(format!("{name}.json"))) {
            Ok(body) => warden_core::claude::read_response(200, body.as_bytes()),
            Err(_) => warden_core::claude::read_response(401, std::io::empty()),
        }
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
        }),
        endpoint: Arc::new(DemoUsageEndpoint {
            dir: args.usage_dir,
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
