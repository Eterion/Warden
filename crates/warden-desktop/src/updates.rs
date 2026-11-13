//! Updates from the repository's releases, for the installed app in a release build: Warden looks once shortly after it
//! starts and then once an hour (one small request each), downloads a newer version on its own, and installs it only
//! on "Install and restart" (confirmed) or when the user quits Warden; never in the middle of work, and never when
//! Windows ends the session.
//!
//! The downloaded installer waits in the data folder (`update/installer`), not in memory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;
use warden_server::app::UpdateView;

/// The first look, shortly after the start.
pub const FIRST_CHECK: Duration = Duration::from_secs(10);
/// Then once an hour.
pub const EVERY: Duration = Duration::from_secs(60 * 60);

const INSTALLER: &str = "installer";
/// Left by "Install and restart", so the restarted Warden opens its window even when Windows started the old one into
/// the tray.
const OPEN_WINDOW: &str = "open-window";

/// Whether this build looks for updates: development builds never do.
pub const fn enabled() -> bool {
    !cfg!(debug_assertions)
}

/// Whether the last "Install and restart" asked for the window, forgetting it.
pub fn take_open_window_mark(dir: &Path) -> bool {
    std::fs::remove_file(dir.join(OPEN_WINDOW)).is_ok()
}

/// What Warden needs from an updater: Tauri's in the app, a fake in the tests. Errors are short words, never the
/// response's own text.
pub trait Source: Send + Sync + 'static {
    /// A newer version found by `check`, kept until it is installed.
    type Found: Send + 'static;
    /// A version newer than this one, with its number, or `None`.
    fn check(&self) -> Result<Option<(String, Self::Found)>, String>;
    /// Downloads it (its signature checked), telling the bytes so far and the total when known.
    fn download(
        &self,
        found: &Self::Found,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, String>;
    /// Runs the installer, which starts Warden again when `restart`. Warden closes for it, so this returns only when
    /// the installer could not start.
    fn install(&self, found: &Self::Found, bytes: &[u8], restart: bool) -> Result<(), String>;
}

enum Phase<F> {
    Idle,
    Downloading,
    Ready { version: String, found: F },
    Installing,
}

type Report = Box<dyn Fn(Option<UpdateView>) + Send + Sync>;
type Log = Box<dyn Fn(&str) + Send + Sync>;

pub struct Updates<S: Source> {
    source: S,
    /// Where the downloaded installer waits.
    dir: PathBuf,
    /// Tells the core what the window and the tray show.
    report: Report,
    log: Log,
    phase: Mutex<Phase<S::Found>>,
    /// The last look failed: a failure is logged once, until a look works again.
    failed: AtomicBool,
    /// A version whose download failed (its signature, say): not downloaded again until another one is offered.
    skipped: Mutex<Option<String>>,
    /// The user asked Warden to quit (Quit, or closing the window with the tray off), so its exit may install.
    quitting: AtomicBool,
    stopped: Mutex<bool>,
    wake: Condvar,
}

/// The download's progress in steps of 5 percent, so the window is not woken for every chunk; `None` while the size
/// is not known.
pub fn percent_step(done: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|t| *t > 0)?;
    let percent = u8::try_from(done.min(total) * 100 / total).unwrap_or(100);
    Some(percent / 5 * 5)
}

/// Whether `found` is newer than `current` (semantic versions; a pre-release counts below its release).
pub fn is_newer(current: &semver::Version, found: &semver::Version) -> bool {
    found > current
}

impl<S: Source> Updates<S> {
    /// `dir` holds the downloaded installer; one left from an earlier run is removed.
    pub fn new(
        source: S,
        dir: PathBuf,
        report: impl Fn(Option<UpdateView>) + Send + Sync + 'static,
        log: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        let _ = std::fs::remove_file(dir.join(INSTALLER));
        Self {
            source,
            dir,
            report: Box::new(report),
            log: Box::new(log),
            phase: Mutex::new(Phase::Idle),
            failed: AtomicBool::new(false),
            skipped: Mutex::new(None),
            quitting: AtomicBool::new(false),
            stopped: Mutex::new(false),
            wake: Condvar::new(),
        }
    }

