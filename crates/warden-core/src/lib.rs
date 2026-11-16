//! Warden's core library, shared by the local server and the `warden` CLI.
//!
//! Accounts and their identity, Claude usage from the usage endpoint, Codex usage from the session logs, and
//! Warden's saved settings and readings, the routing rules and the hooks that carry them.

pub mod claude;
pub mod codex;
pub mod codex_resets;
pub mod detect;
pub mod fallback;
pub mod hooks;
pub mod identity;
pub mod logfile;
pub mod mainlogin;
pub mod model;
pub mod pipe;
pub mod policy;
pub mod refresh;
pub mod routing;
pub mod secret;
pub mod sessions;
pub mod store;
#[doc(hidden)]
pub mod testdir;
pub mod time;
pub mod titles;
