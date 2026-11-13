//! Updates from the repository's releases, in release builds only: Warden looks once shortly after it starts and then
//! once an hour (one small request each), downloads a newer version on its own, and installs it only on "Install and
//! restart" (confirmed) or when Warden quits; never in the middle of work.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;
use warden_server::app::UpdateView;

/// The first look, shortly after the start.
pub const FIRST_CHECK: Duration = Duration::from_secs(10);
/// Then once an hour.
pub const EVERY: Duration = Duration::from_secs(60 * 60);

/// Whether this build looks for updates: development builds never do.
pub const fn enabled() -> bool {
    !cfg!(debug_assertions)
}

/// What Warden needs from an updater: Tauri's in the app, a fake in the tests.
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
    Ready {
        version: String,
        found: F,
        bytes: Vec<u8>,
    },
    Installing,
}

type Report = Box<dyn Fn(Option<UpdateView>) + Send + Sync>;
type Log = Box<dyn Fn(&str) + Send + Sync>;

pub struct Updates<S: Source> {
    source: S,
    /// Tells the core what the window and the tray show.
    report: Report,
    log: Log,
    phase: Mutex<Phase<S::Found>>,
    /// The last look failed: a failure is logged once, until a look works again.
    failed: AtomicBool,
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
    pub fn new(
        source: S,
        report: impl Fn(Option<UpdateView>) + Send + Sync + 'static,
        log: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            source,
            report: Box::new(report),
            log: Box::new(log),
            phase: Mutex::new(Phase::Idle),
            failed: AtomicBool::new(false),
            stopped: Mutex::new(false),
            wake: Condvar::new(),
        }
    }

    fn phase(&self) -> MutexGuard<'_, Phase<S::Found>> {
        self.phase
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
                    (self.log)(&format!("could not look for an update: {e}"));
                }
                None
            }
        };
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
        let downloaded = self.source.download(&found, &mut |done, total| {
            let step = percent_step(done, total);
            if step != shown {
                shown = step;
                (self.report)(Some(view(step, false)));
            }
        });
        match downloaded {
            Ok(bytes) => {
                (self.log)(&format!("update {version} downloaded"));
                *self.phase() = Phase::Ready {
                    version: version.clone(),
                    found,
                    bytes,
                };
                (self.report)(Some(view(Some(100), true)));
            }
            Err(e) => {
                (self.log)(&format!("could not download update {version}: {e}"));
                *self.phase() = Phase::Idle;
                (self.report)(None);
            }
        }
    }

    /// Whether an update is downloaded and ready to install.
    pub fn ready(&self) -> bool {
        matches!(*self.phase(), Phase::Ready { .. })
    }

    /// Installs the downloaded update: "Install and restart" (`restart`) or Warden quitting. `false` when none is
    /// ready or the installer could not start (the update stays ready); in the app, Warden closes on success.
    pub fn install(&self, restart: bool) -> bool {
        let (version, found, bytes) = {
            let mut phase = self.phase();
            match std::mem::replace(&mut *phase, Phase::Installing) {
                Phase::Ready {
                    version,
                    found,
                    bytes,
                } => (version, found, bytes),
                other => {
                    *phase = other;
                    return false;
                }
            }
        };
        (self.log)(&format!("installing update {version}"));
        match self.source.install(&found, &bytes, restart) {
            Ok(()) => true,
            Err(e) => {
                (self.log)(&format!("could not install update {version}: {e}"));
                *self.phase() = Phase::Ready {
                    version,
                    found,
                    bytes,
                };
                false
            }
        }
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

    /// An updater that offers `offer` (or fails), downloads in four chunks and records installs.
    #[derive(Default)]
    struct Fake {
        offer: Mutex<Option<String>>,
        check_fails: AtomicBool,
        download_fails: AtomicBool,
        install_fails: AtomicBool,
        checks: AtomicUsize,
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
            for done in [0, 10, 11, 52, 99, 100] {
                progress(done, Some(100));
            }
            if self.download_fails.load(Ordering::SeqCst) {
                return Err("cut off".into());
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
    }

    fn updates(fake: &Arc<Fake>) -> (Arc<Updates<Arc<Fake>>>, Seen) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (r, l) = (Arc::clone(&reports), Arc::clone(&log));
        let u = Updates::new(
            Arc::clone(fake),
            move |v| r.lock().expect("lock").push(v),
            move |line| l.lock().expect("lock").push(line.to_owned()),
        );
        (Arc::new(u), Seen { reports, log })
    }

    fn view(downloaded: Option<u8>, ready: bool) -> Option<UpdateView> {
        Some(UpdateView {
            version: "0.2.0".into(),
            downloaded,
            ready,
        })
    }

    #[test]
    fn development_builds_never_look() {
        assert_eq!(enabled(), !cfg!(debug_assertions));
        assert_eq!(FIRST_CHECK, Duration::from_secs(10));
        assert_eq!(EVERY, Duration::from_secs(3600));
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
    fn a_found_update_downloads_with_progress_then_waits_ready() {
        let fake = Arc::new(Fake::default());
        let (u, seen) = updates(&fake);
        u.check_now();
        assert!(
            seen.reports.lock().expect("lock").is_empty(),
            "nothing newer"
        );
        *fake.offer.lock().expect("lock") = Some("0.2.0".into());
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
    }

    #[test]
    fn installs_only_when_asked_with_or_without_a_restart() {
        let fake = Arc::new(Fake::default());
        let (u, seen) = updates(&fake);
        assert!(!u.install(true), "nothing ready");
        *fake.offer.lock().expect("lock") = Some("0.2.0".into());
        u.check_now();
        // The installer cannot start: the update stays ready.
        fake.install_fails.store(true, Ordering::SeqCst);
        assert!(!u.install(true));
        assert!(u.ready());
        fake.install_fails.store(false, Ordering::SeqCst);
        // "Install and restart" restarts; quitting does not.
        assert!(u.install(true));
        assert_eq!(
            *fake.installs.lock().expect("lock"),
            [("0.2.0".to_owned(), 100, true)]
        );
        assert!(!u.ready());
        assert!(!u.install(false), "installing already");
        let (u, _) = updates(&fake);
        u.check_now();
        assert!(u.install(false));
        assert!(!fake.installs.lock().expect("lock")[1].2);
        assert!(
            seen.log
                .lock()
                .expect("lock")
                .contains(&"installing update 0.2.0".to_owned())
        );
    }

    #[test]
    fn a_failed_look_is_logged_once_and_a_failed_download_tried_again_later() {
        let fake = Arc::new(Fake::default());
        let (u, seen) = updates(&fake);
        fake.check_fails.store(true, Ordering::SeqCst);
        u.check_now();
        u.check_now();
        assert_eq!(
            *seen.log.lock().expect("lock"),
            ["could not look for an update: offline"]
        );
        fake.check_fails.store(false, Ordering::SeqCst);
        *fake.offer.lock().expect("lock") = Some("0.2.0".into());
        fake.download_fails.store(true, Ordering::SeqCst);
        u.check_now();
        assert!(!u.ready());
        assert_eq!(seen.reports.lock().expect("lock").last(), Some(&None));
        fake.download_fails.store(false, Ordering::SeqCst);
        u.check_now();
        assert!(u.ready(), "the next look downloads it again");
    }

    #[test]
    fn looks_first_soon_then_on_each_interval_until_stopped() {
        let fake = Arc::new(Fake::default());
        let (u, _) = updates(&fake);
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
