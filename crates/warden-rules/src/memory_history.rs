//! Rule: the memory history files keep only what is allowed, and stay within their caps.
//!
//! A day file (`memory/memory-<YYYY-MM-DD>.jsonl` in the data folder) holds series lines and minute lines only:
//! memory figures, account ids and labels, session ids, and session and project names. Never a prompt, token,
//! command line, environment, path or process name. Pass a day file's text to [`assert_only_allowed`] (and the
//! canaries planted where those would be read to `hook_data::assert_not_kept`), and the memory folder to
//! [`assert_capped`]. Check `memory/linked.json` with [`assert_linked_only_allowed`] and [`assert_linked_capped`]:
//! it adds only a version, a snapshot timestamp and numeric process identities referencing the same series fields.

use serde_json::{Map, Value};
use std::path::Path;

/// The fields a series line may have.
pub const SERIES_FIELDS: &[&str] = &[
    "series",
    "kind",
    "provider",
    "accountId",
    "account",
    "sessionId",
    "parentSessionId",
    "name",
    "project",
];
/// The fields a minute line may have: all numbers, and `s` a list of `[series, KiB]` pairs.
pub const MINUTE_FIELDS: &[&str] = &["m", "total", "used", "sum", "s"];
const KINDS: &[&str] = &["session", "left", "other-sign-in", "warden"];
/// Longest a kept text may be.
const MAX_TEXT: usize = 64;

fn fail(place: &str, why: &str) -> ! {
    panic!("rule broken: the memory history in {place} {why}")
}

/// A text that reads like a path or a program: a drive, a folder separator or an `.exe`.
fn looks_like_a_path(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('\\')
        || text.contains(":/")
        || text.starts_with('/')
        || lower.contains(".exe")
        || lower.contains("%userprofile%")
}

/// Panics unless every line of `text` is a series line or a minute line with only the allowed fields: numbers in a
/// minute line, short texts that are not paths in a series line, a known kind.
pub fn assert_only_allowed(place: &str, text: &str) {
    for line in text.lines() {
        let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(line) else {
            fail(place, "has a line that is not a JSON object");
        };
        if fields.contains_key("m") {
            for (key, value) in &fields {
                if !MINUTE_FIELDS.contains(&key.as_str()) {
                    fail(
                        place,
                        &format!("keeps a field not allowed in a minute: {key}"),
                    );
                }
                let numbers = match value {
                    Value::Number(_) => true,
                    Value::Array(pairs) => pairs.iter().all(|p| {
                        p.as_array()
                            .is_some_and(|p| p.len() == 2 && p.iter().all(Value::is_u64))
                    }),
                    _ => false,
                };
                if !numbers {
                    fail(
                        place,
                        &format!("keeps something other than numbers in a minute's {key}"),
                    );
                }
            }
        } else if fields.contains_key("series") {
            assert_series(place, &fields, true);
        } else {
            fail(place, "has a line that is neither a series nor a minute");
        }
    }
}

fn assert_series(place: &str, fields: &Map<String, Value>, numbered: bool) {
    for (key, value) in fields {
        if !SERIES_FIELDS.contains(&key.as_str()) || (!numbered && key == "series") {
            fail(
                place,
                &format!("keeps a field not allowed in a series: {key}"),
            );
        }
        match (key.as_str(), value) {
            ("series", Value::Number(_)) | (_, Value::Null) => {}
            ("kind", Value::String(k)) if KINDS.contains(&k.as_str()) => {}
            ("kind", _) => fail(place, "has a series of an unknown kind"),
            (_, Value::String(t)) => {
                if t.chars().count() > MAX_TEXT {
                    fail(
                        place,
                        &format!("keeps a {key} longer than {MAX_TEXT} characters"),
                    );
                }
                if looks_like_a_path(t) {
                    fail(
                        place,
                        &format!("keeps a path or program in a series' {key}"),
                    );
                }
            }
            _ => fail(
                place,
                &format!("keeps something other than a text in a series' {key}"),
            ),
        }
    }
}

fn linked_file(place: &str, text: &str) -> Map<String, Value> {
    let Ok(Value::Object(fields)) = serde_json::from_str(text) else {
        fail(place, "is not a JSON object");
    };
    fields
}

