//! The Tauri side: the core starts with the app and answers the window in-process through the `warden` URL scheme (no
//! network port); the window opens and closes around it; the tray icon comes and goes
//! with Keep running in the tray; identity changes become Windows notifications. Release builds keep themselves up
//! to date from the repository's releases (`updates`).

use crate::Args;
use crate::alerts::Alerts;
use crate::autostart::RunValue;
use crate::updates::{self, Source, Updates};
use crate::window_state::{self, Place, Screen, Seen};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::http::{Request, Response, header::CONTENT_TYPE};
use tauri::ipc::CapabilityBuilder;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, RunEvent, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use warden_core::claude::AnthropicUsageEndpoint;
use warden_core::fallback;
use warden_core::store::{self, READINGS_FILE, Readings};
use warden_core::time::now_ms;
use warden_server::api::{self, SECURITY_HEADERS};
use warden_server::app::{ApiError, App, Desktop};
use warden_server::{Config, start_core};

const WINDOW: &str = "main";
const TRAY: &str = "warden";
/// Windows draws a window's taskbar button icon this many pixels square at 100% scale.
const TASKBAR_ICON: f64 = 24.0;
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
    /// Warden's data folder, where the window's place is kept.
    data_dir: PathBuf,
    /// The window's place as it moves and changes size, saved when it closes and when Warden quits.
    place: std::sync::Mutex<Option<Place>>,
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

/// Tauri's updater on the repository's latest release manifest (`plugins.updater` in `tauri.conf.json`): it checks
/// the update's signature against the public key there before anything runs.
struct Releases {
    handle: AppHandle,
}

/// Whether the update download may follow a redirect to `url`: HTTPS to an allowed host only, a few hops at most.
fn redirect_allowed(url: &Url, hops: usize) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|h| warden_core::policy::ALLOWED_OUTBOUND_HOSTS.contains(&h))
        && hops < 5
}

/// A short word for an updater error, for the log: never the response's own text.
fn error_kind(e: &tauri_plugin_updater::Error) -> &'static str {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Reqwest(_) | E::Network(_) | E::Http(_) => "network",
        E::ReleaseNotFound => "no release",
        E::Serialization(_) | E::Semver(_) | E::UrlParse(_) => "bad manifest",
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) => "bad signature",
        E::Io(_) => "file",
        _ => "updater",
    }
}

impl Source for Releases {
    type Found = Update;

    fn check(&self) -> Result<Option<(String, Update)>, String> {
        let h = self.handle.clone();
        let updater = self
            .handle
            .updater_builder()
            .version_comparator(|current, release| updates::is_newer(&current, &release.version))
            .configure_client(|client| {
                client
                    .connect_timeout(std::time::Duration::from_secs(30))
                    .read_timeout(std::time::Duration::from_secs(60))
                    .redirect(reqwest::redirect::Policy::custom(|attempt| {
                        if redirect_allowed(attempt.url(), attempt.previous().len()) {
                            attempt.follow()
                        } else {
                            attempt.stop()
                        }
                    }))
            })
            // Warden closes for the installer: its core stops first, as on Quit, and Tauri's own cleanup (the tray
            // icon) runs, which this hook replaces.
            .on_before_exit(move || {
                // The window's place is kept, as on Quit, so the restarted Warden opens where it was.
                save_place(&h);
                if let Some(core) = h.try_state::<Core>() {
                    core.app.stop();
                    let _ = core.app.finish_memory();
                }
                h.cleanup_before_exit();
            })
            .build()
            .map_err(|e| error_kind(&e).to_owned())?;
        let found = tauri::async_runtime::block_on(updater.check())
            .map_err(|e| error_kind(&e).to_owned())?;
        match found {
            // The manifest is not signed: its download address must be an allowed host too, not only its redirects.
            Some(u) if !redirect_allowed(&u.download_url, 0) => {
                Err("download address not allowed".into())
            }
            found => Ok(found.map(|u| (u.version.clone(), u))),
        }
    }