    fn phase(&self) -> MutexGuard<'_, Phase<S::Found>> {
        self.phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn skipped(&self) -> MutexGuard<'_, Option<String>> {
        self.skipped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One look: when a newer version is found, it is downloaded at once and then ready. Nothing is looked for while
    /// one downloads, is ready or installs.
    pub fn check_now(&self) {
        {
            let mut phase = self.phase();
            if !matches!(*phase, Phase::Idle) {
                return;
            }
            *phase = Phase::Downloading;
        }
        let found = match self.source.check() {
            Ok(found) => {
                self.failed.store(false, Ordering::Relaxed);
                found
            }
            Err(e) => {
                if !self.failed.swap(true, Ordering::Relaxed) {
                    (self.log)(&format!("could not look for an update ({e})"));
                }
                None
            }
        };
        let found = found.filter(|(version, _)| self.skipped().as_ref() != Some(version));
        let Some((version, found)) = found else {
            *self.phase() = Phase::Idle;
            return;
        };
        (self.log)(&format!("update {version} found"));
        let view = |downloaded, ready| UpdateView {
            version: version.clone(),
            downloaded,
            ready,
        };
        (self.report)(Some(view(None, false)));
        let mut shown = None;
        let downloaded = self
            .source
            .download(&found, &mut |done, total| {
                let step = percent_step(done, total);
                if step != shown {
                    shown = step;
                    (self.report)(Some(view(step, false)));
                }
            })
            .and_then(|bytes| {
                std::fs::create_dir_all(&self.dir)
                    .and_then(|()| std::fs::write(self.dir.join(INSTALLER), bytes))
                    .map_err(|e| format!("could not keep it: {}", e.kind()))
            });
        match downloaded {
            Ok(()) => {
                (self.log)(&format!("update {version} downloaded"));
                *self.phase() = Phase::Ready {
                    version: version.clone(),
                    found,
                };
                (self.report)(Some(view(Some(100), true)));
            }
            Err(e) => {
                (self.log)(&format!(
                    "could not download update {version} ({e}); it is not tried again until another version is offered"
                ));
                *self.skipped() = Some(version);
                *self.phase() = Phase::Idle;
                (self.report)(None);
            }
        }
    }

    /// Whether an update is downloaded and ready to install.
    pub fn ready(&self) -> bool {
        matches!(*self.phase(), Phase::Ready { .. })
    }

    /// Installs the downloaded update: "Install and restart" (`restart`, which also asks the restarted Warden to open
    /// its window) or the user quitting. In the app Warden closes on success; an error says why it could not start,
    /// and the update stays ready.
    pub fn install(&self, restart: bool) -> Result<(), String> {
        let (version, found) = {
            let mut phase = self.phase();
            match std::mem::replace(&mut *phase, Phase::Installing) {
                Phase::Ready { version, found } => (version, found),
                other => {
                    *phase = other;
                    return Err("no update is ready".into());
                }
            }
        };
        (self.log)(&format!("installing update {version}"));
        let mark = self.dir.join(OPEN_WINDOW);
        let started = std::fs::read(self.dir.join(INSTALLER))
            .map_err(|e| format!("could not read it: {}", e.kind()))
            .and_then(|bytes| {
                if restart {
                    let _ = std::fs::write(&mark, b"");
                }
                self.source.install(&found, &bytes, restart)
            });
        if let Err(e) = &started {
            (self.log)(&format!("could not install update {version} ({e})"));
            let _ = std::fs::remove_file(&mark);
            *self.phase() = Phase::Ready { version, found };
        }
        started
    }

    /// The user asked Warden to quit, so its exit installs a ready update.
    pub fn quit_requested(&self) {
        self.quitting.store(true, Ordering::Relaxed);
    }

    /// Warden exits: a ready update installs (without a restart) only when the user asked to quit, never when
    /// Windows ends the session. Whether the installer started.
    pub fn on_exit(&self) -> bool {
        self.stop();
        self.quitting.load(Ordering::Relaxed) && self.ready() && self.install(false).is_ok()
    }

    /// Looks once after `first`, then every `every`, until stopped.
    pub fn run(&self, first: Duration, every: Duration) {
        let mut wait = first;
        while self.sleep(wait) {
            self.check_now();
            wait = every;
        }
    }

    /// Waits, and says whether to go on (`false` once stopped).
    fn sleep(&self, wait: Duration) -> bool {
        let stopped = self
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (stopped, _) = self
            .wake
            .wait_timeout_while(stopped, wait, |s| !*s)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !*stopped
    }

