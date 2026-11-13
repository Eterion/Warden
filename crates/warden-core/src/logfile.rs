//! Warden's own small rolling log, in the data folder's `logs` folder: one file a day (`warden-YYYY-MM-DD.log`,
//! dated in local time like its lines, which also carry the UTC offset; a new file at local midnight), the last few
//! days kept, and capped in total. It holds only what the server and the
//! desktop app write to their log: never a token, prompt, tool input or message (the rule tests read it).

use crate::time::now_ms;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The log folder inside the data folder.
pub const LOG_DIR: &str = "logs";
/// Days of files kept, today's included.
pub const KEEP_DAYS: usize = 5;
/// Most one day's file may hold; past it, the day's further lines are dropped (after one line saying so).
pub const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Most all files may hold together: older files go first when a new day starts.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024;

const PREFIX: &str = "warden-";
const DAY_MS: i64 = 86_400_000;
/// Room kept at the end of a day's file for the line saying it is full.
const FULL_NOTE_ROOM: u64 = 100;
const SUFFIX: &str = ".log";

/// The log folder of a data folder.
pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(LOG_DIR)
}

/// A rolling log in one folder. Writing never fails the caller: a line that cannot be written is dropped.
pub struct RollingLog {
    dir: PathBuf,
    open: Mutex<Option<Day>>,
}

struct Day {
    date: String,
    file: File,
    bytes: u64,
    full: bool,
}

impl RollingLog {
    pub fn new(dir: PathBuf) -> RollingLog {
        RollingLog {
            dir,
            open: Mutex::new(None),
        }
    }

    /// Appends one line, stamped with the local time and its UTC offset.
    pub fn write(&self, line: &str) {
        self.write_at(now_ms(), line);
    }

    pub fn write_at(&self, at_ms: i64, line: &str) {
        self.write_in(at_ms, local_offset_s(at_ms), line);
    }

    /// Appends one line at a time in a time zone `offset_s` seconds ahead of UTC.
    fn write_in(&self, at_ms: i64, offset_s: i32, line: &str) {
        let Ok(mut open) = self.open.lock() else {
            return;
        };
        let (date, time) = stamp(at_ms, offset_s);
        if open.as_ref().is_none_or(|d| d.date != date) {
            *open = self.open_day(&date, at_ms, offset_s).ok();
        }
        let Some(day) = open.as_mut() else { return };
        if day.full {
            return;
        }
        // One line per write, whatever the text holds.
        let text = format!("{date} {time} {}\n", line.replace(['\r', '\n'], " "));
        let len = text.len() as u64;
        if day.bytes + len > MAX_FILE_BYTES - FULL_NOTE_ROOM {
            day.full = true;
            // The note goes in the room kept for it, so the file stays within the cap.
            let note = format!("{date} {time} log full for today; later lines are dropped\n");
            if day.bytes + note.len() as u64 <= MAX_FILE_BYTES {
                let _ = day.file.write_all(note.as_bytes());
            }
            return;
        }
        if day.file.write_all(text.as_bytes()).is_ok() {
            day.bytes += len;
        }
    }

    /// Opens (or creates) the day's file, after pruning the older ones.
    fn open_day(&self, date: &str, at_ms: i64, offset_s: i32) -> io::Result<Day> {
        fs::create_dir_all(&self.dir)?;
        let name = format!("{PREFIX}{date}{SUFFIX}");
        let oldest = stamp(at_ms - (KEEP_DAYS as i64 - 1) * DAY_MS, offset_s).0;
        prune(&self.dir, &name, &oldest);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(&name))?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Day {
            date: date.to_owned(),
            file,
            bytes,
            full: bytes >= MAX_FILE_BYTES - FULL_NOTE_ROOM,
        })
    }
}

