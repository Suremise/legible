//! Save as PDF, the parts that aren't Windows': where the PDF goes by default, and the file
//! it's printed to first.
//!
//! WebView2 prints to a staging file beside the one chosen ([`staging_path`]), named for this
//! process and job, and only a print that worked replaces the chosen file: one that fails, is
//! cancelled or runs out of time leaves a PDF already there as it was, and removes only its own
//! staging file.
//!
//! [`Jobs`] keeps every job's stage for the process, beyond its window's life: choosing,
//! printing, committing (its staging file replacing the chosen one, which nothing can call off
//! once it's started), or given up (it ran out of time while WebView2 was printing). A window
//! has one job at a time; a job that ran out of time while printing never commits, and its
//! window is free again at once. A window that closes ends its jobs but a commit: the staging
//! file of one printing or given up is removed (its WebView2 is gone, so nothing more comes).
//!
//! [`Jobs::busy`] says whether work the process owns is still unsettled (a commit, a print
//! given up, a staging file being removed): Legible doesn't exit, with its last window closed,
//! until it's settled ([`remove_staging`] and [`commit`] are that work, off the UI thread).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The PDF's default name: the document's, with `.pdf` for its extension.
pub fn default_name(document: &Path) -> String {
    let stem = document
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "document".to_owned());
    format!("{stem}.pdf")
}

/// Where the Save dialog opens: the document's folder if a file can be made there, else
/// `documents` (the user's Documents), else nowhere in particular.
pub fn default_folder(
    folder: Option<&Path>,
    writable: impl Fn(&Path) -> bool,
    documents: Option<PathBuf>,
) -> Option<PathBuf> {
    match folder {
        Some(folder) if writable(folder) => Some(folder.to_owned()),
        _ => documents,
    }
}

/// Whether a file can be made in `folder`: one is made and removed.
pub fn can_write_in(folder: &Path) -> bool {
    let probe = folder.join(format!(".legible-write-check-{}.tmp", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// The file job `job` of process `pid` prints to, beside `chosen` (the same volume, so it can
/// replace it in one step).
pub fn staging_path(chosen: &Path, pid: u32, job: u64) -> PathBuf {
    let name = chosen
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document.pdf".to_owned());
    chosen.with_file_name(format!(".{name}.legible-{pid}-{job}.tmp"))
}

/// The paper Save as PDF uses: Letter where the user's locale measures in inches, else A4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    A4,
    Letter,
}

impl Paper {
    /// Width and height in inches, as WebView2's print settings take them.
    pub fn inches(self) -> (f64, f64) {
        match self {
            Paper::A4 => (210.0 / 25.4, 297.0 / 25.4),
            Paper::Letter => (8.5, 11.0),
        }
    }

    /// From Windows' LOCALE_IMEASURE: "1" is the US system (inches).
    pub fn for_measure(measure: &str) -> Paper {
        if measure.trim() == "1" {
            Paper::Letter
        } else {
            Paper::A4
        }
    }
}

/// Where a job is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    /// Its folder being found, or the Save dialog open.
    Choosing,
    /// WebView2 printing to `staging`, for `chosen`.
    Printing { staging: PathBuf, chosen: PathBuf },
    /// `staging` replacing `chosen`: once started, it runs to the end.
    Committing { staging: PathBuf, chosen: PathBuf },
    /// Printing still, but given up (it ran out of time): when WebView2 ends it, its staging
    /// file is removed, and nothing more.
    Abandoned { staging: PathBuf },
}

struct Job<W> {
    /// The window it's for; None once that window has closed.
    window: Option<W>,
    stage: Stage,
}

/// What the shell does next for a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<W> {
    /// Replace `chosen` with `staging` (off the UI thread), then report it with
    /// [`Jobs::committed`] for `job`.
    Commit {
        job: u64,
        staging: PathBuf,
        chosen: PathBuf,
    },
    /// Remove this staging file (off the UI thread, trying again while it's held).
    Remove(PathBuf),
    /// Tell `window` its PDF is saved as `chosen`.
    Saved { window: W, chosen: PathBuf },
    /// Tell `window` it couldn't be saved.
    Failed { window: W, reason: String },
}

