//! Security policy the rule tests hold the code to (see `AGENTS.md`, Rules).

use std::net::{IpAddr, Ipv4Addr};

/// The only address the local server may listen on.
pub const BIND_IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// The only hosts Warden may call over the network: Anthropic's usage endpoint and sign-in server.
pub const ALLOWED_OUTBOUND_HOSTS: &[&str] = &["api.anthropic.com", "platform.claude.com"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_to_loopback() {
        assert!(BIND_IP.is_loopback());
    }
}
