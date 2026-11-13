//! The Tauri side: the core starts with the app and answers the window in-process through the `warden` URL scheme (no
//! network port); the window opens and closes around it; the tray icon comes and goes
//! with Keep running in the tray; identity changes become Windows notifications.

use crate::Args;
use crate::alerts::Alerts;
use crate::autostart::RunValue;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::http::{Request, Response, header::CONTENT_TYPE};
use tauri::ipc::CapabilityBuilder;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use warden_core::claude::AnthropicUsageEndpoint;
use warden_core::fallback;
use warden_core::store::{self, READINGS_FILE, Readings};
use warden_core::time::now_ms;
use warden_server::api::{self, SECURITY_HEADERS};
use warden_server::app::{ApiError, App};
use warden_server::{Config, start_core};

const WINDOW: &str = "main";
const TRAY: &str = "warden";
/// The app's own URL scheme: the page and its API, answered in-process.
const SCHEME: &str = "warden";
/// The page's address: WebView2 reaches an app's own scheme as `http://<scheme>.localhost`.
const PAGE: &str = "http://warden.localhost/";

/// What the window and the tray need from the running core.
struct Core {
    app: Arc<App>,
    /// The built UI.
    ui_dir: Option<PathBuf>,
    /// What the page has loaded since it was last served, said once each in the log.
    page: PageLoad,
    /// WebView2's profile, inside Warden's data folder.
    webview_dir: PathBuf,
}

/// What the page may ask of Tauri: its window's controls, the folder picker and Quit. Only the app's own page (the
/// `warden` scheme, which Tauri counts as the app's own) in its window gets it.
fn window_capability() -> CapabilityBuilder {
    [
        "core:window:allow-minimize",
        "core:window:allow-toggle-maximize",
        "core:window:allow-internal-toggle-maximize",
        "core:window:allow-is-maximized",
        "core:window:allow-is-minimized",
        "core:window:allow-close",
        "core:window:allow-start-dragging",
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "allow-pick-folder",
        "allow-quit",
    ]
    .into_iter()
    .fold(
        CapabilityBuilder::new("main-window").window(WINDOW),
        CapabilityBuilder::permission,
    )
}

/// The system folder picker, for adding an existing account folder. `None` when cancelled.
#[tauri::command]
async fn pick_folder(window: WebviewWindow) -> Option<String> {
    window
        .dialog()
        .file()
        .set_parent(&window)
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Quit Warden: the window, the tray and the core.
#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Opens the window, or brings it back if it is already open.
pub fn open_window(handle: &AppHandle) {
    if let Some(w) = handle.get_webview_window(WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let core = handle.state::<Core>();
    let Ok(own) = Url::parse(PAGE) else {
        return;
    };
    let built = WebviewWindowBuilder::new(handle, WINDOW, WebviewUrl::CustomProtocol(own.clone()))
        .title("Warden")
        .inner_size(1280.0, 800.0)
        .min_inner_size(800.0, 560.0)
        .decorations(false)
        .data_directory(core.webview_dir.clone())
        // The window shows only Warden's own page; links elsewhere and new windows are refused.
        .on_navigation(move |url| url.origin() == own.origin())
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build();
    if let Err(e) = built {
        (core.app.log)(&format!("could not open the window: {e}"));
    }
}

/// Shows or removes the tray icon. It exists only while Keep running in the tray is on.
fn set_tray(handle: &AppHandle, on: bool) {
    if !on {
        handle.remove_tray_by_id(TRAY);
        return;
    }
    if handle.tray_by_id(TRAY).is_some() {
        return;
    }
    let built = (|| -> tauri::Result<()> {
        let open = MenuItem::with_id(handle, "open", "Open Warden", true, None::<&str>)?;
        let quit = MenuItem::with_id(handle, "quit", "Quit Warden", true, None::<&str>)?;
        let menu = Menu::with_items(handle, &[&open, &quit])?;
        let mut tray = TrayIconBuilder::with_id(TRAY)
            .tooltip("Warden")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "open" => open_window(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    open_window(tray.app_handle());
                }
            });
        if let Some(icon) = handle.default_window_icon() {
            tray = tray.icon(icon.clone());
        }
        tray.build(handle)?;
        Ok(())
    })();
    if let Err(e) = built {
        (handle.state::<Core>().app.log)(&format!("could not show the tray icon: {e}"));
    }
}