/// Panics unless the process link file contains only its numeric fields and allowed series information.
pub fn assert_linked_only_allowed(place: &str, text: &str) {
    let fields = linked_file(place, text);
    if fields.len() != 4
        || !fields.get("version").is_some_and(Value::is_u64)
        || !fields.get("sampledAtMs").is_some_and(Value::is_u64)
    {
        fail(place, "has unexpected process link fields");
    }
    let Some(series) = fields.get("series").and_then(Value::as_array) else {
        fail(place, "has no list of linked series");
    };
    for line in series {
        let Some(info) = line.as_object() else {
            fail(place, "has a linked series that is not an object");
        };
        assert_series(place, info, false);
    }
    let Some(processes) = fields.get("processes").and_then(Value::as_array) else {
        fail(place, "has no list of linked processes");
    };
    for p in processes {
        if !p.as_array().is_some_and(|p| {
            p.len() == 3
                && p.iter().all(Value::is_u64)
                && p[2].as_u64().is_some_and(|n| n < series.len() as u64)
        }) {
            fail(
                place,
                "has a process link other than [pid, started, series]",
            );
        }
    }
}

/// Panics if the process link file exceeds its own byte or entry cap.
pub fn assert_linked_capped(place: &str, text: &str, max_bytes: u64, max_entries: usize) {
    if text.len() as u64 > max_bytes {
        fail(place, &format!("holds more than {max_bytes} bytes"));
    }
    let fields = linked_file(place, text);
    for key in ["processes", "series"] {
        let entries = fields.get(key).and_then(Value::as_array).expect(key);
        if entries.len() > max_entries {
            fail(place, &format!("holds more than {max_entries} {key}"));
        }
    }
}

/// The caps a memory folder must keep.
pub struct Caps<'a> {
    /// The oldest day that may still be there (`YYYY-MM-DD`).
    pub oldest: &'a str,
    pub max_total_bytes: u64,
    pub max_series: usize,
    pub max_minutes: usize,
}