/// Removes the files dated before `oldest` (only the last `KEEP_DAYS` days are kept, today's counted, however long
/// Warden was not running), all but the newest `KEEP_DAYS`, then the oldest until the others leave room for a full
/// day's file within the total cap. Only Warden's own `warden-YYYY-MM-DD.log` files are touched.
fn prune(dir: &Path, today: &str, oldest: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(String, u64, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let date = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
            (date.len() == 10 && name.as_str() != today).then_some(())?;
            let old = date < oldest;
            Some((name, e.metadata().ok()?.len(), old))
        })
        .collect();
    // Newest first (the dates sort as text).
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = 0;
    for (i, (name, len, old)) in files.iter().enumerate() {
        total += len;
        if *old || i + 1 >= KEEP_DAYS || total + MAX_FILE_BYTES > MAX_TOTAL_BYTES {
            let _ = fs::remove_file(dir.join(name));
        }
    }
}

/// How far this computer's time zone is ahead of UTC at a moment, in seconds (daylight saving included); UTC when
/// it cannot be told.
fn local_offset_s(at_ms: i64) -> i32 {
    time::OffsetDateTime::from_unix_timestamp(at_ms.div_euclid(1000))
        .ok()
        .and_then(|t| time::UtcOffset::local_offset_at(t).ok())
        .map_or(0, time::UtcOffset::whole_seconds)
}

