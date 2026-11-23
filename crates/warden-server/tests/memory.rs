//! Memory over time on this computer's real processes: a stand-in session's CLI tree holding a known amount, found
//! under its session within two samples and as "Left running" once its CLI exits; and `GET /api/memory/day`.

mod common;

use common::{
    Fixture, MEMORY_EVERY, StandIn, StandInSpec, call, eventually, memory_now, memory_today,
};

/// The part the copies of this binary play in a stand-in tree.
#[test]
fn memory_helper() {
    common::memory_helper();
}

const HELD_MIB: u64 = 64;

#[test]
fn a_sessions_tree_is_measured_and_what_it_leaves_running_too() {
    let f = Fixture::new();
    let claude = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let server = f.start();
    let addr = server.addr;
    let body =
        serde_json::json!({ "provider": "claude", "folder": claude, "label": "Work" }).to_string();
    assert_eq!(call(addr, "POST", "/api/accounts", Some(&body)).status, 200);
    let id = call(addr, "GET", "/api/state", None).json()["accounts"][0]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let mut tree = StandIn::start(&StandInSpec {
        folder: &f.home.path().join("stand-in"),
        data_dir: f.data.path(),
        account: &id,
        session_id: "s1",
        cwd: f.home.path(),
        mib: HELD_MIB,
        extra_arg: "",
        extra_env: "",
        transcript: None,
    });
    // Within two samples (and the time the event takes), the session counts the helper's memory.
    let found = std::time::Instant::now();
    let mut day = serde_json::Value::Null;
    eventually("the session's memory", || {
        day = memory_today(addr);
        memory_now(&day, "session", "s1").is_some_and(|kib| kib >= HELD_MIB * 1024)
    });
    assert!(found.elapsed() < MEMORY_EVERY * 2 + std::time::Duration::from_secs(5));
    let series = day["series"]
        .as_array()
        .expect("series")
        .iter()
        .find(|s| s["sessionId"] == "s1")
        .expect("the session's series")
        .clone();
    assert_eq!(series["accountId"], id.as_str());
    assert_eq!(series["account"], "Work");
    assert_eq!(series["provider"], "claude");
    assert!(day["today"].as_bool().expect("today"));
    assert!(day["summary"]["computerTotal"].as_u64().expect("total") > 0);
    assert!(day["summary"]["peak"].as_u64().expect("peak") >= HELD_MIB * 1024);
    // Warden itself is measured beside it, apart from the sessions' total.
    assert!(
        day["series"]
            .as_array()
            .expect("series")
            .iter()
            .any(|s| s["kind"] == "warden")
    );

    // The CLI exits; the helper it started keeps running: it is left running, for the same session.
    tree.end_cli();
    eventually("the helper counted as left running", || {
        let day = memory_today(addr);
        memory_now(&day, "left", "s1").is_some_and(|kib| kib >= HELD_MIB * 1024)
    });
    // (Once it exits it is forgotten; a minute keeps its highest value, so this minute still shows it.)
    tree.end_helper();
    server.stop();
    // The day file has the series and its minutes.
    let today = warden_core::memory::today(warden_core::time::now_ms());
    let saved = warden_core::memory::read_day(&f.data.path().join("memory"), &today);
    assert!(
        saved
            .series
            .iter()
            .any(|s| s.info.session_id.as_deref() == Some("s1"))
    );
    assert!(!saved.minutes.is_empty());
}

