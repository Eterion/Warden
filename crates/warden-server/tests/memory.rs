//! Memory over time on this computer's real processes: a stand-in session's CLI tree holding a known amount, found
//! under its session within two samples and as "Left running" once its CLI exits; and `GET /api/memory/day`.

mod common;

use common::{
    Fixture, MEMORY_EVERY, StandIn, StandInSpec, call, eventually, memory_now, memory_today,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use warden_core::{memory, model::Provider};
use warden_server::{app::App, procs::MemorySource};

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
    // When the helper exits it stops counting. A minute keeps its highest value, so this minute still shows it.
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
fn a_left_running_process_still_counts_after_a_restart() {
    #[derive(Default)]
    struct MadeUpMemory(AtomicUsize);

    impl MemorySource for MadeUpMemory {
        fn computer(&self) -> Option<(u64, u64)> {
            Some((128 * 1024, 64 * 1024))
        }

        fn stand_in_cli(&self, _provider: Provider, _id: &str) -> Option<u32> {
            Some(u32::MAX - 1)
        }

        fn processes(
            &self,
            _roots: &[u32],
            _own: u32,
            pick: &mut dyn FnMut(&[memory::Proc]) -> Vec<u32>,
        ) -> Vec<memory::Proc> {
            let stage = self.0.load(Ordering::Relaxed) as u64;
            let cli = u32::MAX - 1;
            let mut processes = vec![memory::Proc {
                pid: u32::MAX - 2,
                parent: Some(cli),
                started: 2,
                private: Some((8 + stage * 8) * 1024),
            }];
            if stage == 0 {
                processes.push(memory::Proc {
                    pid: cli,
                    parent: None,
                    started: 1,
                    private: Some(0),
                });
            }
            let chosen = pick(&processes);
            processes
                .iter_mut()
                .filter(|p| !chosen.contains(&p.pid))
                .for_each(|p| p.private = None);
            processes
        }

        fn every(&self) -> std::time::Duration {
            MEMORY_EVERY
        }
    }

    let f = Fixture::new();
    let source = Arc::new(MadeUpMemory::default());
    let server = f.start_with_memory(None, source.clone());
    let claude = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let id = server
        .app
        .add_existing(Provider::Claude, claude.to_str().expect("folder"), "Work")
        .expect("account");
    let event = |kind| {
        let json = serde_json::json!({ "session_id": "s1", "hook_event_name": kind });
        warden_core::sessions::parse_event(json.to_string().as_bytes()).expect("event")
    };
    server
        .app
        .receive_event(&id, &event("SessionStart"))
        .expect("start session");
    eventually("the session measured", || {
        memory_now(&memory_today(server.addr), "session", "s1") == Some(8)
    });
    assert!(!f.log_text().contains("could not load saved memory links"));
    server
        .app
        .receive_event(&id, &event("SessionEnd"))
        .expect("end session");
    source.0.store(1, Ordering::Relaxed);
    eventually("the process left running", || {
        memory_now(&memory_today(server.addr), "left", "s1") == Some(16)
    });
    server.stop();
    let links = f.data.path().join(memory::DIR).join(memory::LINK_FILE);
    assert!(links.is_file());

    source.0.store(2, Ordering::Relaxed);
    let server = f.start_with_memory(None, source);
    eventually("the restored process measured with its new value", || {
        let day = memory_today(server.addr);
        if memory_now(&day, "left", "s1") != Some(24) {
            return false;
        }
        assert_eq!(day["days"], serde_json::json!([day["date"]]));
        assert!(day["series"].as_array().expect("series").iter().any(|s| {
            s["kind"] == "left"
                && s["sessionId"] == "s1"
                && s["accountId"] == id
                && s["account"] == "Work"
        }));
        true
    });
    assert!(links.is_file());
    server.stop();
}

#[derive(Default)]
struct EmptySnapshot(AtomicUsize);

impl MemorySource for EmptySnapshot {
    fn computer(&self) -> Option<(u64, u64)> {
        Some((1024, 512))
    }

    fn processes(
        &self,
        _roots: &[u32],
        _own: u32,
        pick: &mut dyn FnMut(&[memory::Proc]) -> Vec<u32>,
    ) -> Vec<memory::Proc> {
        self.0.fetch_add(1, Ordering::Relaxed);
        pick(&[]);
        Vec::new()
    }
}

#[test]
fn exited_records_alone_do_not_keep_process_snapshots_running() {
    let f = Fixture::new();
    let app = App::new(
        f.data.path().to_owned(),
        f.env(),
        (f.endpoint, f.sign_in_server, f.resets),
        Arc::new(|_, _, _, _| Ok(())),
        Arc::new(|_| {}),
        None,
        None,
    )
    .expect("app");
    let mut left = memory::LeftRunning::default();
    let session = memory::Live {
        provider: Provider::Claude,
        session_id: "s1".into(),
        parent_session_id: None,
        account_id: "main".into(),
        account_label: None,
        other_sign_in: false,
        name: None,
        project: None,
        cli: 10,
        last_event_at: 0,
    };
    let cli = memory::Proc {
        pid: 10,
        parent: None,
        started: 1,
        private: None,
    };
    left.update(&memory::plan(&[cli], &[session], 1, &left, 2_000));
    let source = EmptySnapshot::default();

    // The process was running, so take the snapshot that finds it gone.
    let written = app.sample_memory(&source, &mut left);
    written.day.expect("first day sample");
    written.links.expect("first links sample");
    assert_eq!(source.0.load(Ordering::Relaxed), 1);
    assert!(!left.any_running());

    // There is only a missing-parent record. The next sample reads the computer without a snapshot.
    let written = app.sample_memory(&source, &mut left);
    written.day.expect("idle day sample");
    written.links.expect("idle links sample");
    assert_eq!(source.0.load(Ordering::Relaxed), 1);
}

#[test]
fn a_day_is_asked_for_by_its_date() {
    let f = Fixture::new();
    // A past day already on disk, and one Warden would drop (older than 30 days).
    let mem = f.data.path().join("memory");
    let bad_links = mem.join(memory::LINK_FILE);
    common::write(&bad_links, "marker from a malformed file");
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
    assert!(!bad_links.exists());
    let log = f.log_text();
    assert_eq!(log.matches("could not load saved memory links").count(), 1);
    assert!(!log.contains("marker from a malformed file"));
}

#[test]
fn todays_memory_is_followed_through_its_wait() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    // Without a sample number it answers at once, with the sample it holds.
    let first = call(addr, "GET", "/api/memory/today/wait?after=0", None);
    assert_eq!(first.status, 200, "{}", first.body);
    let first = first.json();
    let today = warden_core::memory::today(warden_core::time::now_ms());
    assert_eq!(first["date"], today.as_str());
    assert_eq!(first["today"], true);
    let sample = first["sample"].as_u64().expect("sample");

    // With it, it waits for the next sample (one every MEMORY_EVERY here), and answers with that one.
    let asked = std::time::Instant::now();
    let next = call(
        addr,
        "GET",
        &format!("/api/memory/today/wait?after=0&sample={sample}"),
        None,
    )
    .json();
    assert!(next["sample"].as_u64().expect("sample") > sample);
    assert!(asked.elapsed() < MEMORY_EVERY * 4);
    let minutes = next["minutes"].as_array().expect("minutes");
    assert!(!minutes.is_empty(), "today has its first minute");
    let last = minutes.last().expect("a minute")["m"].as_u64().expect("m");
    assert!(next["summary"]["computerTotal"].as_u64().expect("total") > 0);

    // Only the minutes from `after` on come back, and the minute being gathered (still `last` here, or a later one);
    // the figures stay the whole day's.
    let later = call(
        addr,
        "GET",
        &format!("/api/memory/today/wait?after={}", last + 1),
        None,
    )
    .json();
    let ms: Vec<u64> = later["minutes"]
        .as_array()
        .expect("minutes")
        .iter()
        .map(|m| m["m"].as_u64().expect("m"))
        .collect();
    assert!(ms.iter().all(|&m| m >= last), "{ms:?}");
    assert!(ms.iter().filter(|&&m| m == last).count() <= 1, "{ms:?}");
    assert!(!ms.is_empty(), "the minute being gathered comes");
    assert_eq!(later["summary"]["first"], next["summary"]["first"]);
    assert!(
        later["days"]
            .as_array()
            .expect("days")
            .contains(&serde_json::json!(today))
    );

    // A sample number it does not hold (the server started again since) is answered at once too.
    let asked = std::time::Instant::now();
    let other = later["sample"].as_u64().expect("sample") + 1_000;
    let reply = call(
        addr,
        "GET",
        &format!("/api/memory/today/wait?after=0&sample={other}"),
        None,
    );
    assert_eq!(reply.status, 200);
    assert!(asked.elapsed() < MEMORY_EVERY);
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
    common::write(
        &outside.path().join(memory::LINK_FILE),
        r#"{"version":1,"sampledAtMs":2000,"series":[{"kind":"left","provider":"claude","sessionId":"s1"}],"processes":[[10,1,0]]}"#,
    );
    common::write(
        &outside.path().join("linked.json.12-3.tmp"),
        "interrupted write",
    );
    std::fs::create_dir_all(f.data.path()).expect("data");
    junction::create(outside.path(), f.data.path().join("memory")).expect("junction");
    let before = common::folder_bytes(outside.path());

    let source = Arc::new(EmptySnapshot::default());
    let server = f.start_with_memory(None, source.clone());
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
    assert_eq!(
        source.0.load(Ordering::Relaxed),
        0,
        "the saved links were not loaded"
    );
    assert_eq!(common::folder_bytes(outside.path()), before);
    let log = f.log_text();
    assert_eq!(log.matches("could not load saved memory links").count(), 1);
    assert!(log.contains("could not write the memory history"));
    assert!(log.contains("could not write the saved memory links"));
}
