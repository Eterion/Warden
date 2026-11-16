//! `warden-server`: Warden's core and local server, for the browser version, which is for development only. It prints
//! the address to open, carrying this run's token; requests without the token are refused.
//!
//! ```text
//! warden-server [--port 4777] [--data-dir <folder>] [--ui-dir <folder>]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use warden_core::claude::AnthropicUsageEndpoint;
use warden_core::fallback;
use warden_core::time::now_ms;
use warden_server::{Config, DEFAULT_PORT, start};

#[derive(Debug, PartialEq)]
struct Args {
    port: u16,
    data_dir: Option<PathBuf>,
    ui_dir: Option<PathBuf>,
    #[cfg(debug_assertions)]
    no_hooks: bool,
    #[cfg(debug_assertions)]
    home: Option<PathBuf>,
}

impl Args {
    pub fn environment(&self) -> warden_core::detect::Env {
        let env = warden_core::detect::Env::from_process();
        #[cfg(debug_assertions)]
        let env = {
            let mut env = env;
            env.no_hooks = self.no_hooks;
            if let Some(home) = &self.home {
                env.home = Some(home.clone());
                env.claude_config_dir = None;
                env.codex_home = None;
            }
            env
        };
        env
    }
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut out = Args {
        #[cfg(debug_assertions)]
        no_hooks: false,
        #[cfg(debug_assertions)]
        home: None,
        port: DEFAULT_PORT,
        data_dir: None,
        ui_dir: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--port" => {
                out.port = value()?
                    .parse()
                    .map_err(|_| "--port needs a number".to_owned())?
            }
            #[cfg(debug_assertions)]
            "--no-hooks" => out.no_hooks = true,
            #[cfg(debug_assertions)]
            "--home" => out.home = Some(PathBuf::from(value()?)),
            "--data-dir" => out.data_dir = Some(PathBuf::from(value()?)),
            "--ui-dir" => out.ui_dir = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(out)
}

/// `warden.exe`, installed beside the server.
fn route_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let route = exe.with_file_name(format!("warden{}", std::env::consts::EXE_SUFFIX));
    route.is_file().then_some(route)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "warden-server: {e}\nusage: warden-server [--port 4777] [--data-dir <folder>] [--ui-dir <folder>]"
            );
            return ExitCode::from(2);
        }
    };
    let env = args.environment();
    let Some(data_dir) = args.data_dir.or_else(warden_core::store::default_data_dir) else {
        eprintln!("warden-server: no data folder; pass --data-dir");
        return ExitCode::from(2);
    };
    let config = Config {
        data_dir,
        ui_dir: args.ui_dir,
        env,
        endpoint: Arc::new(AnthropicUsageEndpoint::new()),
        resets: Arc::new(warden_core::codex_resets::ChatGptResetsEndpoint::new()),
        sign_in_server: Arc::new(warden_core::mainlogin::AnthropicSignInServer::default()),
        sign_in: Arc::new(fallback::start_sign_in),
        log: Arc::new(|line| eprintln!("{} warden: {line}", now_ms())),
        route_exe: route_exe(),
        desktop: None,
    };
    match start(config, args.port) {
        Ok(running) => {
            eprintln!("warden-server: open {}", running.url());
            running.wait();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("warden-server: could not start: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(debug_assertions)]
    #[test]
    fn development_options_override_the_home_and_disable_hooks() {
        let args = parse(&["--no-hooks", "--home", "test-home"].map(str::to_owned))
            .expect("debug options");
        let env = args.environment();
        assert!(env.hooks_off());
        assert_eq!(env.home, Some(PathBuf::from("test-home")));
        assert_eq!(env.claude_config_dir, None);
        assert_eq!(env.codex_home, None);
        assert!(parse(&["--home".to_owned()]).is_err());
    }
    #[cfg(not(debug_assertions))]
    #[test]
    fn release_has_no_development_options() {
        assert!(parse(&["--no-hooks".to_owned()]).is_err());
        assert!(parse(&["--home".to_owned(), "test-home".to_owned()]).is_err());
    }

    #[test]
    fn parses_arguments() {
        let a = |v: &[&str]| parse(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            a(&[]),
            Ok(Args {
                #[cfg(debug_assertions)]
                no_hooks: false,
                #[cfg(debug_assertions)]
                home: None,
                port: DEFAULT_PORT,
                data_dir: None,
                ui_dir: None
            })
        );
        assert_eq!(
            a(&["--port", "0", "--data-dir", "d", "--ui-dir", "u"]),
            Ok(Args {
                #[cfg(debug_assertions)]
                no_hooks: false,
                #[cfg(debug_assertions)]
                home: None,
                port: 0,
                data_dir: Some("d".into()),
                ui_dir: Some("u".into())
            })
        );
        assert!(a(&["--port"]).is_err());
        assert!(a(&["--what"]).is_err());
    }
}