/// Every Save as PDF job of the process, numbered, and the staging files being removed.
pub struct Jobs<W> {
    next: u64,
    jobs: HashMap<u64, Job<W>>,
    /// [`Step::Remove`]s handed out and not yet reported done ([`Jobs::removed`]).
    removing: usize,
}

impl<W> Default for Jobs<W> {
    fn default() -> Self {
        Jobs {
            next: 0,
            jobs: HashMap::new(),
            removing: 0,
        }
    }
}

impl<W: Copy + Eq> Jobs<W> {
    /// Whether work the process owns is unsettled: a job committing or given up, or a staging
    /// file being removed. (A job choosing or printing has a window, which keeps Legible open.)
    pub fn busy(&self) -> bool {
        self.removing > 0
            || self.jobs.values().any(|job| {
                matches!(
                    job.stage,
                    Stage::Committing { .. } | Stage::Abandoned { .. }
                )
            })
    }

    /// A [`Step::Remove`] handed out earlier has finished (the file gone, or given up on).
    pub fn removed(&mut self) {
        self.removing = self.removing.saturating_sub(1);
    }

    /// Counts the removals in `steps`, so [`Jobs::busy`] holds until each is [`Jobs::removed`].
    fn counted(&mut self, steps: Vec<Step<W>>) -> Vec<Step<W>> {
        self.removing += steps
            .iter()
            .filter(|s| matches!(s, Step::Remove(_)))
            .count();
        steps
    }

    /// Whether `window` has a job choosing, printing or committing.
    pub fn running(&self, window: W) -> bool {
        self.jobs
            .values()
            .any(|job| job.window == Some(window) && !matches!(job.stage, Stage::Abandoned { .. }))
    }

    /// A new job for `window`, or None while it has one choosing, printing or committing.
    pub fn start(&mut self, window: W) -> Option<u64> {
        if self.running(window) {
            return None;
        }
        self.next += 1;
        self.jobs.insert(
            self.next,
            Job {
                window: Some(window),
                stage: Stage::Choosing,
            },
        );
        Some(self.next)
    }

    /// Whether job `job` is waiting for its dialog (its window still open).
    pub fn choosing(&self, job: u64) -> bool {
        self.jobs
            .get(&job)
            .is_some_and(|j| j.window.is_some() && j.stage == Stage::Choosing)
    }

    /// The dialog was cancelled, or the print couldn't start: the job is over.
    pub fn cancelled(&mut self, job: u64) {
        if self.choosing(job) {
            self.jobs.remove(&job);
        }
    }

    /// WebView2 is printing job `job` to `staging`, for `chosen`.
    pub fn printing(&mut self, job: u64, staging: PathBuf, chosen: PathBuf) {
        if let Some(j) = self.jobs.get_mut(&job)
            && j.stage == Stage::Choosing
        {
            j.stage = Stage::Printing { staging, chosen };
        }
    }

    /// WebView2 finished job `job`'s print. Printed for an open job: it commits. Failed: its
    /// staging file goes. Given up meanwhile: only its staging file goes, and it never commits.
    pub fn printed(&mut self, job: u64, result: Result<(), String>) -> Vec<Step<W>> {
        let steps = self.printed_steps(job, result);
        self.counted(steps)
    }

    fn printed_steps(&mut self, job: u64, result: Result<(), String>) -> Vec<Step<W>> {
        let Some(j) = self.jobs.get_mut(&job) else {
            return Vec::new();
        };
        match (j.stage.clone(), result) {
            (Stage::Printing { staging, chosen }, Ok(())) => {
                j.stage = Stage::Committing {
                    staging: staging.clone(),
                    chosen: chosen.clone(),
                };
                vec![Step::Commit {
                    job,
                    staging,
                    chosen,
                }]
            }
            (Stage::Printing { staging, .. }, Err(reason)) => {
                let window = j.window;
                self.jobs.remove(&job);
                let mut steps = vec![Step::Remove(staging)];
                steps.extend(window.map(|window| Step::Failed { window, reason }));
                steps
            }
            (Stage::Abandoned { staging }, _) => {
                self.jobs.remove(&job);
                vec![Step::Remove(staging)]
            }
            _ => Vec::new(),
        }
    }