    fn download(
        &self,
        found: &Update,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, String> {
        let mut done = 0_u64;
        let chunk = |len: usize, total: Option<u64>| {
            done += len as u64;
            progress(done, total);
        };
        tauri::async_runtime::block_on(found.download(chunk, || {}))
            .map_err(|e| error_kind(&e).to_owned())
    }

    fn install(&self, found: &Update, bytes: &[u8], restart: bool) -> Result<(), String> {
        found
            .clone()
            .restart_after_install(restart)
            .install(bytes)
            .map_err(|e| error_kind(&e).to_owned())
    }
}

type ReleaseUpdates = Arc<Updates<Releases>>;

/// What only the desktop app does for the core: start with Windows, Explorer, and installing the update.
struct DesktopSide {
    run: RunValue,
    handle: AppHandle,
}

impl Desktop for DesktopSide {
    fn start_with_windows(&self) -> bool {
        self.run.is_set()
    }

    fn set_start_with_windows(&self, on: bool) -> io::Result<()> {
        self.run.set(on)
    }

    fn open_folder(&self, folder: &Path) -> io::Result<()> {
        fallback::open_folder(folder)
    }

    fn install_update(&self) -> io::Result<()> {
        let updates = self
            .handle
            .try_state::<ReleaseUpdates>()
            .map(|u| Arc::clone(&u))
            .ok_or_else(|| io::Error::other("this build does not update itself"))?;
        // On success Warden closes inside this call, so the page never gets its answer and keeps showing the update
        // installing; an error answers, and the page says so.
        updates.install(true).map_err(io::Error::other)
    }
}

/// The tray's "Install update and restart": asks first, as the window does.
fn confirm_install(handle: &AppHandle) {
    let Some(core) = handle.try_state::<Core>() else {
        return;
    };
    let Some(version) = core.app.update().filter(|u| u.ready).map(|u| u.version) else {
        return;
    };
    let h = handle.clone();
    handle
        .dialog()
        .message("Warden closes, installs the update and starts again.")
        .title(format!("Install Warden {version}?"))
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install and restart".into(),
            "Cancel".into(),
        ))
        .show(move |yes| {
            if yes && let Some(updates) = h.try_state::<ReleaseUpdates>() {
                let updates = Arc::clone(&updates);
                if updates.install(true).is_err() {
                    h.dialog()
                        .message("Warden could not install the update.")
                        .title("Warden")
                        .kind(MessageDialogKind::Error)
                        .show(|_| {});
                }
            }
        });
}

/// Whether an update is downloaded and ready, for the tray menu.
fn update_ready(handle: &AppHandle) -> bool {
    handle
        .try_state::<ReleaseUpdates>()
        .is_some_and(|u| u.ready())
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
    // Where it was last, while that is still on a screen; otherwise the default size, centred.
    let screens: Vec<Screen> = handle
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Screen {
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
        })
        .collect();
    let saved = window_state::usable(
        store::load(&core.data_dir, window_state::FILE).unwrap_or_default(),
        &screens,
    )
    .map(|p| window_state::opening(p, &screens));
    let built = WebviewWindowBuilder::new(handle, WINDOW, WebviewUrl::CustomProtocol(own.clone()))
        .title("Warden")
        .inner_size(1280.0, 800.0)
        .min_inner_size(800.0, 560.0)
        .center()
        // Shown once it is in its place, so it does not jump.
        .visible(saved.is_none())
        .decorations(false)
        .data_directory(core.webview_dir.clone())
        // The window shows only Warden's own page; links elsewhere and new windows are refused.
        .on_navigation(move |url| url.origin() == own.origin())
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build();
    let window = match built {
        Ok(w) => w,
        Err(e) => {
            (core.app.log)(&format!("could not open the window: {e}"));
            return;
        }
    };
    if let Some(p) = saved {
        // The screen first, then the size, so a screen with another scale does not change the size afterwards.
        let _ = window.set_position(PhysicalPosition::new(p.x, p.y));
        let _ = window.set_size(PhysicalSize::new(p.width, p.height));
        if p.maximized {
            let _ = window.maximize();
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
    *lock_place(&core) = saved;
    follow_place(handle, &window);
    let _ = window.set_icon(taskbar_icon(window.scale_factor().unwrap_or(1.0)));
    let h = handle.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            if let Some(w) = h.get_webview_window(WINDOW) {
                follow_place(&h, &w);
            }
        }
        // On a screen with another scale, the taskbar button's icon at that scale.
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            if let Some(w) = h.get_webview_window(WINDOW) {
                let _ = w.set_icon(taskbar_icon(*scale_factor));
            }
        }
        WindowEvent::Destroyed => save_place(&h),
        _ => {}
    });
}