/// Follows the core's state without polling: the tray icon with Keep running in the tray, and a notification for
/// each identity change.
///
/// It starts from the core's first revision, so whatever changed while the window was being built is caught up.
fn follow(handle: AppHandle, app: Arc<App>, mut alerts: Alerts, mut tray: bool) {
    let mut revision = 0;
    while let Some(r) = app.wait_for_change(revision) {
        revision = r;
        let keep = app.keep_in_tray();
        if keep != tray {
            tray = keep;
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                set_tray(&h, keep);
                // Switched off from a browser tab while the window is closed: nothing is left to reopen Warden.
                if !keep && h.get_webview_window(WINDOW).is_none() {
                    h.exit(0);
                }
            });
        }
        for m in alerts.update(&app.identity_alerts()) {
            if let Err(e) = handle
                .notification()
                .builder()
                .title(m.title)
                .body(m.body)
                .show()
            {
                (app.log)(&format!("could not show a notification: {e}"));
            }
        }
    }
}

fn start_app(
    handle: &AppHandle,
    args: &Args,
    data_dir: PathBuf,
    ui_dir: Option<PathBuf>,
    route_exe: Option<PathBuf>,
) -> Result<(), String> {
    let desktop = RunValue::for_this_app(&args.options())
        .map_err(|e| format!("could not find Warden's own program: {e}"))?;
    let webview_dir = data_dir.join("webview");
    let saved: Readings = store::load(&data_dir, READINGS_FILE).unwrap_or_default();
    let alerts = Alerts::from_saved(&saved);
    window_csp(handle).ok_or("the app has no Content-Security-Policy")?;
    let core = start_core(Config {
        data_dir,
        ui_dir,
        env: args.environment(),
        endpoint: Arc::new(AnthropicUsageEndpoint::new()),
        sign_in_server: Arc::new(warden_core::mainlogin::AnthropicSignInServer::default()),
        sign_in: Arc::new(fallback::start_sign_in),
        log: Arc::new(|line| eprintln!("{} warden: {line}", now_ms())),
        desktop: Some(Arc::new(desktop)),
        route_exe,
    })
    .map_err(|e| format!("Warden could not start its core: {e}"))?;
    handle
        .add_capability(window_capability())
        .map_err(|e| e.to_string())?;
    let app = Arc::clone(&core.app);
    handle.manage(Core {
        app: Arc::clone(&app),
        ui_dir: core.ui_dir.clone(),
        page: PageLoad::default(),
        webview_dir,
    });

    let keep = app.keep_in_tray();
    set_tray(handle, keep);
    if !(args.autostart && keep) {
        open_window(handle);
    }
    let h = handle.clone();
    std::thread::spawn(move || follow(h, app, alerts, keep));
    Ok(())
}