/// `("2026-11-10", "10:05:03 +01:00")` for a time in milliseconds since the epoch, in a time zone `offset_s` seconds
/// ahead of UTC.
fn stamp(at_ms: i64, offset_s: i32) -> (String, String) {
    let local = at_ms + i64::from(offset_s) * 1000;
    let days = local.div_euclid(86_400_000);
    let ms = local.rem_euclid(86_400_000);
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    let sign = if offset_s < 0 { '-' } else { '+' };
    let off = offset_s.unsigned_abs() / 60;
    let (oh, om) = (off / 60, off % 60);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{h:02}:{m:02}:{s:02} {sign}{oh:02}:{om:02}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;

    const DAY: i64 = 86_400_000;
    // 2026-11-10T09:05:03.250Z
    const AT: i64 = 1_794_301_503_250;
    const CET: i32 = 3600;

    #[test]
    fn stamps_are_local_dates_and_times_with_the_offset() {
        assert_eq!(stamp(0, 0), ("1970-01-01".into(), "00:00:00 +00:00".into()));
        assert_eq!(
            stamp(AT, CET),
            ("2026-11-10".into(), "10:05:03 +01:00".into())
        );
        assert_eq!(
            stamp(AT, -(5 * 3600 + 30 * 60)),
            ("2026-11-10".into(), "03:35:03 -05:30".into())
        );
        assert_eq!(stamp(951_782_400_000, 0).0, "2000-02-29");
        // Late in the UTC day is already the next day east of it, and the day before west of it.
        assert_eq!(stamp(AT + 14 * 3_600_000, CET).0, "2026-11-11");
        assert_eq!(stamp(AT, -10 * 3600).0, "2026-11-09");
    }

    #[test]
    fn the_local_offset_is_whole_minutes_within_a_day() {
        let off = local_offset_s(AT);
        assert_eq!(off % 60, 0);
        assert!(off.abs() <= 14 * 3600, "{off}");
    }

    #[test]
    fn one_file_a_day_one_line_per_write() {
        let dir = Dir::new("logfile");
        let log = RollingLog::new(dir.path().join("logs"));
        log.write_in(AT, CET, "Work: usage read");
        log.write_in(AT + 1000, CET, "two\nlines");
        log.write_in(AT + DAY, CET, "next day");
        let first =
            fs::read_to_string(dir.path().join("logs/warden-2026-11-10.log")).expect("first day");
        assert_eq!(
            first,
            "2026-11-10 10:05:03 +01:00 Work: usage read\n2026-11-10 10:05:04 +01:00 two lines\n"
        );
        let second =
            fs::read_to_string(dir.path().join("logs/warden-2026-11-11.log")).expect("next day");
        assert_eq!(second, "2026-11-11 10:05:03 +01:00 next day\n");
    }

    #[test]
    fn the_day_rolls_over_at_local_midnight() {
        let dir = Dir::new("logfile");
        let logs = dir.path().join("logs");
        let log = RollingLog::new(logs.clone());
        // 2026-11-10 23:00:00 UTC is midnight at +01:00: the 11th there.
        let midnight = AT - AT.rem_euclid(DAY) + DAY - 3_600_000;
        log.write_in(midnight - 1000, CET, "before");
        log.write_in(midnight, CET, "after");
        assert_eq!(
            fs::read_to_string(logs.join("warden-2026-11-10.log")).expect("10th"),
            "2026-11-10 23:59:59 +01:00 before\n"
        );
        assert_eq!(
            fs::read_to_string(logs.join("warden-2026-11-11.log")).expect("11th"),
            "2026-11-11 00:00:00 +01:00 after\n"
        );
    }

    #[test]
    fn keeps_a_few_days_and_leaves_other_files_alone() {
        let dir = Dir::new("logfile");
        let logs = dir.path().join("logs");
        fs::create_dir_all(&logs).expect("mkdir");
        fs::write(logs.join("notes.txt"), "mine").expect("write");
        let log = RollingLog::new(logs.clone());
        for d in 0..8 {
            log.write_in(AT + d * DAY, 0, "a line");
        }
        let mut names: Vec<String> = fs::read_dir(&logs)
            .expect("read")
            .map(|e| e.expect("entry").file_name().into_string().expect("name"))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "notes.txt",
                "warden-2026-11-13.log",
                "warden-2026-11-14.log",
                "warden-2026-11-15.log",
                "warden-2026-11-16.log",
                "warden-2026-11-17.log",
            ]
        );
    }

    #[test]
    fn caps_a_day_and_the_total() {
        let dir = Dir::new("logfile");
        let logs = dir.path().join("logs");
        let log = RollingLog::new(logs.clone());
        let long = "x".repeat(1000);
        for d in 0..4 {
            for _ in 0..600 {
                log.write_in(AT + d * DAY, 0, &long);
            }
        }
        let sizes: Vec<u64> = fs::read_dir(&logs)
            .expect("read")
            .map(|e| e.expect("entry").metadata().expect("meta").len())
            .collect();
        assert!(sizes.iter().all(|s| *s <= MAX_FILE_BYTES), "{sizes:?}");
        assert!(sizes.iter().sum::<u64>() <= MAX_TOTAL_BYTES, "{sizes:?}");
        let today = fs::read_to_string(logs.join("warden-2026-11-13.log")).expect("today");
        assert!(today.ends_with("log full for today; later lines are dropped\n"));
    }

    #[test]
    fn days_before_the_last_few_go_even_after_a_gap() {
        let dir = Dir::new("logfile");
        let logs = dir.path().join("logs");
        let log = RollingLog::new(logs.clone());
        log.write_in(AT, 0, "before a week away");
        log.write_in(AT + DAY, 0, "the day after");
        // Warden was not running for a week: on its return, both old days are past the last five.
        let back = RollingLog::new(logs.clone());
        back.write_in(AT + 8 * DAY, 0, "back");
        let names: Vec<String> = fs::read_dir(&logs)
            .expect("read")
            .map(|e| e.expect("entry").file_name().into_string().expect("name"))
            .collect();
        assert_eq!(names, ["warden-2026-11-18.log"]);
        // Four days back is still within the last five; five days back is not.
        log.write_in(AT + 4 * DAY, 0, "a line for the 14th");
        log.write_in(AT + 5 * DAY, 0, "a line for the 15th");
        back.write_in(AT + 9 * DAY, 0, "the 19th");
        assert!(!logs.join("warden-2026-11-14.log").exists());
        assert!(logs.join("warden-2026-11-15.log").exists());
    }
}