/// The app icon's ICO layers the taskbar button can use, smallest first, each a square of RGBA pixels decoded from
/// `icons/icon.ico` by the build script.
const TASKBAR_ICONS: [(u32, &[u8]); 4] = [
    (
        24,
        include_bytes!(concat!(env!("OUT_DIR"), "/app-icon-24.rgba")),
    ),
    (
        32,
        include_bytes!(concat!(env!("OUT_DIR"), "/app-icon-32.rgba")),
    ),
    (
        48,
        include_bytes!(concat!(env!("OUT_DIR"), "/app-icon-48.rgba")),
    ),
    (
        64,
        include_bytes!(concat!(env!("OUT_DIR"), "/app-icon-64.rgba")),
    ),
];

/// The window's icon for its taskbar button on a screen of this scale: the ICO's layer for that size, or the next
/// larger one. Tauri's default window icon is the ICO's first layer (16 pixels), which the taskbar enlarges to its 24
/// and blurs.
fn taskbar_icon(scale: f64) -> tauri::image::Image<'static> {
    let (size, rgba) = TASKBAR_ICONS[taskbar_layer(icon_px(TASKBAR_ICON, scale))];
    tauri::image::Image::new(rgba, size, size)
}

/// Which of `TASKBAR_ICONS` to use for an icon this many pixels square: the smallest that is at least as large (a
/// smaller one Windows would have to enlarge), else the largest.
fn taskbar_layer(px: u32) -> usize {
    TASKBAR_ICONS
        .iter()
        .position(|(size, _)| *size >= px)
        .unwrap_or(TASKBAR_ICONS.len() - 1)
}

fn icon_px(logical: f64, scale: f64) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (logical * scale).round().max(1.0) as u32
}

