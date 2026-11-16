//! Warden's core and its API (`docs/api.md`): reads usage on its interval and from the Codex logs as they change, and
//! answers the UI. The desktop app answers in-process ([`start_core`], `api.rs`); the development server ([`start`])
//! also listens on 127.0.0.1, behind a per-run token. The hooks reach the core over the per-user pipe
//! (`warden_core::pipe`), never over HTTP.

pub mod api;
pub mod app;
pub mod http;
pub mod procs;

use app::{App, Desktop, Log, SignIn};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use warden_core::claude::UsageEndpoint;
use warden_core::detect::Env;
use warden_core::pipe::Listener;
use warden_core::policy::BIND_IP;

/// The port the development server listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 4777;
/// Request workers. An open window holds one in its long poll for changes.
const WORKERS: usize = 6;
/// Hook connections handled at once, each on a small thread of its own; one more is hung up on at once.
const MAX_PIPE_CONNECTIONS: usize = 32;
/// A connection's thread only reads one line and answers: a small stack.
const PIPE_STACK: usize = 128 * 1024;

pub struct Config {
    pub data_dir: PathBuf,
    pub ui_dir: Option<PathBuf>,
    pub env: Env,
    pub endpoint: Arc<dyn UsageEndpoint>,
    pub sign_in_server: Arc<dyn warden_core::mainlogin::SignInServer>,
    /// ChatGPT's backend, for Codex accounts' banked resets.
    pub resets: Arc<dyn warden_core::codex_resets::ResetsEndpoint>,
    pub sign_in: SignIn,
    pub log: Log,
    /// The `warden` binary the routing hook runs (`None`: not found).
    pub route_exe: Option<PathBuf>,
    /// The desktop app's own parts; `None` in the browser version.
    pub desktop: Option<Arc<dyn Desktop>>,
}

/// The running core: the app and its background threads, without a listener.
pub struct Core {
    pub app: Arc<App>,
    /// The built UI, served with the API.
    pub ui_dir: Option<PathBuf>,
    /// The hooks' pipe; `None` when it could not be opened.
    pipe: Option<Arc<Listener>>,
    /// The thread taking the pipe's connections.
    pipe_threads: Vec<JoinHandle<()>>,
    threads: Vec<JoinHandle<()>>,
}