/// Answers one request from the window through the `warden` scheme: the page's files and the API, as the development
/// server would, with the window's Content-Security-Policy. Only the page itself may send them (no `Origin`, or its
/// own).
fn answer_window(handle: &AppHandle, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let answer = match handle.try_state::<Core>() {
        None => api::Answer::error(ApiError(503, "Warden is still starting.".into())),
        Some(_)
            if !own_origin(
                request
                    .headers()
                    .get("Origin")
                    .map(|o| o.to_str().unwrap_or("")),
            ) =>
        {
            api::Answer::error(ApiError(
                403,
                "Requests are accepted only from Warden's own page.".into(),
            ))
        }
        Some(core) => {
            let url = request.uri().path_and_query().map_or("/", |p| p.as_str());
            let answer = api::answer(
                &core.app,
                core.ui_dir.as_deref(),
                request.method().as_str(),
                url,
                request.body(),
            );
            note_page_load(&core, api::path_of(url), &answer);
            answer
        }
    };
    // Refuse everything should the policy ever be missing (the app does not start without it).
    let csp = window_csp(handle).unwrap_or_else(|| "default-src 'none'".to_owned());
    let mut response = Response::builder()
        .status(answer.status)
        .header(CONTENT_TYPE, answer.content_type);
    for (k, v) in SECURITY_HEADERS {
        response = response.header(k, v);
    }
    response = response.header("Content-Security-Policy", csp);
    response
        .body(answer.body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// The window's Content-Security-Policy, `app.security.csp` in `tauri.conf.json`, sent with every answer, the ones
/// before the core has started included.
fn window_csp(handle: &AppHandle) -> Option<String> {
    handle
        .config()
        .app
        .security
        .csp
        .as_ref()
        .map(ToString::to_string)
}

/// Whether a request's `Origin` is the page's own: none (a GET from the page) or the page's.
fn own_origin(origin: Option<&str>) -> bool {
    origin.is_none_or(|o| o == PAGE.trim_end_matches('/'))
}

/// Says in the log, once per page load, that the page was served, its styles too, and that its scripts reached the
/// API: the desktop budgets check reads these to know the page loaded under its Content-Security-Policy.
fn note_page_load(core: &Core, path: &str, answer: &api::Answer) {
    if let Some(line) = core.page.note(path, answer.status, answer.content_type) {
        (core.app.log)(line);
    }
}

/// Since the page was last served: whether its styles were, and whether its scripts reached the API.
#[derive(Default)]
struct PageLoad {
    styled: AtomicBool,
    reached_api: AtomicBool,
}

impl PageLoad {
    /// The line an answer earns in the log, if any. Only the page's own document counts as a page load: a path the
    /// UI does not have (WebView2's own `/favicon.ico`) is answered with the page too, and is no new load.
    fn note(&self, path: &str, status: u16, content_type: &str) -> Option<&'static str> {
        if status != 200 {
            return None;
        }
        if content_type.starts_with("text/html") {
            if !matches!(path, "/" | "/index.html") {
                return None;
            }
            self.styled.store(false, Ordering::Relaxed);
            self.reached_api.store(false, Ordering::Relaxed);
            Some("the window loaded its page")
        } else if content_type.starts_with("text/css") && !self.styled.swap(true, Ordering::Relaxed)
        {
            Some("the window's page loaded its styles")
        } else if path.starts_with("/api/") && !self.reached_api.swap(true, Ordering::Relaxed) {
            Some("the window's page reached the API")
        } else {
            None
        }
    }
}

/// The key that makes one Warden per data folder. The user's own folder (no `--data-dir`) keeps the app's
/// identifier, which the installed app's notifications use too; another folder gets the identifier with a hash of
/// its full path (compared without case, as Windows does), so it runs beside the user's Warden.
pub fn instance_id(identifier: &str, data_dir: Option<&Path>) -> String {
    let Some(dir) = data_dir else {
        return identifier.to_owned();
    };
    let full = std::path::absolute(dir).unwrap_or_else(|_| dir.to_owned());
    // FNV-1a: stable across Rust versions, unlike the standard hasher.
    let hash = full
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{identifier}.{hash:016x}")
}

pub fn run(
    args: Args,
    data_dir: PathBuf,
    ui_dir: Option<PathBuf>,
    route_exe: Option<PathBuf>,
) -> ExitCode {
    // One Warden per data folder: the single-instance plugin keys on the identifier.
    let mut context = tauri::generate_context!();
    let id = instance_id(&context.config().identifier, args.data_dir.as_deref());
    context.config_mut().identifier = id;
    let built = tauri::Builder::default()
        // First, so a second start only brings the running Warden's window back, before starting anything.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            open_window(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![pick_folder, quit])
        // The page and its API, in-process: each request on a thread of its own, since the long poll waits.
        .register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
            let handle = ctx.app_handle().clone();
            std::thread::spawn(move || responder.respond(answer_window(&handle, &request)));
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            if let Err(e) = start_app(&handle, &args, data_dir, ui_dir, route_exe) {
                eprintln!("warden-desktop: {e}");
                let h = handle.clone();
                handle
                    .dialog()
                    .message(e)
                    .title("Warden")
                    .kind(MessageDialogKind::Error)
                    .show(move |_| h.exit(1));
            }
            Ok(())
        })
        .build(context);
    let app = match built {
        Ok(a) => a,
        Err(e) => {
            eprintln!("warden-desktop: could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    app.run(|handle, event| match event {
        // The last window closed (not Quit): stay in the tray when that is on, otherwise quit.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => {
            if handle
                .try_state::<Core>()
                .is_some_and(|c| c.app.keep_in_tray())
            {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => {
            if let Some(core) = handle.try_state::<Core>() {
                core.app.stop();
            }
        }
        _ => {}
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_page_load_is_said_once_each_whatever_else_the_window_asks_for() {
        let page = PageLoad::default();
        // What one load asks for, as the window sends it: the page, its script, styles and font, WebView2's own icon
        // (answered with the page, since the UI has none), and two API calls.
        let load = [
            ("/", 200, "text/html; charset=utf-8"),
            ("/assets/index.js", 200, "text/javascript"),
            ("/assets/index.css", 200, "text/css"),
            ("/assets/inter.woff2", 200, "font/woff2"),
            ("/favicon.ico", 200, "text/html; charset=utf-8"),
            ("/api/state", 200, "application/json"),
            ("/api/state/wait", 200, "application/json"),
        ];
        let said = |page: &PageLoad| -> Vec<&'static str> {
            load.iter()
                .filter_map(|(p, s, c)| page.note(p, *s, c))
                .collect()
        };
        let lines = vec![
            "the window loaded its page",
            "the window's page loaded its styles",
            "the window's page reached the API",
        ];
        assert_eq!(said(&page), lines);
        // The next opening of the window says all three again; a failed answer says nothing.
        assert_eq!(said(&page), lines);
        assert_eq!(page.note("/index.html", 404, "text/html"), None);
    }

    #[test]
    fn the_window_policy_is_strict() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let csp = config["app"]["security"]["csp"].as_str().expect("a CSP");
        // The bridge to the window (its controls, the folder picker, Quit) is the only other connection.
        warden_rules::csp::assert_strict(csp, &["ipc:", "http://ipc.localhost"]);
    }

    #[test]
    fn only_the_page_itself_may_ask() {
        assert!(own_origin(None));
        assert!(own_origin(Some("http://warden.localhost")));
        for other in [
            "null",
            "http://127.0.0.1:4777",
            "https://example.com",
            "http://warden.localhost.example.com",
            "",
        ] {
            assert!(!own_origin(Some(other)), "{other}");
        }
    }

    #[test]
    fn one_warden_per_data_folder() {
        let id = "app.warden.desktop";
        assert_eq!(instance_id(id, None), id);
        let tmp = std::env::temp_dir();
        let a = instance_id(id, Some(&tmp.join("One")));
        assert!(a.starts_with("app.warden.desktop."), "{a}");
        assert_eq!(
            a,
            instance_id(id, Some(&tmp.join("one"))),
            "case does not matter"
        );
        assert_ne!(a, instance_id(id, Some(&tmp.join("two"))));
    }

    #[test]
    fn a_relative_data_folder_shares_the_key_of_its_full_path() {
        let id = "app.warden.desktop";
        let cwd = std::env::current_dir().expect("cwd");
        let rel = instance_id(id, Some(Path::new("some data")));
        assert_eq!(rel, instance_id(id, Some(&cwd.join("some data"))));
        let hash = rel.strip_prefix("app.warden.desktop.").expect("prefix");
        assert_eq!(hash.len(), 16, "{rel}");
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{rel}"
        );
        // The same folder always gives the same key, so a second start finds the first.
        assert_eq!(rel, instance_id(id, Some(Path::new("some data"))));
        assert_ne!(rel, instance_id("other.app", Some(Path::new("some data"))));
    }
}