fn lock_place(core: &Core) -> std::sync::MutexGuard<'_, Option<Place>> {
    core.place
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Takes in where the window is now and how big, and whether it is minimized or maximized.
fn follow_place(handle: &AppHandle, window: &WebviewWindow) {
    let Some(core) = handle.try_state::<Core>() else {
        return;
    };
    let (Ok(at), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    let seen = Seen {
        x: at.x,
        y: at.y,
        width: size.width,
        height: size.height,
        minimized: window.is_minimized().unwrap_or(false),
        maximized: window.is_maximized().unwrap_or(false),
    };
    let mut place = lock_place(&core);
    *place = window_state::follow(*place, seen);
}

/// Keeps the window's place in the data folder: when the window closes and when Warden quits.
fn save_place(handle: &AppHandle) {
    let Some(core) = handle.try_state::<Core>() else {
        return;
    };
    let Some(place) = *lock_place(&core) else {
        return;
    };
    if let Err(e) = store::save(&core.data_dir, window_state::FILE, &place) {
        (core.app.log)(&format!("could not keep the window's place ({})", e.kind()));
    }
}

/// The tray's menu: Open, Install update and restart while an update is ready, Quit.
fn tray_menu(handle: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(handle, "open", "Open Warden", true, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "Quit Warden", true, None::<&str>)?;
    if update_ready(handle) {
        let install = MenuItem::with_id(
            handle,
            "install",
            "Install update and restart",
            true,
            None::<&str>,
        )?;
        Menu::with_items(handle, &[&open, &install, &quit])
    } else {
        Menu::with_items(handle, &[&open, &quit])
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
        let menu = tray_menu(handle)?;
        let mut tray = TrayIconBuilder::with_id(TRAY)
            .tooltip("Warden")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "open" => open_window(app),
                "install" => confirm_install(app),
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

/// Follows the core's state without polling: the tray icon with Keep running in the tray (its menu offers a ready
/// update), and a notification for each identity change.
///
/// It starts from the core's first revision, so whatever changed while the window was being built is caught up.
fn follow(handle: AppHandle, app: Arc<App>, mut alerts: Alerts, mut tray: bool) {
    let mut revision = 0;
    let mut ready = update_ready(&handle);
    while let Some(r) = app.wait_for_change(revision) {
        revision = r;
        let now_ready = update_ready(&handle);
        if now_ready != ready && tray {
            let h = handle.clone();
            // The same icon gets the new menu, so a pinned icon stays pinned.
            let _ = handle.run_on_main_thread(move || {
                if let (Some(tray), Ok(menu)) = (h.tray_by_id(TRAY), tray_menu(&h)) {
                    let _ = tray.set_menu(Some(menu));
                }
            });
        }
        ready = now_ready;
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
    self_update: bool,
) -> Result<(), String> {
    let desktop = RunValue::for_this_app(&args.options())
        .map_err(|e| format!("could not find Warden's own program: {e}"))?;
    let webview_dir = data_dir.join("webview");
    let update_dir = data_dir.join("update");
    let place_dir = data_dir.clone();
    let saved: Readings = store::load(&data_dir, READINGS_FILE).unwrap_or_default();
    let alerts = Alerts::from_saved(&saved);
    window_csp(handle).ok_or("the app has no Content-Security-Policy")?;
    let env = args.environment();
    let cli_version = warden_core::claude::installed_cli_version(&env);
    let core = start_core(Config {
        data_dir,
        ui_dir,
        env,
        endpoint: Arc::new(AnthropicUsageEndpoint::new(cli_version.as_deref())),
        resets: Arc::new(warden_core::codex_resets::ChatGptResetsEndpoint::new()),
        sign_in_server: Arc::new(warden_core::mainlogin::AnthropicSignInServer::default()),
        sign_in: Arc::new(fallback::start_sign_in),
        log: Arc::new(|line| eprintln!("{} warden: {line}", now_ms())),
        desktop: Some(Arc::new(DesktopSide {
            run: desktop,
            handle: handle.clone(),
        })),
        route_exe,
        memory: Arc::new(warden_server::procs::SystemMemory::new()),
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
        data_dir: place_dir,
        place: std::sync::Mutex::new(None),
    });

    if self_update {
        let (to_core, to_log) = (Arc::clone(&app), Arc::clone(&app));
        let releases: ReleaseUpdates = Arc::new(Updates::new(
            Releases {
                handle: handle.clone(),
            },
            update_dir.clone(),
            move |update| to_core.set_update(update),
            move |line| (to_log.log)(line),
        ));
        handle.manage(Arc::clone(&releases));
        std::thread::spawn(move || releases.run(updates::FIRST_CHECK, updates::EVERY));
    }

    let keep = app.keep_in_tray();
    set_tray(handle, keep);
    // After "Install and restart" the window comes back, even when Windows started the old Warden into the tray.
    let after_update = updates::take_open_window_mark(&update_dir);
    if after_update || !(args.autostart && keep) {
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

/// `self_update`: this is the installed app in a release build, which keeps itself up to date.
pub fn run(
    args: Args,
    data_dir: PathBuf,
    ui_dir: Option<PathBuf>,
    route_exe: Option<PathBuf>,
    self_update: bool,
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![pick_folder, quit])
        // The page and its API, in-process: each request on a thread of its own, since the long poll waits.
        .register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
            let handle = ctx.app_handle().clone();
            std::thread::spawn(move || responder.respond(answer_window(&handle, &request)));
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            if let Err(e) = start_app(&handle, &args, data_dir, ui_dir, route_exe, self_update) {
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
        // The last window closed (`code: None`) or Quit (`Some`): stay in the tray when the window closed and that is
        // on; otherwise the user is quitting, so a downloaded update may install as Warden exits.
        RunEvent::ExitRequested { code, api, .. } => {
            if code.is_none()
                && handle
                    .try_state::<Core>()
                    .is_some_and(|c| c.app.keep_in_tray())
            {
                api.prevent_exit();
            } else if let Some(updates) = handle.try_state::<ReleaseUpdates>() {
                updates.quit_requested();
            }
        }
        RunEvent::Exit => {
            // Quit with the window open: its place is kept as when it closes.
            save_place(handle);
            // The memory minute being gathered is written now: the process may end before the sampler wakes.
            if let Some(core) = handle.try_state::<Core>() {
                let _ = core.app.finish_memory();
            }
            // Only after the user's quit (never when Windows ends the session) a downloaded update installs, without
            // starting Warden again; Warden closes for it.
            if let Some(updates) = handle.try_state::<ReleaseUpdates>() {
                updates.on_exit();
            }
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
    fn the_taskbar_gets_the_icon_layer_for_its_size() {
        // The taskbar's 24 pixels at 100%, 125%, 150%, 200%, 250% and 300%: the layer for that size, else the next
        // larger, else the largest there is.
        let layer = |scale| TASKBAR_ICONS[taskbar_layer(icon_px(TASKBAR_ICON, scale))].0;
        assert_eq!(layer(1.0), 24);
        assert_eq!(layer(1.25), 32);
        assert_eq!(layer(1.5), 48);
        assert_eq!(layer(2.0), 48);
        assert_eq!(layer(2.5), 64);
        assert_eq!(layer(3.0), 64);
        // A scale Windows did not give counts as 100%.
        assert_eq!(layer(0.0), 24);
        assert_eq!(layer(f64::NAN), 24);
        // Each layer is a whole square of RGBA pixels.
        for (size, rgba) in TASKBAR_ICONS {
            assert_eq!(rgba.len(), (size * size * 4) as usize, "layer {size}");
        }
    }

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
    fn the_update_download_follows_redirects_only_to_allowed_hosts() {
        let url = |u: &str| Url::parse(u).expect("url");
        for allowed in [
            "https://github.com/Eterion/Warden/releases/latest/download/latest.json",
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1",
        ] {
            assert!(redirect_allowed(&url(allowed), 1), "{allowed}");
        }
        for refused in [
            "https://example.com/latest.json",
            "https://github.com.example.com/x",
        ] {
            assert!(!redirect_allowed(&url(refused), 1), "{refused}");
        }
        let manifest =
            url("https://github.com/Eterion/Warden/releases/latest/download/latest.json");
        let mut plain = manifest.clone();
        plain.set_scheme("http").expect("http");
        assert!(!redirect_allowed(&plain, 1), "plain HTTP");
        assert!(!redirect_allowed(&manifest, 5), "too many hops");
    }

    #[test]
    fn the_updater_reads_the_repositorys_latest_release() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let updater = &config["plugins"]["updater"];
        assert_eq!(
            updater["endpoints"],
            serde_json::json!([
                "https://github.com/Eterion/Warden/releases/latest/download/latest.json"
            ])
        );
        assert_eq!(updater["windows"]["installMode"], "passive");
        assert!(updater["pubkey"].as_str().is_some_and(|k| !k.is_empty()));
        // Only the release workflow signs the update artifacts; a local installer build needs no key.
        let release: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.release.conf.json")).expect("release");
        assert_eq!(release["bundle"]["createUpdaterArtifacts"], true);
        let installer: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.installer.conf.json")).expect("installer");
        assert!(installer["bundle"].get("createUpdaterArtifacts").is_none());
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