impl Core {
    /// Stops every background thread.
    pub fn stop(self) {
        self.app.stop();
        // The acceptor returns once a connection comes; between two accepts it has no instance waiting, so it is
        // woken until it has gone (a moment at most). Connections being handled are not waited for: each ends when
        // its hook does.
        if let Some(pipe) = &self.pipe {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while self.pipe_threads.iter().any(|t| !t.is_finished())
                && std::time::Instant::now() < deadline
            {
                pipe.wake();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        for t in self.pipe_threads.into_iter().chain(self.threads) {
            let _ = t.join();
        }
    }
}

/// Starts the core: Warden's own outdated hooks rewritten, the hooks' pipe, the usage loop, the Codex log watcher and
/// the hook file watcher. Nothing listens on the network.
pub fn start_core(config: Config) -> io::Result<Core> {
    let app = App::new(
        config.data_dir,
        config.env,
        (config.endpoint, config.sign_in_server, config.resets),
        config.sign_in,
        config.log,
        config.route_exe,
        config.desktop,
    )?
    .shared();
    app.repair_hooks();
    // The hooks' pipe, for the current user alone. Without it the status hooks report nothing, but the rest works.
    let pipe = match procs::current_user_sid().and_then(|sid| Listener::create(&app.data_dir, &sid))
    {
        Ok(listener) => Some(Arc::new(listener)),
        Err(e) => {
            (app.log)(&format!("could not open the hooks' pipe: {e}"));
            None
        }
    };
    let mut pipe_threads = Vec::new();
    if let Some(pipe) = pipe.as_ref().map(Arc::clone) {
        let app = Arc::clone(&app);
        pipe_threads.push(std::thread::spawn(move || {
            // Each connection on its own thread, so one hook that never finishes its message (it is the user's own
            // process, and `warden event` gives up within a second) holds up no other.
            let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            while let Ok(mut conn) = pipe.accept() {
                if app.stopped() {
                    break;
                }
                if open.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_PIPE_CONNECTIONS {
                    open.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    continue;
                }
                let (app, done) = (Arc::clone(&app), Arc::clone(&open));
                let spawned = std::thread::Builder::new()
                    .stack_size(PIPE_STACK)
                    .spawn(move || {
                        let client = conn.client_pid();
                        if let Ok(message) = conn.read() {
                            // An unknown account or a malformed event is dropped; the hook is answered either way.
                            let _ = app.receive_message(&message, client);
                        }
                        conn.answer();
                        done.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    open.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }));
    }
    let mut threads = Vec::new();
    {
        let app = Arc::clone(&app);
        threads.push(std::thread::spawn(move || {
            // Usage on its interval: one small request per Claude account, then wait. After the first reading
            // (identity checked), sessions from the CLIs' own lists once, for accounts no hook reports yet.
            let mut first = true;
            loop {
                app.read_claude_usage();
                if std::mem::take(&mut first) {
                    app.list_sessions();
                }
                if !app.wait(app.usage_interval()) {
                    break;
                }
            }
        }));
    }
    {
        let app = Arc::clone(&app);
        threads.push(std::thread::spawn(move || app::watch_codex(&app)));
    }
    {
        let app = Arc::clone(&app);
        threads.push(std::thread::spawn(move || app::watch_hook_files(&app)));
    }
    Ok(Core {
        app,
        ui_dir: config.ui_dir,
        pipe,
        pipe_threads,
        threads,
    })
}

/// The development server: the core, and a listener on 127.0.0.1 that answers only requests carrying its token.
pub struct Running {
    pub addr: SocketAddr,
    pub app: Arc<App>,
    pub token: Arc<http::Token>,
    core: Core,
    server: Arc<tiny_http::Server>,
    workers: Vec<JoinHandle<()>>,
}

impl Running {
    /// The address to open: the page, with this run's token.
    pub fn url(&self) -> String {
        format!("http://{}/?token={}", self.addr, self.token.value())
    }

    /// Stops every thread and closes the listener.
    pub fn stop(self) {
        self.app.stop();
        for _ in 0..WORKERS {
            self.server.unblock();
        }
        for t in self.workers {
            let _ = t.join();
        }
        self.core.stop();
    }

    /// Waits until the server stops (it runs until the process ends).
    pub fn wait(self) {
        for t in self.workers {
            let _ = t.join();
        }
        self.core.stop();
    }
}

/// Starts the development server: the core, and the listener on 127.0.0.1 with its request workers. `port` 0 picks
/// a free port (tests).
pub fn start(config: Config, port: u16) -> io::Result<Running> {
    let core = start_core(config)?;
    let server = match tiny_http::Server::http(SocketAddr::new(BIND_IP, port)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            core.stop();
            return Err(io::Error::other(e));
        }
    };
    let Some(addr) = server.server_addr().to_ip() else {
        core.stop();
        return Err(io::Error::other("the listener has no IP address"));
    };
    let token = match http::Token::new(addr.port()) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            core.stop();
            return Err(e);
        }
    };
    let app = Arc::clone(&core.app);
    (app.log)(&format!("listening on http://{addr}"));

    let mut workers = Vec::new();
    for _ in 0..WORKERS {
        let (server, app, token) = (Arc::clone(&server), Arc::clone(&app), Arc::clone(&token));
        let ui_dir = core.ui_dir.clone();
        let port = addr.port();
        workers.push(std::thread::spawn(move || {
            while let Ok(req) = server.recv() {
                if app.stopped() {
                    break;
                }
                http::handle(&app, req, port, &token, ui_dir.as_deref());
            }
        }));
    }
    Ok(Running {
        addr,
        app,
        token,
        core,
        server,
        workers,
    })
}