#[test]
fn a_day_is_asked_for_by_its_date() {
    let f = Fixture::new();
    // A past day already on disk, and one Warden would drop (older than 30 days).
    let mem = f.data.path().join("memory");
    let now = warden_core::time::now_ms();
    let today = warden_core::memory::today(now);
    let today_n = warden_core::time::parse_date(&today).expect("today");
    let past = warden_core::time::date_text(today_n - 2);
    let old = warden_core::time::date_text(today_n - 40);
    for date in [&past, &old] {
        common::write(
            &warden_core::memory::day_file(&mem, date),
            concat!(
                "{\"series\":1,\"kind\":\"session\",\"provider\":\"claude\",\"accountId\":\"main\",\"account\":\"Main\",\"sessionId\":\"p1\",\"parentSessionId\":null,\"name\":\"Fix the login page\",\"project\":\"Warden\"}\n",
                "{\"m\":600,\"total\":1000,\"used\":500,\"sum\":200,\"s\":[[1,200]]}\n",
                "{\"m\":603,\"total\":1000,\"used\":500,\"sum\":300,\"s\":[[1,300]]}\n",
            ),
        );
    }
    let server = f.start();
    let addr = server.addr;
    assert!(!warden_core::memory::day_file(&mem, &old).exists());

    let reply = call(addr, "GET", &format!("/api/memory/day?date={past}"), None);
    assert_eq!(reply.status, 200);
    let day = reply.json();
    assert_eq!(day["date"], past.as_str());
    assert_eq!(day["today"], false);
    assert_eq!(day["series"][0]["name"], "Fix the login page");
    assert_eq!(day["minutes"].as_array().expect("minutes").len(), 2);
    assert_eq!(day["gaps"], serde_json::json!([{ "from": 601, "to": 603 }]));
    assert_eq!(day["summary"]["peak"], 300);
    assert_eq!(day["summary"]["peakAt"], 603);
    assert_eq!(day["summary"]["peakShare"], 30.0);
    assert_eq!(day["summary"]["now"], serde_json::Value::Null);
    // Today shows up among the days once it has a sample.
    eventually("today's first sample", || {
        memory_today(addr)["days"]
            .as_array()
            .is_some_and(|d| d.contains(&serde_json::json!(today)))
    });
    let days = memory_today(addr)["days"].clone();
    assert_eq!(days, serde_json::json!([past, today]));

    // A day with nothing is empty.
    let empty = warden_core::time::date_text(today_n - 1);
    let day = call(addr, "GET", &format!("/api/memory/day?date={empty}"), None).json();
    assert_eq!(day["minutes"], serde_json::json!([]));
    assert_eq!(day["series"], serde_json::json!([]));
    assert_eq!(day["summary"]["peak"], serde_json::Value::Null);

    // A date to come, or not a date, is refused.
    let tomorrow = warden_core::time::date_text(today_n + 1);
    for bad in [
        format!("date={tomorrow}"),
        "date=2026-02-30".into(),
        "date=yesterday".into(),
        "date=2026-11-1".into(),
        String::new(),
    ] {
        let reply = call(addr, "GET", &format!("/api/memory/day?{bad}"), None);
        assert_eq!(reply.status, 400, "{bad}: {}", reply.body);
    }
    // The memory day is not part of the state: no revision moves with a sample.
    let revision = call(addr, "GET", "/api/state", None).json()["revision"].clone();
    std::thread::sleep(MEMORY_EVERY * 3);
    assert_eq!(
        call(addr, "GET", "/api/state", None).json()["revision"],
        revision
    );
    server.stop();
}

#[test]
fn a_linked_memory_folder_is_left_alone() {
    let f = Fixture::new();
    // The memory folder is a junction to a folder outside the data folder, holding an old day Warden would prune.
    let outside = warden_core::testdir::Dir::new("memory-outside");
    let today = warden_core::memory::today(warden_core::time::now_ms());
    let today_n = warden_core::time::parse_date(&today).expect("today");
    let old =
        warden_core::memory::day_file(outside.path(), &warden_core::time::date_text(today_n - 40));
    common::write(
        &old,
        "{\"m\":1,\"total\":1,\"used\":1,\"sum\":1,\"s\":[]}\n",
    );
    std::fs::create_dir_all(f.data.path()).expect("data");
    junction::create(outside.path(), f.data.path().join("memory")).expect("junction");
    let before = common::folder_bytes(outside.path());

    let server = f.start();
    let addr = server.addr;
    // Samples go on (and are answered from memory); nothing is read, written or removed through the link.
    eventually("today's first sample", || {
        memory_today(addr)["minutes"]
            .as_array()
            .is_some_and(|m| !m.is_empty())
    });
    let past = warden_core::time::date_text(today_n - 40);
    let day = call(addr, "GET", &format!("/api/memory/day?date={past}"), None).json();
    assert_eq!(day["minutes"], serde_json::json!([]));
    assert_eq!(day["days"], serde_json::json!([today]));
    server.stop();
    assert_eq!(common::folder_bytes(outside.path()), before);
    assert!(f.log_text().contains("could not write the memory history"));
}
