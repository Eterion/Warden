//! Rule: the page runs under a strict Content-Security-Policy: everything is refused unless named, scripts and
//! styles come only from the app itself (nothing inline, no `eval`), and it connects only to the app itself.

/// The directives of a policy, each with its sources.
fn directives(policy: &str) -> Vec<(String, Vec<String>)> {
    policy
        .split(';')
        .filter_map(|d| {
            let mut words = d.split_whitespace();
            let name = words.next()?.to_ascii_lowercase();
            Some((name, words.map(str::to_owned).collect()))
        })
        .collect()
}

/// Panics unless `policy` is strict: `default-src 'none'`; `script-src` and `style-src` exactly `'self'`; every
/// other source `'self'` or `'none'`, except that `connect-src` may also name `connect` (the desktop app's bridge to
/// its window, for example); `base-uri`, `form-action` and `frame-ancestors` set to `'none'`.
pub fn assert_strict(policy: &str, connect: &[&str]) {
    let all = directives(policy);
    let get = |name: &str| {
        all.iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| panic!("rule broken: the policy has no {name} ({policy})"))
    };
    let only = |name: &str, want: &[&str]| {
        let got = get(name);
        assert!(
            got.len() == want.len() && got.iter().zip(want).all(|(g, w)| g == w),
            "rule broken: {name} must be {} but is {} ({policy})",
            want.join(" "),
            got.join(" ")
        );
    };
    only("default-src", &["'none'"]);
    only("script-src", &["'self'"]);
    only("style-src", &["'self'"]);
    for name in ["base-uri", "form-action", "frame-ancestors"] {
        only(name, &["'none'"]);
    }
    get("connect-src");
    for (name, sources) in &all {
        for s in sources {
            let ok = s == "'self'"
                || s == "'none'"
                || (name == "connect-src" && connect.contains(&s.as_str()));
            assert!(ok, "rule broken: {name} allows {s} ({policy})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRICT: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

    #[test]
    fn a_strict_policy_passes() {
        assert_strict(STRICT, &[]);
        assert_strict(
            &STRICT.replace(
                "connect-src 'self'",
                "connect-src 'self' ipc: http://ipc.localhost",
            ),
            &["ipc:", "http://ipc.localhost"],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken: script-src")]
    fn inline_scripts_are_caught() {
        assert_strict(
            &STRICT.replace("script-src 'self'", "script-src 'self' 'unsafe-inline'"),
            &[],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken: style-src")]
    fn inline_styles_are_caught() {
        assert_strict(
            &STRICT.replace("style-src 'self'", "style-src 'self' 'unsafe-inline'"),
            &[],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken: connect-src allows https://example.com")]
    fn connections_elsewhere_are_caught() {
        assert_strict(
            &STRICT.replace(
                "connect-src 'self'",
                "connect-src 'self' https://example.com",
            ),
            &[],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken: img-src allows data:")]
    fn other_sources_are_caught() {
        assert_strict(
            &STRICT.replace("img-src 'self'", "img-src 'self' data:"),
            &[],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken: default-src")]
    fn an_open_default_is_caught() {
        assert_strict(&STRICT.replace("default-src 'none'", "default-src *"), &[]);
    }

    #[test]
    #[should_panic(expected = "rule broken: the policy has no script-src")]
    fn a_missing_directive_is_caught() {
        assert_strict(&STRICT.replace("script-src 'self';", ""), &[]);
    }
}
