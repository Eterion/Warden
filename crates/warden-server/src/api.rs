//! The API routes and the built UI's files, apart from how a request arrives: the development server answers them
//! over HTTP on 127.0.0.1 (`http.rs`), the desktop app in-process through its custom URL scheme.

use crate::app::{ApiError, ApiResult, App, SettingsPatch};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::{Component, Path, PathBuf};
use warden_core::model::Provider;

/// How long a UI request waits for a change before answering anyway.
const LONG_POLL: std::time::Duration = std::time::Duration::from_secs(50);

/// The largest request body accepted.
pub const MAX_BODY: usize = 64 * 1024;

/// The headers every answer carries, whichever way it is sent. Each transport adds its own
/// `Content-Security-Policy`.
pub const SECURITY_HEADERS: [(&str, &str); 4] = [
    ("X-Content-Type-Options", "nosniff"),
    ("Cache-Control", "no-store"),
    ("Referrer-Policy", "no-referrer"),
    ("X-Frame-Options", "DENY"),
];

/// One answer: its status, content type and body.
pub struct Answer {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

const JSON: &str = "application/json; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

impl Answer {
    fn json(status: u16, body: &impl serde::Serialize) -> Answer {
        Answer {
            status,
            content_type: JSON,
            body: serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec()),
        }
    }

    pub fn error(e: ApiError) -> Answer {
        Answer::json(e.0, &serde_json::json!({ "error": e.1 }))
    }

    fn text(status: u16, text: &str) -> Answer {
        Answer {
            status,
            content_type: TEXT,
            body: text.as_bytes().to_vec(),
        }
    }
}

fn query_u64(url: &str, key: &str) -> Option<u64> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

fn body<T: DeserializeOwned>(body: &[u8]) -> ApiResult<T> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::bad("The request is not in the expected shape."))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddBody {
    provider: Provider,
    folder: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewBody {
    provider: Provider,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchBody {
    label: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MainBody {
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityBody {
    action: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupBody {
    accounts: Vec<AddBody>,
    #[serde(default)]
    skip: bool,
}

/// The path of a request's URL, without its query.
pub fn path_of(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or("/")
}

/// Answers one request the transport has let through: `method` in capitals, `url` the path and query, `body` as
/// sent (more than [`MAX_BODY`] is refused). The long poll (`GET /api/state/wait`) holds the calling thread.
pub fn answer(app: &App, ui_dir: Option<&Path>, method: &str, url: &str, body: &[u8]) -> Answer {
    let path = path_of(url);
    if !path.starts_with("/api/") {
        if method != "GET" && method != "HEAD" {
            return Answer::error(ApiError(405, "Only GET is allowed here.".into()));
        }
        return serve_file(path, ui_dir);
    }
    if body.len() > MAX_BODY {
        return Answer::error(ApiError(413, "The request is too large.".into()));
    }
    let segments: Vec<&str> = path
        .trim_start_matches("/api/")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if method == "GET" && segments == ["state", "wait"] {
        let since = query_u64(url, "since").unwrap_or(0);
        return Answer::json(200, &app.state_after(since, LONG_POLL));
    }
    let result: ApiResult<Option<serde_json::Value>> = (|| {
        match (method, segments.as_slice()) {
            ("GET", ["state"]) => {}
            ("POST", ["usage", "refresh"]) => {
                // Identity first, so listed sessions count for the account signed in now. By hand: every account's
                // limit resets too.
                app.refresh_usage();
                app.list_sessions();
            }
            ("PUT", ["settings"]) => {
                let b: SettingsPatch = self::body(body)?;
                app.set_settings(b)?;
            }
            ("POST", ["tools", "scan"]) => app.scan_tools(),
            ("POST", ["main", "sign-in"]) => app.sign_in_main()?,
            ("GET", ["setup", "candidates"]) => {
                return Ok(Some(
                    serde_json::to_value(app.candidates()).unwrap_or_default(),
                ));
            }
            ("POST", ["setup"]) => {
                let b: SetupBody = self::body(body)?;
                let chosen: Vec<_> = b
                    .accounts
                    .into_iter()
                    .map(|a| (a.provider, a.folder, a.label))
                    .collect();
                app.finish_setup(&chosen, b.skip)?;
            }
            ("POST", ["setup", "restart"]) => app.restart_setup()?,
            ("POST", ["accounts"]) => {
                let b: AddBody = self::body(body)?;
                app.add_existing(b.provider, &b.folder, &b.label)?;
            }
            ("POST", ["accounts", "new"]) => {
                let b: NewBody = self::body(body)?;
                app.add_new(b.provider, &b.label)?;
            }
            ("PATCH", ["accounts", id]) => {
                let b: PatchBody = self::body(body)?;
                app.update_account(id, b.label.as_deref(), b.enabled)?;
            }
            ("POST", ["accounts", id, "identity"]) => {
                let b: IdentityBody = self::body(body)?;
                if b.action != "accept" {
                    return Err(ApiError::bad("The only identity action is accept."));
                }
                app.accept_identity(id)?;
            }
            ("DELETE", ["accounts", id]) => app.remove_account(id)?,
            ("PATCH", ["main"]) => {
                let b: MainBody = self::body(body)?;
                app.rename_main(&b.label)?;
            }
            ("POST", ["accounts", id, "sign-in"]) => app.sign_in_account(id)?,
            ("POST", ["accounts", id, "hooks", "repair"]) => app.repair_account_hooks(id)?,
            ("POST", ["logs", "open"]) => app.open_log_folder()?,
            ("POST", ["update", "install"]) => app.install_update()?,
            _ => return Err(ApiError(404, "There is no such API.".into())),
        }
        Ok(None)
    })();
    match result {
        Ok(Some(value)) => Answer::json(200, &value),
        Ok(None) => Answer::json(200, &app.state()),
        Err(e) => Answer::error(e),
    }
}

/// A file of the built UI. Paths are taken one plain component at a time, so nothing outside the UI folder can be
/// reached; unknown paths get the app's page.
fn serve_file(path: &str, ui_dir: Option<&Path>) -> Answer {
    let Some(root) = ui_dir else {
        return Answer::text(
            404,
            "Warden's UI is not built into this server. Run it with --ui-dir ui/dist, or use npm run dev -w ui.",
        );
    };
    let relative = PathBuf::from(path.trim_start_matches('/'));
    let safe = relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        && !path.contains('\\')
        && !path.contains(':');
    let file = if safe && !relative.as_os_str().is_empty() && root.join(&relative).is_file() {
        root.join(&relative)
    } else {
        root.join("index.html")
    };
    match std::fs::read(&file) {
        Ok(body) => Answer {
            status: 200,
            content_type: content_type(&file),
            body,
        },
        Err(_) => Answer::text(404, "Not found"),
    }
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "json" => "application/json",
        "txt" => TEXT,
        _ => "application/octet-stream",
    }
}
