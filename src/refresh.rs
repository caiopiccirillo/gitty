//! Background diff computation so the UI thread never blocks on
//! repository I/O.
//!
//! The worker recomputes both diffs on an interval and pushes a snapshot
//! only when something actually changed. Snapshots are stamped with the
//! mutation epoch they were started at, so the app can discard results
//! that went stale while the user was staging. Mutations poke the worker
//! through a channel so the refreshed state arrives immediately instead
//! of after the next interval.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;

use crate::diff::DiffView;
use crate::git;

/// How often the worker recomputes the diffs without being poked.
pub const WORKER_INTERVAL: Duration = Duration::from_secs(1);

/// One computed snapshot, stamped with the epoch at computation start.
pub struct RefreshOutcome {
    pub epoch: u64,
    pub unstaged: DiffView,
    pub staged: DiffView,
}

/// Handle of the background worker: the snapshot channel and the poke
/// channel that wakes it immediately after a mutation.
pub struct RefreshWorker {
    pub rx: Receiver<RefreshOutcome>,
    poke: Sender<()>,
}

impl RefreshWorker {
    /// Ask the worker to recompute the diffs right away.
    pub fn poke(&self) {
        let _ = self.poke.send(());
    }
}

/// Spawn the background worker. The returned channel yields a snapshot
/// whenever the diffs change; the worker exits once the app is dropped.
pub fn spawn(repo_path: PathBuf, epoch: Arc<AtomicU64>) -> RefreshWorker {
    let (poke_tx, poke_rx) = mpsc::channel();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || work_loop(&repo_path, &epoch, &tx, &poke_rx));
    RefreshWorker { rx, poke: poke_tx }
}

fn work_loop(
    repo_path: &Path,
    epoch: &AtomicU64,
    tx: &Sender<RefreshOutcome>,
    poke_rx: &Receiver<()>,
) {
    let mut last: Option<(DiffView, DiffView)> = None;
    // A mutation was requested since the last send; the next snapshot must
    // be sent even if the diffs look identical, stamped with the current
    // epoch so the app accepts it.
    let mut force_send = false;
    loop {
        while poke_rx.try_recv().is_ok() {
            force_send = true;
        }
        let started_at = epoch.load(Ordering::SeqCst);
        // On error (repo briefly locked, mid-operation) the next tick
        // retries; `last` and `force_send` are kept so recovery sends the
        // new state.
        if let Ok((unstaged, staged)) = load(repo_path) {
            let changed = match &last {
                Some((u, s)) => *u != unstaged || *s != staged,
                None => true,
            };
            if changed || force_send {
                last = Some((unstaged.clone(), staged.clone()));
                let outcome = RefreshOutcome {
                    epoch: started_at,
                    unstaged,
                    staged,
                };
                if tx.send(outcome).is_err() {
                    return; // app dropped
                }
                force_send = false;
            }
        }
        // Wait for the next interval or a poke from a mutation.
        if poke_rx.recv_timeout(WORKER_INTERVAL).is_ok() {
            force_send = true;
        }
    }
}

fn load(repo_path: &std::path::Path) -> Result<(DiffView, DiffView)> {
    let unstaged = git::load_unstaged_diff(repo_path)?;
    let staged = git::load_staged_diff(repo_path)?;
    Ok((unstaged, staged))
}
