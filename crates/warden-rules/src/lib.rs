//! Test harness for Warden's own rules (`warden-plan.md`, Review, layers 1 and 3).
//!
//! Later steps apply these checks to the real code in `crates/warden-server/tests/rules.rs`.
//! Each check panics with a message starting "rule broken", so a failing test reads as a rule break.

pub mod csp;
pub mod hook_data;
pub mod hosts;
pub mod memory_history;
pub mod net;
pub mod secrets;
