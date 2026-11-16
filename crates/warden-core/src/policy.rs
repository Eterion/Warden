//! Security policy the rule tests hold the code to (see `AGENTS.md`, Rules).

use std::net::{IpAddr, Ipv4Addr};

/// The only address the local server may listen on.
pub const BIND_IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// The only hosts Warden may call over the network: Anthropic's usage endpoint and sign-in server, ChatGPT's backend
/// for Codex accounts' banked resets, and the GitHub hosts the installed app's updates come from (the release manifest
/// and installer on `github.com`, which sends the download on to GitHub's release asset hosts).
pub const ALLOWED_OUTBOUND_HOSTS: &[&str] = &[
    "api.anthropic.com",
    "platform.claude.com",
    "chatgpt.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_to_loopback() {
        assert!(BIND_IP.is_loopback());
    }
}
