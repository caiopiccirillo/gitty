//! Background diff computation so the UI thread never blocks on
//! repository I/O.
//!
//! The worker recomputes both diffs on an interval and pushes a snapshot
//! only when something actually changed. Snapshots are stamped with the
//! mutation epoch they were started at, so the app can discard results
//! that went stale while the user was staging.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;

use crate::diff::DiffView;
use crate::git;

/// How often the worker recomputes the diffs.
pub const WORKER_INTERVAL: Duration = Duration::from_secs(1);

/// One computed snapshot, stamped with the epoch at computation start.
pub struct RefreshOutcome {
    pub epoch: u64,
    pub unstaged: DiffView,
    pub staged: DiffView,
}

/// Spawn the background worker. The returned channel yields a snapshot
/// whenever the diffs change; the worker exits once the app is dropped.
pub fn spawn(repo_path: PathBuf, epoch: Arc<AtomicU64>) -> Receiver<RefreshOutcome> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || work_loop(&repo_path, &epoch, &tx));
    rx
}

fn work_loop(repo_path: &Path, epoch: &AtomicU64, tx: &Sender<RefreshOutcome>) {
    let mut last: Option<(DiffView, DiffView)> = None;
    loop {
        let started_at = epoch.load(Ordering::SeqCst);
        // On error (repo briefly locked, mid-operation) the next tick
        // retries; `last` is kept so recovery sends the new state.
        if let Ok((unstaged, staged)) = load(repo_path) {
            let changed = match &last {
                Some((u, s)) => *u != unstaged || *s != staged,
                None => true,
            };
            if changed {
                last = Some((unstaged.clone(), staged.clone()));
                let outcome = RefreshOutcome {
                    epoch: started_at,
                    unstaged,
                    staged,
                };
                if tx.send(outcome).is_err() {
                    return; // app dropped
                }
            }
        }
        thread::sleep(WORKER_INTERVAL);
    }
}

fn load(repo_path: &std::path::Path) -> Result<(DiffView, DiffView)> {
    let unstaged = git::load_unstaged_diff(repo_path)?;
    let staged = git::load_staged_diff(repo_path)?;
    Ok((unstaged, staged))
}