/// Panics if the memory folder holds a day older than `oldest`, more than its total, or a day with more series or
/// minute lines than allowed.
pub fn assert_capped(dir: &Path, caps: &Caps) {
    let mut total = 0;
    for entry in std::fs::read_dir(dir).expect("the memory folder").flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = name
            .strip_prefix("memory-")
            .and_then(|n| n.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if date < caps.oldest {
            fail(
                &name,
                &format!("is kept past its days (older than {})", caps.oldest),
            );
        }
        let text = std::fs::read_to_string(entry.path()).expect("a day file");
        total += text.len() as u64;
        let mut series = std::collections::HashSet::new();
        let mut minutes = 0;
        for line in text.lines() {
            if let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(line) {
                if let Some(n) = fields.get("series").and_then(Value::as_u64) {
                    series.insert(n);
                } else if fields.contains_key("m") {
                    minutes += 1;
                }
            }
        }
        if series.len() > caps.max_series || series.iter().any(|n| *n as usize > caps.max_series) {
            fail(
                &name,
                &format!("holds more than {} series", caps.max_series),
            );
        }
        if minutes > caps.max_minutes {
            fail(
                &name,
                &format!("holds more than {} minutes", caps.max_minutes),
            );
        }
    }
    if total > caps.max_total_bytes {
        fail(
            "the memory folder",
            &format!("holds {total} bytes, more than {}", caps.max_total_bytes),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIES: &str = r#"{"series":1,"kind":"session","provider":"claude","accountId":"a1","account":"Work","sessionId":"s1","parentSessionId":null,"name":"Fix the login page","project":"Warden"}"#;
    const MINUTE: &str =
        r#"{"m":615,"total":33554432,"used":18432000,"sum":3355443,"s":[[1,812345],[2,120000]]}"#;

    #[test]
    fn a_clean_day_passes() {
        assert_only_allowed("day", &format!("{SERIES}\n{MINUTE}\n"));
    }

    fn linked_text(series: &str) -> String {
        let info = series
            .replace(r#""series":1,"#, "")
            .replace("session\"", "left\"");
        format!(r#"{{"version":1,"sampledAtMs":6000,"series":[{info}],"processes":[[10,5,0]]}}"#)
    }

    #[test]
    fn clean_process_links_pass() {
        let text = linked_text(SERIES);
        assert_linked_only_allowed("linked.json", &text);
        assert_linked_capped("linked.json", &text, 600, 1);
    }

    #[test]
    #[should_panic(expected = "keeps a field not allowed in a series: commandLine")]
    fn a_command_line_in_linked_series_is_caught() {
        let text = linked_text(&SERIES.replace(
            r#""project":"Warden""#,
            r#""project":"Warden","commandLine":"claude -p secret""#,
        ));
        assert_linked_only_allowed("linked.json", &text);
    }

    #[test]
    #[should_panic(expected = "has a process link other than [pid, started, series]")]
    fn an_extra_value_in_a_process_link_is_caught() {
        let text = linked_text(SERIES).replace("[10,5,0]", r#"[10,5,0,"node.exe"]"#);
        assert_linked_only_allowed("linked.json", &text);
    }

    #[test]
    #[should_panic(expected = "has a process link other than [pid, started, series]")]
    fn an_out_of_range_process_link_index_is_caught() {
        let text = linked_text(SERIES).replace("[10,5,0]", "[10,5,1]");
        assert_linked_only_allowed("linked.json", &text);
    }

    #[test]
    #[should_panic(expected = "has unexpected process link fields")]
    fn an_extra_top_level_link_field_is_caught() {
        let text = linked_text(SERIES)
            .replace(r#""version":1"#, r#""version":1,"commandLine":"node.exe""#);
        assert_linked_only_allowed("linked.json", &text);
    }

    #[test]
    #[should_panic(expected = "keeps a field not allowed in a series: series")]
    fn a_series_number_in_a_linked_series_is_caught() {
        let text = linked_text(SERIES).replace(r#""kind":"left""#, r#""kind":"left","series":1"#);
        assert_linked_only_allowed("linked.json", &text);
    }

    #[test]
    #[should_panic(expected = "holds more than 1 processes")]
    fn too_many_process_links_are_caught() {
        let text = linked_text(SERIES).replace("[10,5,0]", "[10,5,0],[11,5,0]");
        assert_linked_capped("linked.json", &text, 600, 1);
    }

    #[test]
    #[should_panic(expected = "rule broken: the memory history")]
    fn a_command_line_field_is_caught() {
        let line = SERIES.replace(
            r#""project":"Warden""#,
            r#""project":"Warden","cmd":"claude -p secret""#,
        );
        assert_only_allowed("day", &line);
    }

    #[test]
    #[should_panic(expected = "keeps a path or program")]
    fn a_path_in_a_name_is_caught() {
        let line = SERIES.replace("Fix the login page", r"%USERPROFILE%\\work");
        assert_only_allowed("day", &line);
    }

    #[test]
    #[should_panic(expected = "keeps a path or program")]
    fn a_process_name_is_caught() {
        assert_only_allowed("day", &SERIES.replace("Warden", "node.exe"));
    }

    #[test]
    #[should_panic(expected = "something other than numbers")]
    fn text_in_a_minute_is_caught() {
        assert_only_allowed("day", &MINUTE.replace("[2,120000]", r#"[2,"claude.exe"]"#));
    }

    #[test]
    #[should_panic(expected = "neither a series nor a minute")]
    fn another_kind_of_line_is_caught() {
        assert_only_allowed("day", r#"{"prompt":"hello"}"#);
    }

    #[test]
    #[should_panic(expected = "longer than")]
    fn a_long_name_is_caught() {
        assert_only_allowed(
            "day",
            &SERIES.replace("Fix the login page", &"x".repeat(65)),
        );
    }

    fn folder(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("warden-rules-memory-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    const CAPS: Caps = Caps {
        oldest: "2026-10-25",
        max_total_bytes: 600,
        max_series: 2,
        max_minutes: 2,
    };

    #[test]
    fn a_folder_within_its_caps_passes() {
        let dir = folder("ok");
        std::fs::write(
            dir.join("memory-2026-11-23.jsonl"),
            format!("{SERIES}\n{MINUTE}\n"),
        )
        .expect("write");
        assert_capped(&dir, &CAPS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[should_panic(expected = "is kept past its days")]
    fn an_old_day_is_caught() {
        let dir = folder("old");
        std::fs::write(dir.join("memory-2026-10-24.jsonl"), MINUTE).expect("write");
        assert_capped(&dir, &CAPS);
    }

    #[test]
    #[should_panic(expected = "more than 2 minutes")]
    fn too_many_minutes_are_caught() {
        let dir = folder("minutes");
        std::fs::write(
            dir.join("memory-2026-11-23.jsonl"),
            format!("{MINUTE}\n{MINUTE}\n{MINUTE}\n"),
        )
        .expect("write");
        assert_capped(&dir, &CAPS);
    }

    #[test]
    #[should_panic(expected = "more than 2 series")]
    fn too_many_series_are_caught() {
        let dir = folder("series");
        let third = SERIES.replace(r#""series":1"#, r#""series":3"#);
        std::fs::write(dir.join("memory-2026-11-23.jsonl"), third).expect("write");
        assert_capped(&dir, &CAPS);
    }

    #[test]
    #[should_panic(expected = "more than 600")]
    fn a_folder_over_its_total_is_caught() {
        let dir = folder("total");
        for day in ["2026-11-21", "2026-11-22", "2026-11-23"] {
            std::fs::write(
                dir.join(format!("memory-{day}.jsonl")),
                format!("{SERIES}\n{MINUTE}\n"),
            )
            .expect("write");
        }
        assert_capped(&dir, &CAPS);
    }
}