    pub fn stop(&self) {
        *self
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use warden_core::testdir::Dir;

    /// An updater that offers `offer` (or fails), downloads in chunks and records installs.
    #[derive(Default)]
    struct Fake {
        offer: Mutex<Option<String>>,
        check_fails: AtomicBool,
        download_fails: AtomicBool,
        install_fails: AtomicBool,
        checks: AtomicUsize,
        downloads: AtomicUsize,
        installs: Mutex<Vec<(String, usize, bool)>>,
    }

    impl Source for Arc<Fake> {
        type Found = String;
        fn check(&self) -> Result<Option<(String, String)>, String> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            if self.check_fails.load(Ordering::SeqCst) {
                return Err("offline".into());
            }
            Ok(self
                .offer
                .lock()
                .expect("lock")
                .clone()
                .map(|v| (v.clone(), v)))
        }
        fn download(
            &self,
            _: &String,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<Vec<u8>, String> {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            for done in [0, 10, 11, 52, 99, 100] {
                progress(done, Some(100));
            }
            if self.download_fails.load(Ordering::SeqCst) {
                return Err("bad signature".into());
            }
            Ok(vec![7; 100])
        }
        fn install(&self, found: &String, bytes: &[u8], restart: bool) -> Result<(), String> {
            if self.install_fails.load(Ordering::SeqCst) {
                return Err("refused".into());
            }
            self.installs
                .lock()
                .expect("lock")
                .push((found.clone(), bytes.len(), restart));
            Ok(())
        }
    }

    struct Seen {
        reports: Arc<Mutex<Vec<Option<UpdateView>>>>,
        log: Arc<Mutex<Vec<String>>>,
        dir: Dir,
    }

    fn updates(fake: &Arc<Fake>) -> (Arc<Updates<Arc<Fake>>>, Seen) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let dir = Dir::new("updates");
        let (r, l) = (Arc::clone(&reports), Arc::clone(&log));
        let u = Updates::new(
            Arc::clone(fake),
            dir.path().join("update"),
            move |v| r.lock().expect("lock").push(v),
            move |line| l.lock().expect("lock").push(line.to_owned()),
        );
        (Arc::new(u), Seen { reports, log, dir })
    }

    fn offer(fake: &Fake, version: &str) {
        *fake.offer.lock().expect("lock") = Some(version.into());
    }

    fn view(downloaded: Option<u8>, ready: bool) -> Option<UpdateView> {
        Some(UpdateView {
            version: "0.2.0".into(),
            downloaded,
            ready,
        })
    }

    #[test]
    fn looks_soon_after_the_start_then_hourly() {
        assert_eq!(FIRST_CHECK, Duration::from_secs(10));
        assert_eq!(EVERY, Duration::from_secs(3600));
        #[cfg(debug_assertions)]
        assert!(!enabled(), "development builds never look");
    }

    #[test]
    fn progress_moves_in_steps_of_five_percent() {
        assert_eq!(percent_step(0, None), None);
        assert_eq!(percent_step(10, Some(0)), None);
        assert_eq!(percent_step(0, Some(200)), Some(0));
        assert_eq!(percent_step(9, Some(100)), Some(5));
        assert_eq!(percent_step(99, Some(100)), Some(95));
        assert_eq!(percent_step(100, Some(100)), Some(100));
        assert_eq!(percent_step(250, Some(100)), Some(100));
    }

    #[test]
    fn only_a_newer_version_counts() {
        let v = |s: &str| semver::Version::parse(s).expect("version");
        assert!(is_newer(&v("0.1.0"), &v("0.2.0")));
        assert!(is_newer(&v("0.2.0"), &v("0.2.1")));
        assert!(is_newer(&v("0.9.0"), &v("0.10.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.2.0")));
        assert!(!is_newer(&v("0.2.1"), &v("0.2.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.2.0-rc.1")));
    }

    #[test]
    fn a_found_update_downloads_with_progress_then_waits_ready_on_disk() {
        let fake = Arc::new(Fake::default());
        let (u, seen) = updates(&fake);
        u.check_now();
        assert!(
            seen.reports.lock().expect("lock").is_empty(),
            "nothing newer"
        );
        offer(&fake, "0.2.0");
        u.check_now();
        assert_eq!(
            *seen.reports.lock().expect("lock"),
            vec![
                view(None, false),
                view(Some(0), false),
                view(Some(10), false),
                view(Some(50), false),
                view(Some(95), false),
                view(Some(100), false),
                view(Some(100), true),
            ],
            "11% is still the 10% step: no report"
        );
        assert!(u.ready());
        let installer = seen.dir.path().join("update").join(INSTALLER);
        assert_eq!(std::fs::read(&installer).expect("kept").len(), 100);
        // Ready: no more looks until it is installed.
        let checks = fake.checks.load(Ordering::SeqCst);
        u.check_now();
        assert_eq!(fake.checks.load(Ordering::SeqCst), checks);
        assert!(
            fake.installs.lock().expect("lock").is_empty(),
            "never on its own"
        );
        assert_eq!(
            *seen.log.lock().expect("lock"),
            ["update 0.2.0 found", "update 0.2.0 downloaded"]
        );
        // A new start removes an installer left from before.
        let _again = Updates::new(
            Arc::clone(&fake),
            seen.dir.path().join("update"),
            |_| {},
            |_| {},
        );
        assert!(!installer.exists());
    }

