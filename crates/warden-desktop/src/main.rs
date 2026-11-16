//! Warden's desktop app: the same core, API and UI as the browser version, in a Tauri window, with no network port.
//!
//! ```text
//! warden-desktop [--data-dir <folder>] [--ui-dir <folder>] [--autostart]
//! ```
//!
//! The core (the server) runs for as long as the app does. The window exists only while open: closing it destroys
//! the WebView and frees its memory. With Keep running in the tray off, closing the window quits; on, Warden stays
//! in the tray. The page and its API requests are answered in-process through the app's own URL scheme; the window's
//! own controls, the folder picker and Quit are the only things it asks of Tauri.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod autostart;
mod shell;
mod updates;
mod window_state;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, PartialEq)]
pub struct Args {
    pub data_dir: Option<PathBuf>,
    pub ui_dir: Option<PathBuf>,
    /// Started by Windows at sign-in: with Keep running in the tray on, start in the tray without a window.
    pub autostart: bool,
    #[cfg(debug_assertions)]
    pub no_hooks: bool,
    #[cfg(debug_assertions)]
    pub home: Option<PathBuf>,
}

impl Args {
    /// The options Start with Windows repeats, so a Warden with its own data folder starts as itself.
    pub fn options(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (flag, dir) in [("--data-dir", &self.data_dir), ("--ui-dir", &self.ui_dir)] {
            if let Some(d) = dir {
                let d = std::path::absolute(d).unwrap_or_else(|_| d.clone());
                out.extend([flag.to_owned(), d.to_string_lossy().into_owned()]);
            }
        }
        #[cfg(debug_assertions)]
        {
            if self.no_hooks {
                out.push("--no-hooks".into());
            }
            if let Some(home) = &self.home {
                let home = std::path::absolute(home).unwrap_or_else(|_| home.clone());
                out.extend(["--home".into(), home.to_string_lossy().into_owned()]);
            }
        }
        out
    }
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
        data_dir: None,
        ui_dir: None,
        autostart: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            #[cfg(debug_assertions)]
            "--no-hooks" => out.no_hooks = true,
            #[cfg(debug_assertions)]
            "--home" => out.home = Some(PathBuf::from(value()?)),
            "--data-dir" => out.data_dir = Some(PathBuf::from(value()?)),
            "--ui-dir" => out.ui_dir = Some(PathBuf::from(value()?)),
            autostart::AUTOSTART_ARG => out.autostart = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(out)
}

/// The built UI: `ui` beside the installed app, or the repo's `ui/dist` for a build in `target/<profile>`.
fn find_ui_dir(exe_dir: &Path) -> Option<PathBuf> {
    [exe_dir.join("ui"), exe_dir.join("../../ui/dist")]
        .into_iter()
        .find(|d| d.join("index.html").is_file())
}

/// `warden.exe`, which the routing hook runs: `bin` beside the installed app (the installer keeps it apart from the
/// app's own program), or beside the app in `target/<profile>`.
fn find_route_exe(exe_dir: &Path) -> Option<PathBuf> {
    let name = format!("warden{}", std::env::consts::EXE_SUFFIX);
    [exe_dir.join("bin").join(&name), exe_dir.join(&name)]
        .into_iter()
        .find(|f| f.is_file())
}