    /// Job `job`'s staging file replaced the chosen one (or couldn't).
    pub fn committed(&mut self, job: u64, result: Result<(), String>) -> Vec<Step<W>> {
        let steps = self.committed_steps(job, result);
        self.counted(steps)
    }

    fn committed_steps(&mut self, job: u64, result: Result<(), String>) -> Vec<Step<W>> {
        let Some(Job {
            window,
            stage: Stage::Committing { staging, chosen },
        }) = self.jobs.get(&job).map(|j| Job {
            window: j.window,
            stage: j.stage.clone(),
        })
        else {
            return Vec::new();
        };
        self.jobs.remove(&job);
        match (result, window) {
            (Ok(()), Some(window)) => vec![Step::Saved { window, chosen }],
            (Ok(()), None) => Vec::new(),
            (Err(reason), window) => {
                let mut steps = vec![Step::Remove(staging)];
                steps.extend(window.map(|window| Step::Failed { window, reason }));
                steps
            }
        }
    }

    /// Job `job` has run out of time. Printing, it's given up (its window is free again, and it
    /// will never commit); committing, it can't be called off, and runs to the end.
    pub fn timed_out(&mut self, job: u64, reason: &str) -> Vec<Step<W>> {
        let Some(j) = self.jobs.get_mut(&job) else {
            return Vec::new();
        };
        let Stage::Printing { staging, .. } = j.stage.clone() else {
            return Vec::new();
        };
        let window = j.window;
        j.stage = Stage::Abandoned { staging };
        window
            .map(|window| Step::Failed {
                window,
                reason: reason.to_owned(),
            })
            .into_iter()
            .collect()
    }

    /// `window` closed: its WebView2 is gone, so nothing more comes for its jobs. One choosing
    /// is over; one printing or given up is over too, its staging file removed; one committing
    /// runs to the end, unannounced.
    pub fn window_closed(&mut self, window: W) -> Vec<Step<W>> {
        let mut steps = Vec::new();
        self.jobs.retain(|_, job| {
            if job.window != Some(window) {
                return true;
            }
            job.window = None;
            match job.stage.clone() {
                Stage::Choosing => false,
                Stage::Printing { staging, .. } | Stage::Abandoned { staging } => {
                    steps.push(Step::Remove(staging));
                    false
                }
                Stage::Committing { .. } => true,
            }
        });
        self.counted(steps)
    }
}

/// Replaces `chosen` with `staging`, in one step (they're in the same folder).
pub fn commit(staging: &Path, chosen: &Path) -> Result<(), String> {
    std::fs::rename(staging, chosen).map_err(|error| {
        tracing::warn!(%error, "couldn't put a PDF in its place");
        crate::reason::of(&error)
    })
}

