//! The API's error paths: bad input, unknown ids and refused changes answer with a reason and change nothing.

mod common;

use common::{Fixture, call};
use serde_json::{Value, json};
use std::net::SocketAddr;

fn state(addr: SocketAddr) -> Value {
    call(addr, "GET", "/api/state", None).json()
}

/// The accounts without their usage readings, which the server takes in by itself at any moment.
fn accounts(s: &Value) -> Value {
    let mut list = s["accounts"].clone();
    for a in list.as_array_mut().expect("accounts") {
        let a = a.as_object_mut().expect("account");
        a.remove("usage");
        a.remove("usageError");
    }
    list
}

/// Two Claude accounts, main first; returns their ids.
fn two_accounts(f: &Fixture, addr: SocketAddr) -> (String, String) {
    for (name, label) in [(".claude-a", "Main"), (".claude-b", "Second")] {
        let folder = f.claude_folder(name, &format!("{label}@acme.example"), "t");
        let body = json!({ "provider": "claude", "folder": folder, "label": label }).to_string();
        let reply = call(addr, "POST", "/api/accounts", Some(&body));
        assert_eq!(reply.status, 200, "{}", reply.body);
    }
    let s = state(addr);
    let id = |i: usize| s["accounts"][i]["id"].as_str().expect("id").to_owned();
    (id(0), id(1))
}

/// The status and the error text of a refused request.
fn refused(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let reply = call(addr, method, path, body);
    let error = reply.json()["error"]
        .as_str()
        .unwrap_or_else(|| panic!("no error text: {}", reply.body))
        .to_owned();
    (reply.status, error)
}

const NO_ACCOUNT: &str = "There is no such account.";
const BAD_SHAPE: &str = "The request is not in the expected shape.";

#[test]
fn unknown_account_ids_are_not_found() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    two_accounts(&f, addr);
    let before = accounts(&state(addr));

    for (method, path, body) in [
        ("PATCH", "/api/accounts/nope", Some(r#"{"label": "X"}"#)),
        ("DELETE", "/api/accounts/nope", None),
        ("POST", "/api/accounts/nope/hooks/repair", None),
    ] {
        assert_eq!(
            refused(addr, method, path, body),
            (404, NO_ACCOUNT.to_owned()),
            "{method} {path}"
        );
    }
    assert_eq!(accounts(&state(addr)), before, "nothing changed");
    server.stop();
}

#[test]
fn bodies_in_the_wrong_shape_are_refused() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let (_, second) = two_accounts(&f, addr);
    let patch = format!("/api/accounts/{second}");
    let before = state(addr);

    for (method, path, body) in [
        ("PATCH", patch.as_str(), "not json"),
        ("PATCH", patch.as_str(), ""),
        ("PATCH", patch.as_str(), r#"{"label": 5}"#),
        ("PATCH", patch.as_str(), r#"{"colour": "red"}"#),
        ("PATCH", patch.as_str(), r#"{"enabled":"yes"}"#),
        ("PATCH", "/api/main", r#"{"label":5}"#),
        (
            "POST",
            "/api/accounts",
            r#"{"provider": "gemini", "folder": "x", "label": "x"}"#,
        ),
        ("POST", "/api/accounts/new", r#"{"provider": "claude"}"#),
    ] {
        assert_eq!(
            refused(addr, method, path, Some(body)),
            (400, BAD_SHAPE.to_owned()),
            "{method} {path} {body}"
        );
    }
    let after = state(addr);
    assert_eq!(accounts(&after), accounts(&before));
    server.stop();
}

#[test]
fn a_body_over_the_limit_is_refused_as_too_large() {
    let f = Fixture::new();
    let server = f.start();
    let label = "x".repeat(70 * 1024);
    let body = json!({ "theme": label }).to_string();
    let (status, error) = refused(server.addr, "PUT", "/api/settings", Some(&body));
    assert_eq!((status, error.as_str()), (413, "The request is too large."));
    server.stop();
}

#[test]
fn unknown_routes_and_methods_are_refused() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let no_api = (404, "There is no such API.".to_owned());
    // A known path with a method it does not take is no API either.
    assert_eq!(refused(addr, "DELETE", "/api/state", None), no_api);
    assert_eq!(refused(addr, "GET", "/api/accounts", None), no_api);
    assert_eq!(refused(addr, "POST", "/api/routing", Some("{}")), no_api);
    assert_eq!(refused(addr, "GET", "/api/accounts/a/b/c", None), no_api);
    for (method, path) in [
        ("PUT", "/api/accounts/order"),
        ("PUT", "/api/routing"),
        ("PUT", "/api/accounts/a1/hooks"),
        ("POST", "/api/hooks/reinstall"),
        ("DELETE", "/api/hooks/updated"),
    ] {
        assert_eq!(
            refused(addr, method, path, Some("{}")),
            no_api,
            "{method} {path}"
        );
    }
    // Outside the API only reading is allowed.
    assert_eq!(
        refused(addr, "POST", "/index.html", Some("{}")),
        (405, "Only GET is allowed here.".to_owned())
    );
    assert_eq!(
        refused(addr, "DELETE", "/", None),
        (405, "Only GET is allowed here.".to_owned())
    );
    server.stop();
}

#[test]
fn names_are_checked_before_anything_changes() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let (_, second) = two_accounts(&f, addr);
    let patch = format!("/api/accounts/{second}");

    let (status, error) = refused(addr, "PATCH", &patch, Some(r#"{"label": "   "}"#));
    assert_eq!((status, error.as_str()), (400, "Give the account a name."));
    let long = json!({ "label": "n".repeat(61) }).to_string();
    let (status, error) = refused(addr, "PATCH", &patch, Some(&long));
    assert_eq!(
        (status, error.as_str()),
        (400, "Keep the name under 60 characters.")
    );
    let s = state(addr);
    assert_eq!(s["accounts"][1]["id"], second.as_str(), "not made main");
    assert_eq!(s["accounts"][1]["label"], "Second");

    // A new account without a name starts no sign-in.
    let (status, _) = refused(
        addr,
        "POST",
        "/api/accounts/new",
        Some(r#"{"provider": "claude", "label": ""}"#),
    );
    assert_eq!(status, 400);
    assert!(f.sign_ins.lock().expect("lock").is_empty());
    server.stop();
}

#[test]
fn identity_accept_needs_a_change_and_the_accept_action() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let (main, _) = two_accounts(&f, addr);
    let path = format!("/api/accounts/{main}/identity");
    assert_eq!(
        refused(addr, "POST", &path, Some(r#"{"action": "reject"}"#)),
        (400, "The only identity action is accept.".to_owned())
    );
    assert_eq!(
        refused(addr, "POST", &path, Some(r#"{"action": "accept"}"#)),
        (400, "This account's identity has not changed.".to_owned())
    );
    server.stop();
}