/// Whether this is the installed app: the installer puts `warden.exe` in `bin` beside it. Only it updates itself, so
/// a build run from `target` (the checks, a throwaway Warden beside your own) never looks for updates or replaces the
/// installed app.
fn installed(exe_dir: &Path) -> bool {
    exe_dir
        .join("bin")
        .join(format!("warden{}", std::env::consts::EXE_SUFFIX))
        .is_file()
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "warden-desktop: {e}\nusage: warden-desktop [--data-dir <folder>] [--ui-dir <folder>] [--autostart]"
            );
            return ExitCode::from(2);
        }
    };
    let Some(data_dir) = args
        .data_dir
        .clone()
        .or_else(warden_core::store::default_data_dir)
    else {
        eprintln!("warden-desktop: no data folder; pass --data-dir");
        return ExitCode::from(2);
    };
    let ui_dir = args.ui_dir.clone().or_else(|| {
        let exe = std::env::current_exe().ok()?;
        find_ui_dir(exe.parent()?)
    });
    let route_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| find_route_exe(exe.parent()?));
    let self_update = updates::enabled()
        && std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(installed))
            .unwrap_or(false);
    shell::run(args, data_dir, ui_dir, route_exe, self_update)
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
        let again = parse(&args.options()).expect("debug options repeat");
        assert!(again.no_hooks);
        assert_eq!(
            again.home,
            Some(std::path::absolute("test-home").expect("absolute"))
        );
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
                data_dir: None,
                ui_dir: None,
                autostart: false,
            })
        );
        assert_eq!(
            a(&["--data-dir", "d", "--ui-dir", "u", "--autostart"]),
            Ok(Args {
                #[cfg(debug_assertions)]
                no_hooks: false,
                #[cfg(debug_assertions)]
                home: None,
                data_dir: Some("d".into()),
                ui_dir: Some("u".into()),
                autostart: true,
            })
        );
        assert!(a(&["--what"]).is_err());
        // The desktop app has no port.
        assert_eq!(
            a(&["--port", "4800"]),
            Err("unknown argument --port".to_owned())
        );
    }

    #[test]
    fn start_with_windows_repeats_the_options_that_make_a_separate_warden() {
        let a = |v: &[&str]| {
            parse(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                .expect("parses")
                .options()
        };
        assert!(a(&[]).is_empty());
        assert!(a(&["--autostart"]).is_empty());
        let dir = std::env::temp_dir().join("other warden");
        let got = a(&["--data-dir", &dir.to_string_lossy()]);
        assert_eq!(got, ["--data-dir", &dir.to_string_lossy()]);
    }

    #[test]
    fn finds_warden_route_in_bin_first() {
        let dir = warden_core::testdir::Dir::new("route-exe");
        let name = format!("warden{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(find_route_exe(dir.path()), None);
        std::fs::write(dir.path().join(&name), "").expect("write");
        assert_eq!(find_route_exe(dir.path()), Some(dir.path().join(&name)));
        std::fs::create_dir_all(dir.path().join("bin")).expect("mkdir");
        std::fs::write(dir.path().join("bin").join(&name), "").expect("write");
        assert_eq!(
            find_route_exe(dir.path()),
            Some(dir.path().join("bin").join(&name))
        );
    }

    #[test]
    fn only_the_installed_app_updates_itself() {
        let dir = warden_core::testdir::Dir::new("installed");
        let name = format!("warden{}", std::env::consts::EXE_SUFFIX);
        assert!(!installed(dir.path()));
        // A build in `target/<profile>` has warden.exe beside it, not in `bin`.
        std::fs::write(dir.path().join(&name), "").expect("write");
        assert!(!installed(dir.path()));
        std::fs::create_dir_all(dir.path().join("bin")).expect("mkdir");
        std::fs::write(dir.path().join("bin").join(&name), "").expect("write");
        assert!(installed(dir.path()));
    }

    #[test]
    fn finds_the_ui_beside_the_app_first() {
        let dir = warden_core::testdir::Dir::new("ui-dir");
        let exe_dir = dir.path().join("target/release");
        std::fs::create_dir_all(&exe_dir).expect("mkdir");
        assert_eq!(find_ui_dir(&exe_dir), None);
        std::fs::create_dir_all(dir.path().join("ui/dist")).expect("mkdir");
        std::fs::write(dir.path().join("ui/dist/index.html"), "<!doctype html>").expect("write");
        assert_eq!(find_ui_dir(&exe_dir), Some(exe_dir.join("../../ui/dist")));
        std::fs::create_dir_all(exe_dir.join("ui")).expect("mkdir");
        std::fs::write(exe_dir.join("ui/index.html"), "<!doctype html>").expect("write");
        assert_eq!(find_ui_dir(&exe_dir), Some(exe_dir.join("ui")));
    }

    #[test]
    fn says_what_is_wrong_with_the_arguments() {
        let a = |v: &[&str]| parse(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            a(&["--data-dir"]),
            Err("--data-dir needs a value".to_owned())
        );
        assert_eq!(a(&["--ui-dir"]), Err("--ui-dir needs a value".to_owned()));
        assert_eq!(a(&["d"]), Err("unknown argument d".to_owned()));
        // A flag's value is taken as it is, even one that looks like a flag.
        assert_eq!(
            a(&["--data-dir", "--autostart"]).map(|x| (x.data_dir, x.autostart)),
            Ok((Some("--autostart".into()), false))
        );
    }

    #[test]
    fn a_repeated_option_keeps_its_last_value() {
        let a = |v: &[&str]| parse(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        let got = a(&[
            "--ui-dir",
            "u1",
            "--ui-dir",
            "u2",
            "--data-dir",
            "x",
            "--data-dir",
            "y",
        ])
        .expect("parses");
        assert_eq!(got.ui_dir, Some("u2".into()));
        assert_eq!(got.data_dir, Some("y".into()));
    }

    #[test]
    fn start_with_windows_repeats_folders_as_full_paths() {
        let args = Args {
            #[cfg(debug_assertions)]
            no_hooks: false,
            #[cfg(debug_assertions)]
            home: None,
            data_dir: Some("relative data".into()),
            ui_dir: Some("relative ui".into()),
            autostart: true,
        };
        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(
            args.options(),
            [
                "--data-dir".to_owned(),
                cwd.join("relative data").to_string_lossy().into_owned(),
                "--ui-dir".to_owned(),
                cwd.join("relative ui").to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn start_with_windows_options_parse_back_to_the_same_warden() {
        let dir = std::env::temp_dir().join("other warden");
        let args = Args {
            #[cfg(debug_assertions)]
            no_hooks: false,
            #[cfg(debug_assertions)]
            home: None,
            data_dir: Some(dir.clone()),
            ui_dir: Some(dir.join("ui")),
            autostart: false,
        };
        let mut again = args.options();
        again.push(autostart::AUTOSTART_ARG.to_owned());
        assert_eq!(
            parse(&again),
            Ok(Args {
                autostart: true,
                ..args
            })
        );
    }

    #[test]
    fn needs_index_html_to_count_as_the_ui_and_a_file_as_warden() {
        let dir = warden_core::testdir::Dir::new("ui-needs-index");
        std::fs::create_dir_all(dir.path().join("ui")).expect("mkdir");
        assert_eq!(find_ui_dir(dir.path()), None, "a folder without index.html");
        let name = format!("warden{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(dir.path().join("bin").join(&name)).expect("mkdir");
        assert_eq!(
            find_route_exe(dir.path()),
            None,
            "a folder named like the program"
        );
    }
}