/// Removes a staging file, trying again while it's held (WebView2 may still be closing it), for
/// `bound` at most. Says whether it's gone.
pub fn remove_staging(staging: &Path, bound: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + bound;
    loop {
        match std::fs::remove_file(staging) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(n: u64) -> (PathBuf, PathBuf) {
        let chosen = PathBuf::from(r"D:\Out\Plan.pdf");
        (staging_path(&chosen, 1, n), chosen)
    }

    #[test]
    fn a_job_prints_then_commits_then_is_saved() {
        let mut jobs = Jobs::default();
        let job = jobs.start('a').unwrap();
        assert!(jobs.choosing(job));
        assert_eq!(jobs.start('a'), None, "one at a time per window");
        assert!(jobs.start('b').is_some(), "another window's own");
        let (staging, chosen) = paths(job);
        jobs.printing(job, staging.clone(), chosen.clone());
        assert_eq!(
            jobs.printed(job, Ok(())),
            [Step::Commit {
                job,
                staging,
                chosen: chosen.clone()
            }]
        );
        assert_eq!(jobs.start('a'), None, "still busy while committing");
        assert_eq!(
            jobs.committed(job, Ok(())),
            [Step::Saved {
                window: 'a',
                chosen
            }]
        );
        assert!(jobs.start('a').is_some(), "free again");
    }

    #[test]
    fn a_print_that_ran_out_of_time_never_commits_over_a_newer_save() {
        let mut jobs = Jobs::default();
        let old = jobs.start('a').unwrap();
        let (old_staging, chosen) = paths(old);
        jobs.printing(old, old_staging.clone(), chosen.clone());
        let failed = jobs.timed_out(old, "it took too long");
        assert!(matches!(&failed[..], [Step::Failed { window: 'a', .. }]));
        // The window is free; a newer save of the same file goes ahead and commits.
        let new = jobs.start('a').unwrap();
        let (new_staging, _) = paths(new);
        jobs.printing(new, new_staging.clone(), chosen.clone());
        assert!(matches!(
            &jobs.printed(new, Ok(()))[..],
            [Step::Commit { .. }]
        ));
        // The old print ends late: only its own staging file goes, and it never commits.
        assert_eq!(jobs.printed(old, Ok(())), [Step::Remove(old_staging)]);
        assert_eq!(
            jobs.committed(new, Ok(())),
            [Step::Saved {
                window: 'a',
                chosen
            }]
        );
        // And a callback for a job that's gone does nothing.
        assert!(jobs.printed(old, Ok(())).is_empty());
        assert!(jobs.committed(old, Ok(())).is_empty());
    }

    #[test]
    fn a_commit_under_way_can_neither_time_out_nor_be_overtaken() {
        let mut jobs = Jobs::default();
        let job = jobs.start('a').unwrap();
        let (staging, chosen) = paths(job);
        jobs.printing(job, staging, chosen);
        jobs.printed(job, Ok(()));
        // The rename is slow (a share): the timer firing changes nothing...
        assert!(jobs.timed_out(job, "it took too long").is_empty());
        // ...and no other save of this window can start until it ends.
        assert_eq!(jobs.start('a'), None);
        assert!(matches!(
            &jobs.committed(job, Ok(()))[..],
            [Step::Saved { .. }]
        ));
    }

    #[test]
    fn a_failed_print_or_commit_removes_only_its_own_staging_file() {
        let mut jobs = Jobs::default();
        let job = jobs.start('a').unwrap();
        let (staging, chosen) = paths(job);
        jobs.printing(job, staging.clone(), chosen.clone());
        assert_eq!(
            jobs.printed(job, Err("no printer".into())),
            [
                Step::Remove(staging),
                Step::Failed {
                    window: 'a',
                    reason: "no printer".into()
                }
            ]
        );
        let job = jobs.start('a').unwrap();
        let (staging, chosen) = paths(job);
        jobs.printing(job, staging.clone(), chosen);
        jobs.printed(job, Ok(()));
        assert_eq!(
            jobs.committed(job, Err("in use".into())),
            [
                Step::Remove(staging),
                Step::Failed {
                    window: 'a',
                    reason: "in use".into()
                }
            ]
        );
    }

    #[test]
    fn closing_a_window_cleans_up_its_print_and_lets_a_commit_finish() {
        let mut jobs = Jobs::default();
        // Choosing: over, and nothing owed.
        let choosing = jobs.start('a').unwrap();
        assert!(jobs.window_closed('a').is_empty());
        assert!(!jobs.choosing(choosing) && !jobs.busy());
        // Printing: over, its staging file removed (its WebView2 is gone); busy until removed;
        // a late callback does nothing.
        let printing = jobs.start('b').unwrap();
        let (staging, chosen) = paths(printing);
        jobs.printing(printing, staging.clone(), chosen);
        assert_eq!(jobs.window_closed('b'), [Step::Remove(staging)]);
        assert!(jobs.busy());
        jobs.removed();
        assert!(!jobs.busy());
        assert!(jobs.printed(printing, Ok(())).is_empty());
        // Given up (timed out), then closed: removed too.
        let timed_out = jobs.start('e').unwrap();
        let (staging, chosen) = paths(timed_out);
        jobs.printing(timed_out, staging.clone(), chosen);
        jobs.timed_out(timed_out, "slow");
        assert!(jobs.busy(), "a print given up is still owed its cleanup");
        assert_eq!(jobs.window_closed('e'), [Step::Remove(staging)]);
        jobs.removed();
        assert!(!jobs.busy());
        // Committing: it runs to the end, with nobody to tell, and is owed until then.
        let committing = jobs.start('c').unwrap();
        let (staging, chosen) = paths(committing);
        jobs.printing(committing, staging, chosen);
        jobs.printed(committing, Ok(()));
        assert!(jobs.window_closed('c').is_empty());
        assert!(jobs.busy());
        assert!(jobs.committed(committing, Ok(())).is_empty());
        assert!(!jobs.busy());
        // A cancelled dialog ends its job too.
        let cancelled = jobs.start('d').unwrap();
        jobs.cancelled(cancelled);
        assert!(jobs.start('d').is_some());
    }

    /// A gate the "share" holds a file operation at; opened by the test (or when dropped).
    struct Gate(std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl Gate {
        fn new() -> Gate {
            Gate(std::sync::Arc::default())
        }
        fn wait(gate: &std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>) {
            let (open, opened) = &**gate;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut is_open = open.lock().unwrap();
            while !*is_open && std::time::Instant::now() < deadline {
                is_open = opened
                    .wait_timeout(is_open, std::time::Duration::from_millis(20))
                    .unwrap()
                    .0;
            }
        }
        fn open(&self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    impl Drop for Gate {
        fn drop(&mut self) {
            self.open();
        }
    }

    /// The shell's part, as a test runs it: each step's file work on a thread of its own (held
    /// at `gate`), its end reported back on `done` as the event loop would hear it.
    fn run_steps(
        steps: Vec<Step<char>>,
        gate: &Gate,
        done: &std::sync::mpsc::Sender<(Option<u64>, Result<(), String>)>,
    ) {
        for step in steps {
            let (gate, done) = (gate.0.clone(), done.clone());
            match step {
                Step::Commit {
                    job,
                    staging,
                    chosen,
                } => {
                    std::thread::spawn(move || {
                        Gate::wait(&gate);
                        let _ = done.send((Some(job), commit(&staging, &chosen)));
                    });
                }
                Step::Remove(staging) => {
                    std::thread::spawn(move || {
                        Gate::wait(&gate);
                        remove_staging(&staging, std::time::Duration::from_secs(5));
                        let _ = done.send((None, Ok(())));
                    });
                }
                Step::Saved { .. } | Step::Failed { .. } => {}
            }
        }
    }

    /// Settles what comes back until nothing is owed (as the event loop does before exiting).
    fn settle(
        jobs: &mut Jobs<char>,
        gate: &Gate,
        tx: &std::sync::mpsc::Sender<(Option<u64>, Result<(), String>)>,
        rx: &std::sync::mpsc::Receiver<(Option<u64>, Result<(), String>)>,
    ) {
        while jobs.busy() {
            let (job, result) = rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("owed work never ended");
            match job {
                Some(job) => {
                    let steps = jobs.committed(job, result);
                    run_steps(steps, gate, tx);
                }
                None => jobs.removed(),
            }
        }
    }

    #[test]
    fn the_last_window_closing_waits_for_its_commit_and_cleanups_with_real_files() {
        let temp = tempfile::tempdir().unwrap();
        let chosen = temp.path().join("Plan.pdf");
        std::fs::write(&chosen, b"%PDF- the old one").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let gate = Gate::new();
        let mut jobs = Jobs::default();

        // Window a: printed, committing, held on a slow share.
        let a = jobs.start('a').unwrap();
        let a_staging = staging_path(&chosen, 1, a);
        std::fs::write(&a_staging, b"%PDF- the new one").unwrap();
        jobs.printing(a, a_staging.clone(), chosen.clone());
        let steps = jobs.printed(a, Ok(()));
        run_steps(steps, &gate, &tx);
        // Window b: still printing (a staging file half-written). Window c: timed out.
        let other = temp.path().join("Other.pdf");
        std::fs::write(&other, b"%PDF- untouched").unwrap();
        let b = jobs.start('b').unwrap();
        let b_staging = staging_path(&other, 1, b);
        std::fs::write(&b_staging, b"%PDF- half").unwrap();
        jobs.printing(b, b_staging.clone(), other.clone());
        let c = jobs.start('c').unwrap();
        let c_staging = staging_path(&other, 1, c);
        std::fs::write(&c_staging, b"%PDF- late").unwrap();
        jobs.printing(c, c_staging.clone(), other.clone());
        jobs.timed_out(c, "slow");

        // Every window closes (the last one too): the work is still owed, nothing done yet.
        for window in ['a', 'b', 'c'] {
            let steps = jobs.window_closed(window);
            run_steps(steps, &gate, &tx);
        }
        assert!(jobs.busy(), "Legible would exit with work unsettled");
        assert_eq!(std::fs::read(&chosen).unwrap(), b"%PDF- the old one");

        // The share answers: the commit lands, the staging files go, and only then is it done.
        gate.open();
        settle(&mut jobs, &gate, &tx, &rx);
        assert!(!jobs.busy());
        assert_eq!(std::fs::read(&chosen).unwrap(), b"%PDF- the new one");
        assert_eq!(
            std::fs::read(&other).unwrap(),
            b"%PDF- untouched",
            "cancelled prints keep it"
        );
        for staging in [a_staging, b_staging, c_staging] {
            assert!(!staging.exists(), "{} left", staging.display());
        }
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_staging_file_still_held_is_removed_once_its_holder_lets_go() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join(".Plan.pdf.legible-1-1.tmp");
        // Opened without sharing delete, as a writer still holding it would.
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&staging)
            .unwrap();
        // Held past its bound, it's given up on (and says so)...
        assert!(!remove_staging(&staging, std::time::Duration::ZERO));
        assert!(staging.exists());
        // ...and within it, it keeps trying until its holder lets go.
        let path = staging.clone();
        let remover =
            std::thread::spawn(move || remove_staging(&path, std::time::Duration::from_secs(10)));
        drop(held);
        assert!(remover.join().unwrap());
        assert!(!staging.exists());
        // One already gone is done at once.
        assert!(remove_staging(&staging, std::time::Duration::ZERO));
    }

    #[test]
    fn the_pdf_is_named_after_the_document() {
        assert_eq!(default_name(Path::new(r"C:\Notes\Plan.md")), "Plan.pdf");
        assert_eq!(
            default_name(Path::new(r"C:\Notes\Q3 review.v2.markdown")),
            "Q3 review.v2.pdf"
        );
        assert_eq!(
            default_name(Path::new(r"C:\Notes\Ünïcödé 日本語.md")),
            "Ünïcödé 日本語.pdf"
        );
        assert_eq!(default_name(Path::new("")), "document.pdf");
    }

    #[test]
    fn it_goes_beside_the_document_or_else_in_documents() {
        let docs = Some(PathBuf::from(r"C:\Users\Sam\Documents"));
        let folder = Path::new(r"\\server\share\Team");
        assert_eq!(
            default_folder(Some(folder), |_| true, docs.clone()).as_deref(),
            Some(folder)
        );
        assert_eq!(default_folder(Some(folder), |_| false, docs.clone()), docs);
        assert_eq!(default_folder(None, |_| true, docs.clone()), docs);
        assert_eq!(default_folder(Some(folder), |_| false, None), None);
    }

    #[test]
    fn a_folder_is_writable_only_if_a_file_can_be_made_there_and_none_is_left() {
        let temp = tempfile::tempdir().unwrap();
        assert!(can_write_in(temp.path()));
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            0,
            "the check left a file"
        );
        assert!(!can_write_in(&temp.path().join("missing")));
    }

    #[test]
    fn each_job_prints_to_its_own_file_beside_the_chosen_one() {
        let chosen = Path::new(r"D:\Out\Plan.pdf");
        let a = staging_path(chosen, 42, 1);
        assert_eq!(a, Path::new(r"D:\Out\.Plan.pdf.legible-42-1.tmp"));
        assert_eq!(
            a.parent(),
            chosen.parent(),
            "the same folder, so the same volume"
        );
        assert_ne!(a, staging_path(chosen, 42, 2));
        assert_ne!(a, staging_path(chosen, 43, 1));
        assert_ne!(a, chosen);
    }

    #[test]
    fn letter_where_the_locale_measures_in_inches() {
        assert_eq!(Paper::for_measure("1"), Paper::Letter);
        assert_eq!(Paper::for_measure("0"), Paper::A4);
        assert_eq!(Paper::for_measure(""), Paper::A4);
        let (w, h) = Paper::A4.inches();
        assert!((w - 8.2677).abs() < 1e-3 && (h - 11.6929).abs() < 1e-3);
    }
}