    #[test]
    fn the_button_installs_and_restarts_into_the_window() {
        let fake = Arc::new(Fake::default());
        let (u, seen) = updates(&fake);
        let dir = seen.dir.path().join("update");
        assert_eq!(u.install(true), Err("no update is ready".into()));
        offer(&fake, "0.2.0");
        u.check_now();
        // The installer cannot start: why, and the update stays ready, with no window asked for.
        fake.install_fails.store(true, Ordering::SeqCst);
        assert_eq!(u.install(true), Err("refused".into()));
        assert!(u.ready());
        assert!(!take_open_window_mark(&dir));
        fake.install_fails.store(false, Ordering::SeqCst);
        assert_eq!(u.install(true), Ok(()));
        assert_eq!(
            *fake.installs.lock().expect("lock"),
            [("0.2.0".to_owned(), 100, true)]
        );
        assert!(!u.ready());
        assert!(u.install(false).is_err(), "installing already");
        // The restarted Warden opens its window once.
        assert!(take_open_window_mark(&dir));
        assert!(!take_open_window_mark(&dir));
        assert!(
            seen.log
                .lock()
                .expect("lock")
                .contains(&"installing update 0.2.0".to_owned())
        );
    }

    #[test]
    fn only_quitting_installs_on_exit_without_a_restart() {
        let fake = Arc::new(Fake::default());
        // Windows ends the session: Warden exits without the user's Quit, and nothing installs.
        let (u, _seen) = updates(&fake);
        offer(&fake, "0.2.0");
        u.check_now();
        assert!(!u.on_exit());
        assert!(fake.installs.lock().expect("lock").is_empty());
        // The user quits: the update installs, and Warden does not start again.
        let (u, seen) = updates(&fake);
        u.check_now();
        u.quit_requested();
        assert!(u.on_exit());
        assert_eq!(
            *fake.installs.lock().expect("lock"),
            [("0.2.0".to_owned(), 100, false)]
        );
        assert!(!take_open_window_mark(&seen.dir.path().join("update")));
        // Quitting with nothing ready installs nothing.
        let (u, _) = updates(&Arc::new(Fake::default()));
        u.quit_requested();
        assert!(!u.on_exit());
    }

    #[test]
    fn a_failed_look_is_logged_once_and_a_failed_download_not_repeated() {
        let fake = Arc::new(Fake::default());
        let (u, seen) = updates(&fake);
        fake.check_fails.store(true, Ordering::SeqCst);
        u.check_now();
        u.check_now();
        assert_eq!(
            *seen.log.lock().expect("lock"),
            ["could not look for an update (offline)"]
        );
        fake.check_fails.store(false, Ordering::SeqCst);
        offer(&fake, "0.2.0");
        fake.download_fails.store(true, Ordering::SeqCst);
        u.check_now();
        assert!(!u.ready());
        assert_eq!(seen.reports.lock().expect("lock").last(), Some(&None));
        // The same version is not downloaded again, every hour, forever.
        u.check_now();
        u.check_now();
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
        // Another version is.
        fake.download_fails.store(false, Ordering::SeqCst);
        offer(&fake, "0.2.1");
        u.check_now();
        assert!(u.ready());
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn looks_first_soon_then_on_each_interval_until_stopped() {
        let fake = Arc::new(Fake::default());
        let (u, _seen) = updates(&fake);
        let running = Arc::clone(&u);
        let started = std::time::Instant::now();
        let thread = std::thread::spawn(move || {
            running.run(Duration::from_millis(20), Duration::from_millis(60));
        });
        while fake.checks.load(Ordering::SeqCst) < 3 {
            assert!(started.elapsed() < Duration::from_secs(10), "it looks");
            std::thread::sleep(Duration::from_millis(5));
        }
        // The first look after `first`, then one each `every`: three looks take at least 20 + 60 + 60 ms.
        assert!(started.elapsed() >= Duration::from_millis(140));
        u.stop();
        thread.join().expect("stops");
        let checks = fake.checks.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            fake.checks.load(Ordering::SeqCst),
            checks,
            "no look after stop"
        );
    }
}
