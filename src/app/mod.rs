//! Application state: the two diffs, the files pane tree, the diff cursors
//! and the staging actions.
//!
//! Navigation is a three-level hierarchy: the file tree (left pane), the
//! hunks of the selected entry, and a per-line cursor inside the diff
//! (right pane). The hunk under the cursor is the target of stage/unstage.
//! Key handling and cursor movement live in [`input`].
//!
//! Two layouts are supported: the classic single diff pane (the focused
//! side only) and a lazygit-style split with the staged and unstaged panes
//! side by side. Each side keeps its own cursor state ([`PaneState`]) so
//! moving the focus never loses your place.

mod input;

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use anyhow::Result;
use ratatui::widgets::ListState;

use crate::diff::{DiffLine, DiffView, FileInfo, HunkId, LineKind, SelectedLines};
use crate::git;
use crate::refresh::{self, RefreshOutcome};
use crate::tree::{self, FileEntry, Node};

/// Which side of the staging area is focused (and, in the classic layout,
/// which one is shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Unstaged,
    Staged,
}

impl Side {
    /// Index into [`App::panes`].
    pub fn index(self) -> usize {
        match self {
            Side::Unstaged => 0,
            Side::Staged => 1,
        }
    }

    pub fn other(self) -> Side {
        match self {
            Side::Unstaged => Side::Staged,
            Side::Staged => Side::Unstaged,
        }
    }
}

/// The screen layout: the classic single diff pane, or the lazygit-style
/// split with the staged and unstaged panes side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Classic,
    Split,
}

/// Cursor state of one diff pane (staged or unstaged).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneState {
    /// Cursor line within the selected file's displayed diff.
    pub cursor: usize,
    /// Where a visual line selection was started with `v`, if any.
    pub visual_anchor: Option<usize>,
    /// Scroll offset of the diff pane, relative to the displayed lines.
    pub scroll: usize,
}

/// Which pane has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

pub struct App {
    pub unstaged: DiffView,
    pub staged: DiffView,
    /// The focused side: the one shown in the classic layout and the one
    /// the diff keys act on in both layouts.
    pub side: Side,
    pub mode: Mode,
    pub focus: Focus,
    /// Visible rows of the files pane tree (directories + files).
    pub tree: Vec<Node>,
    /// Merged file list the split-mode tree is built from; the file rows of
    /// `tree` index into it.
    pub entries: Vec<FileEntry>,
    /// Directory paths the user has collapsed.
    pub collapsed_dirs: HashSet<String>,
    /// Selected row in the files pane tree.
    pub selected_row: usize,
    /// Scroll state of the files pane; ratatui manages the offset so the
    /// selection stays visible in long lists.
    pub files_state: ListState,
    /// Per-side cursor state of the two diff panes.
    pub panes: [PaneState; 2],
    /// The commit message being typed (`c`), if the commit box is open.
    pub commit_input: Option<CommitInput>,
    /// A destructive discard awaiting confirmation (`d` then `y`).
    pub discard_confirm: Option<DiscardPrompt>,
    /// One-off feedback shown in the status bar (e.g. staging errors).
    pub message: Option<String>,
    viewport_height: usize,
    repo_path: PathBuf,
    pub should_quit: bool,
    /// Bumped on every mutation; background snapshots stamped with an older
    /// epoch are discarded as stale.
    epoch: Arc<AtomicU64>,
    /// Channel of the background refresh worker (`None` in tests).
    refresh_rx: Option<mpsc::Receiver<RefreshOutcome>>,
}

impl App {
    /// Load both diffs of the repository containing `repo_path` and spawn
    /// the background refresh worker.
    pub fn load(repo_path: &Path) -> Result<Self> {
        let mut app = Self::new(
            git::load_unstaged_diff(repo_path)?,
            git::load_staged_diff(repo_path)?,
            repo_path.to_path_buf(),
        );
        app.refresh_rx = Some(refresh::spawn(
            repo_path.to_path_buf(),
            Arc::clone(&app.epoch),
        ));
        Ok(app)
    }

    pub fn new(unstaged: DiffView, staged: DiffView, repo_path: PathBuf) -> Self {
        let mut app = Self {
            unstaged,
            staged,
            side: Side::Unstaged,
            mode: Mode::Classic,
            focus: Focus::Files,
            tree: Vec::new(),
            entries: Vec::new(),
            collapsed_dirs: HashSet::new(),
            selected_row: 0,
            files_state: ListState::default().with_selected(Some(0)),
            panes: [PaneState::default(); 2],
            commit_input: None,
            discard_confirm: None,
            message: None,
            viewport_height: 0,
            repo_path,
            should_quit: false,
            epoch: Arc::new(AtomicU64::new(0)),
            refresh_rx: None,
        };
        app.rebuild_tree();
        app
    }

    pub fn current_diff(&self) -> &DiffView {
        self.diff_of(self.side)
    }

    pub fn diff_of(&self, side: Side) -> &DiffView {
        match side {
            Side::Unstaged => &self.unstaged,
            Side::Staged => &self.staged,
        }
    }

    /// Pane state of the focused side.
    pub fn pane(&self) -> &PaneState {
        &self.panes[self.side.index()]
    }

    pub fn pane_mut(&mut self) -> &mut PaneState {
        &mut self.panes[self.side.index()]
    }

    /// Pane state of `side`, for rendering both panes in the split layout.
    pub fn pane_of(&self, side: Side) -> &PaneState {
        &self.panes[side.index()]
    }

    pub fn pane_of_mut(&mut self, side: Side) -> &mut PaneState {
        &mut self.panes[side.index()]
    }

    /// Cursor of the focused pane.
    pub fn cursor(&self) -> usize {
        self.pane().cursor
    }

    pub fn scroll(&self) -> usize {
        self.pane().scroll
    }

    pub fn visual_anchor(&self) -> Option<usize> {
        self.pane().visual_anchor
    }

    /// Lines shown in the diff pane of the focused side. For a file: its
    /// diff without the `diff --git`/`index`/`---`/`+++` header lines (the
    /// pane title shows the path instead). For a directory: the diffs of
    /// all files beneath it, concatenated, keeping each file's header lines
    /// as separators.
    pub fn display_lines(&self) -> Vec<&DiffLine> {
        self.display_lines_for(self.side)
    }

    /// Like [`display_lines`](Self::display_lines), but for a specific side
    /// (the split layout renders both).
    pub fn display_lines_for(&self, side: Side) -> Vec<&DiffLine> {
        let diff = self.diff_of(side);
        match self.selected_node() {
            Some(&Node::File { .. }) => self
                .selected_file_index_in(side)
                .map(|idx| file_display_lines(diff, idx))
                .unwrap_or_default(),
            Some(Node::Dir { path, .. }) => {
                let mut lines = Vec::new();
                for idx in self.dir_file_indices(side, path) {
                    if let Some(range) = diff.file_line_range(idx) {
                        lines.extend(diff.lines[range].iter());
                    }
                }
                lines
            }
            None => Vec::new(),
        }
    }

    /// The currently selected row of the files pane tree.
    pub fn selected_node(&self) -> Option<&Node> {
        self.tree.get(self.selected_row)
    }

    /// The index of the selected row in the diff of `side`, if that side
    /// has changes for it. The tree always indexes into [`App::entries`]:
    /// in the classic layout the entries are built from the focused side
    /// only, so the other side resolves to `None`.
    pub fn selected_file_index_in(&self, side: Side) -> Option<usize> {
        let &Node::File { file_idx, .. } = self.selected_node()? else {
            return None;
        };
        let entry = &self.entries[file_idx];
        match side {
            Side::Staged => entry.staged,
            Side::Unstaged => entry.unstaged,
        }
    }

    /// Indices of all files beneath a directory on one side, recursively.
    fn dir_file_indices(&self, side: Side, dir: &str) -> Vec<usize> {
        let prefix = format!("{dir}/");
        self.diff_of(side)
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.path.starts_with(&prefix))
            .map(|(i, _)| i)
            .collect()
    }

    /// All files beneath a directory on one side, recursively.
    fn dir_files(&self, side: Side, dir: &str) -> Vec<FileInfo> {
        self.dir_file_indices(side, dir)
            .into_iter()
            .filter_map(|i| self.diff_of(side).files.get(i).cloned())
            .collect()
    }

    /// Row index of a directory in the tree, if visible.
    fn dir_row(&self, path: &str) -> Option<usize> {
        self.tree
            .iter()
            .position(|n| matches!(n, Node::Dir { path: p, .. } if p == path))
    }

    /// Rebuild the files pane tree and keep the selection valid. The tree
    /// always indexes the merged [`FileEntry`] list; in the classic layout
    /// the entries are built from the focused side only.
    fn rebuild_tree(&mut self) {
        self.entries = match self.mode {
            Mode::Classic => match self.side {
                Side::Unstaged => tree::merge_files(&[], &self.unstaged.files),
                Side::Staged => tree::merge_files(&self.staged.files, &[]),
            },
            Mode::Split => tree::merge_files(&self.staged.files, &self.unstaged.files),
        };
        self.tree = tree::visible_rows_merged(&self.entries, &self.collapsed_dirs);
        self.selected_row = if self.tree.is_empty() {
            0
        } else {
            self.selected_row.min(self.tree.len() - 1)
        };
        self.files_state.select(Some(self.selected_row));
    }

    /// Reset both panes' cursor state.
    fn reset_panes(&mut self) {
        self.panes = [PaneState::default(); 2];
    }

    /// Switch between the classic single-pane layout and the split layout.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Classic => Mode::Split,
            Mode::Split => Mode::Classic,
        };
        self.focus = Focus::Files;
        self.selected_row = 0;
        self.reset_panes();
        self.rebuild_tree();
        self.snap_to_first_change();
    }

    /// Hunk under the cursor — the target of stage/unstage.
    pub fn current_hunk(&self) -> Option<HunkId> {
        let lines = self.display_lines();
        let line = lines.get(self.pane().cursor)?;
        line.hunk_idx.map(|hunk_idx| HunkId {
            file_idx: line.file_idx,
            hunk_idx,
        })
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
        for side in [Side::Unstaged, Side::Staged] {
            self.clamp_cursor_for(side);
        }
    }

    /// Stage the hunk under the cursor (unstaged side).
    pub fn stage_selected_hunk(&mut self) {
        self.with_hunk("stage", |repo_path, hunk| {
            git::stage_hunk(repo_path, hunk.file_idx, hunk.hunk_idx)
        });
    }

    /// Unstage the hunk under the cursor (staged side).
    pub fn unstage_selected_hunk(&mut self) {
        self.with_hunk("unstage", |repo_path, hunk| {
            git::unstage_hunk(repo_path, hunk.file_idx, hunk.hunk_idx)
        });
    }

    /// Stage the whole selected file (files pane, unstaged side).
    pub fn stage_selected_file(&mut self) {
        self.stage_file_in(self.side);
    }

    /// Stage the whole selected file as seen on `side`.
    pub fn stage_file_in(&mut self, side: Side) {
        self.with_file_in(side, "stage", git::stage_file);
    }

    /// Unstage the whole selected file (files pane, staged side).
    pub fn unstage_selected_file(&mut self) {
        self.unstage_file_in(self.side);
    }

    /// Unstage the whole selected file as seen on `side`.
    pub fn unstage_file_in(&mut self, side: Side) {
        self.with_file_in(side, "unstage", git::unstage_file);
    }

    fn with_file_in(&mut self, side: Side, verb: &str, op: impl FnOnce(&Path, &FileInfo) -> Result<()>) {
        let Some(file) = self
            .selected_file_index_in(side)
            .and_then(|i| self.diff_of(side).files.get(i))
            .cloned()
        else {
            return;
        };
        self.run_op(verb, |repo_path| op(repo_path, &file));
    }

    /// Stage all files beneath the selected directory (files pane, unstaged side).
    pub fn stage_selected_dir(&mut self) {
        self.stage_dir_in(self.side);
    }

    /// Stage all files beneath the selected directory as seen on `side`.
    pub fn stage_dir_in(&mut self, side: Side) {
        if let Some(Node::Dir { path, .. }) = self.selected_node().cloned() {
            self.with_dir_in(side, "stage", &path, git::stage_file);
        }
    }

    /// Unstage all files beneath the selected directory (files pane, staged side).
    pub fn unstage_selected_dir(&mut self) {
        self.unstage_dir_in(self.side);
    }

    /// Unstage all files beneath the selected directory as seen on `side`.
    pub fn unstage_dir_in(&mut self, side: Side) {
        if let Some(Node::Dir { path, .. }) = self.selected_node().cloned() {
            self.with_dir_in(side, "unstage", &path, git::unstage_file);
        }
    }

    fn with_dir_in(
        &mut self,
        side: Side,
        verb: &str,
        dir: &str,
        op: impl Fn(&Path, &FileInfo) -> Result<()>,
    ) {
        let files = self.dir_files(side, dir);
        self.run_op(verb, |repo_path| files.iter().try_for_each(|f| op(repo_path, f)));
    }

    /// Range of display lines covered by the visual selection, if active.
    pub fn selection_range(&self) -> Option<Range<usize>> {
        self.selection_range_for(self.side)
    }

    /// Like [`selection_range`](Self::selection_range), but for a specific
    /// pane.
    pub fn selection_range_for(&self, side: Side) -> Option<Range<usize>> {
        let pane = self.pane_of(side);
        let anchor = pane.visual_anchor?;
        Some(anchor.min(pane.cursor)..anchor.max(pane.cursor) + 1)
    }

    /// The changed (`+`/`-`) lines covered by the visual selection, as
    /// per-hunk ordinals consumed by git::stage_lines/unstage_lines.
    fn selected_lines(&self) -> Option<(HunkId, SelectedLines)> {
        let range = self.selection_range()?;
        let anchor = self.pane().visual_anchor?;
        let lines = self.display_lines();
        let hunk = HunkId {
            file_idx: lines.get(anchor)?.file_idx,
            hunk_idx: lines.get(anchor)?.hunk_idx?,
        };
        let mut selected = SelectedLines::default();
        let (mut adds, mut dels) = (0, 0);
        for i in self.hunk_bounds_at(anchor) {
            match lines[i].kind {
                LineKind::Addition => {
                    if range.contains(&i) {
                        selected.additions.insert(adds);
                    }
                    adds += 1;
                }
                LineKind::Deletion => {
                    if range.contains(&i) {
                        selected.deletions.insert(dels);
                    }
                    dels += 1;
                }
                _ => {}
            }
        }
        (!selected.is_empty()).then_some((hunk, selected))
    }

    /// Stage only the visually selected lines (unstaged side).
    pub fn stage_selected_lines(&mut self) {
        let Some((hunk, selected)) = self.selected_lines() else {
            self.message = Some("no changed lines selected".into());
            return;
        };
        self.run_op("stage", |repo_path| {
            git::stage_lines(repo_path, hunk.file_idx, hunk.hunk_idx, &selected)
        });
    }

    /// Unstage only the visually selected lines (staged side).
    pub fn unstage_selected_lines(&mut self) {
        let Some((hunk, selected)) = self.selected_lines() else {
            self.message = Some("no changed lines selected".into());
            return;
        };
        self.run_op("unstage", |repo_path| {
            git::unstage_lines(repo_path, hunk.file_idx, hunk.hunk_idx, &selected)
        });
    }

    /// Open the discard prompt for what's under the cursor (`d`).
    pub fn prompt_discard(&mut self) {
        let side = self.side;
        let action = match self.focus {
            Focus::Files => match self.selected_node().cloned() {
                Some(Node::File { .. }) => {
                    let Some(file) = self
                        .selected_file_index_in(side)
                        .and_then(|i| self.diff_of(side).files.get(i))
                        .cloned()
                    else {
                        self.message = Some("nothing to discard on this side".into());
                        return;
                    };
                    DiscardAction::File { file, side }
                }
                Some(Node::Dir { path, .. }) => DiscardAction::Dir { path, side },
                None => return,
            },
            Focus::Diff => {
                let Some(hunk) = self.current_hunk() else {
                    self.message = Some("no hunk under the cursor".into());
                    return;
                };
                match self.selected_lines() {
                    Some((hunk, selected)) => DiscardAction::Lines { hunk, selected, side },
                    None if self.pane().visual_anchor.is_some() => {
                        self.message = Some("no changed lines selected".into());
                        return;
                    }
                    None => DiscardAction::Hunk { hunk, side },
                }
            }
        };
        let what = match &action {
            DiscardAction::Hunk { hunk, .. } => {
                let path = &self.diff_of(side).files[hunk.file_idx].path;
                format!("hunk {} of {path}", hunk.hunk_idx + 1)
            }
            DiscardAction::Lines { hunk, selected, .. } => {
                let path = &self.diff_of(side).files[hunk.file_idx].path;
                format!(
                    "{} line(s) of hunk {} in {path}",
                    selected.additions.len() + selected.deletions.len(),
                    hunk.hunk_idx + 1
                )
            }
            DiscardAction::File { file, .. } => format!("file {}", file.path),
            DiscardAction::Dir { path, .. } => format!("directory {path}/"),
            DiscardAction::Files { files, .. } => {
                let dir = files.first().and_then(|f| tree::parent_dir(&f.path)).unwrap_or("");
                format!("directory {dir}/")
            }
        };
        self.discard_confirm = Some(DiscardPrompt { what, action });
    }

    /// Run the confirmed discard action.
    fn confirm_discard(&mut self) {
        let Some(prompt) = self.discard_confirm.take() else {
            return;
        };
        // Directories are resolved to their files here, where the diffs are
        // available; the operation itself needs no app state.
        let action = match prompt.action {
            DiscardAction::Dir { path, side } => {
                let files = self.dir_files(side, &path);
                DiscardAction::Files { files, side }
            }
            other => other,
        };
        self.run_op("discard", |repo_path| execute_discard(repo_path, action));
    }

    /// Run a git operation against the repo and refresh, showing failures
    /// in the status bar.
    fn run_op(&mut self, verb: &str, op: impl FnOnce(&Path) -> Result<()>) {
        self.message = match op(&self.repo_path).and_then(|()| self.refresh()) {
            Ok(()) => None,
            Err(e) => Some(format!("{verb} failed: {e}")),
        };
    }

    fn with_hunk(&mut self, verb: &str, op: impl FnOnce(&Path, HunkId) -> Result<()>) {
        let Some(hunk) = self.current_hunk() else {
            self.message = Some("no hunk under the cursor".into());
            return;
        };
        self.run_op(verb, |repo_path| op(repo_path, hunk));
    }

    /// Reload both diffs from disk (synchronously, after a mutation) and
    /// invalidate any background snapshot still in flight.
    pub fn refresh(&mut self) -> Result<()> {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let unstaged = git::load_unstaged_diff(&self.repo_path)?;
        let staged = git::load_staged_diff(&self.repo_path)?;
        self.apply_refreshed(unstaged, staged);
        Ok(())
    }

    /// Apply any finished background snapshot (called on idle ticks; cheap
    /// no-op when the channel is empty).
    pub fn poll_refresh(&mut self) {
        if self.pane().visual_anchor.is_some() {
            return; // apply later, when the selection is done
        }
        let Some(rx) = &self.refresh_rx else {
            return;
        };
        let mut latest = None;
        while let Ok(outcome) = rx.try_recv() {
            latest = Some(outcome);
        }
        if let Some(outcome) = latest
            && outcome.epoch >= self.epoch.load(Ordering::SeqCst)
        {
            self.apply_refreshed(outcome.unstaged, outcome.staged);
        }
    }

    /// Synchronous refresh used by tests and explicit calls.
    pub fn auto_refresh(&mut self) {
        if self.pane().visual_anchor.is_none() {
            let _ = self.refresh();
        }
    }

    /// Open the commit message box (`c`), if there is anything staged.
    pub fn open_commit(&mut self) {
        if self.staged.files.is_empty() {
            self.message = Some("nothing staged".into());
        } else {
            self.commit_input = Some(CommitInput::default());
        }
    }

    /// Commit the staged changes with the typed message and close the box.
    pub fn commit(&mut self) {
        let Some(input) = self.commit_input.take() else {
            return;
        };
        let message = input.text.trim().to_string();
        if message.is_empty() {
            self.message = Some("empty commit message".into());
            return;
        }
        self.message = match git::commit(&self.repo_path, &message)
            .and_then(|short| self.refresh().map(|()| format!("committed {short}")))
        {
            Ok(msg) => Some(msg),
            Err(e) => Some(format!("commit failed: {e}")),
        };
    }

    /// Swap in new diffs, preserving the selection and cursor by path.
    /// No-op when nothing changed.
    fn apply_refreshed(&mut self, unstaged: DiffView, staged: DiffView) {
        if unstaged == self.unstaged && staged == self.staged {
            return;
        }
        let identity = self.selected_node().map(|n| self.identity_of(n));
        let saved = self.panes.map(|pane| (pane.cursor, pane.scroll));
        self.unstaged = unstaged;
        self.staged = staged;
        self.rebuild_tree();
        if self.tree.is_empty() {
            self.focus = Focus::Files;
        }
        for pane in &mut self.panes {
            pane.visual_anchor = None;
        }
        if let Some(row) = identity.and_then(|id| self.find_row(&id)) {
            self.selected_row = row;
            self.files_state.select(Some(row));
            for (pane, (cursor, scroll)) in self.panes.iter_mut().zip(saved) {
                pane.cursor = cursor;
                pane.scroll = scroll;
            }
        } else {
            for pane in &mut self.panes {
                pane.cursor = 0;
                pane.scroll = 0;
            }
        }
        for side in [Side::Unstaged, Side::Staged] {
            self.clamp_cursor_for(side);
        }
    }

    /// Path-based identity of a tree row, so it can be re-found after a
    /// refresh even if file indices shifted.
    fn identity_of(&self, node: &Node) -> NodeIdentity {
        match node {
            Node::Dir { path, .. } => NodeIdentity::Dir(path.clone()),
            Node::File { file_idx, .. } => NodeIdentity::File(self.file_path_at(*file_idx)),
        }
    }

    fn find_row(&self, identity: &NodeIdentity) -> Option<usize> {
        self.tree.iter().position(|node| match (node, identity) {
            (Node::Dir { path, .. }, NodeIdentity::Dir(want)) => path == want,
            (Node::File { file_idx, .. }, NodeIdentity::File(want)) => {
                self.file_path_at(*file_idx) == *want
            }
            _ => false,
        })
    }

    /// The path of a file row (from the merged entries, for both layouts).
    fn file_path_at(&self, file_idx: usize) -> String {
        self.entries[file_idx].path.clone()
    }
}

/// Run the git operations behind a confirmed discard.
fn execute_discard(repo_path: &Path, action: DiscardAction) -> Result<()> {
    match action {
        DiscardAction::Hunk { hunk, side } => {
            if side == Side::Staged {
                git::discard_staged_hunk(repo_path, hunk.file_idx, hunk.hunk_idx)?;
            } else {
                git::discard_hunk(repo_path, hunk.file_idx, hunk.hunk_idx)?;
            }
        }
        DiscardAction::Lines { hunk, selected, side } => {
            if side == Side::Staged {
                git::discard_staged_lines(repo_path, hunk.file_idx, hunk.hunk_idx, &selected)?;
            } else {
                git::discard_lines(repo_path, hunk.file_idx, hunk.hunk_idx, &selected)?;
            }
        }
        DiscardAction::File { file, side } => {
            if side == Side::Staged {
                git::discard_staged_file(repo_path, &file)?;
            } else {
                git::discard_file(repo_path, &file)?;
            }
        }
        DiscardAction::Files { files, side } => {
            for file in files {
                if side == Side::Staged {
                    git::discard_staged_file(repo_path, &file)?;
                } else {
                    git::discard_file(repo_path, &file)?;
                }
            }
        }
        DiscardAction::Dir { .. } => unreachable!("resolved to Files before execution"),
    }
    Ok(())
}

/// Path-based identity of a tree row (see [`App::refresh`]).
enum NodeIdentity {
    Dir(String),
    File(String),
}

/// Text of the commit message box with a byte-offset cursor that always
/// sits on a char boundary.
#[derive(Debug, Default)]
pub struct CommitInput {
    pub text: String,
    pub cursor: usize,
}

/// A destructive discard awaiting confirmation.
#[derive(Debug)]
pub struct DiscardPrompt {
    /// What the status bar should ask about, e.g. "hunk 2 of f.txt".
    pub what: String,
    pub action: DiscardAction,
}

/// What a confirmed discard should revert. The side selects the staged-tab
/// operations, which revert both the worktree and the index to HEAD.
#[derive(Debug)]
pub enum DiscardAction {
    Hunk { hunk: HunkId, side: Side },
    Lines {
        hunk: HunkId,
        selected: SelectedLines,
        side: Side,
    },
    File { file: FileInfo, side: Side },
    Dir { path: String, side: Side },
    /// A directory resolved to its files, ready to discard.
    Files { files: Vec<FileInfo>, side: Side },
}

impl CommitInput {
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    fn left(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.cursor = prev;
        }
    }

    fn right(&mut self) {
        self.cursor = self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| self.cursor + c.len_utf8());
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
    }

    /// Cursor position in characters (for rendering).
    pub fn cursor_chars(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }
}

/// A file's diff lines without its file-header lines.
fn file_display_lines(diff: &DiffView, file_idx: usize) -> Vec<&DiffLine> {
    let Some(range) = diff.file_line_range(file_idx) else {
        return Vec::new();
    };
    let lines = &diff.lines[range];
    let headers = lines
        .iter()
        .take_while(|l| l.kind == LineKind::FileHeader)
        .count();
    lines[headers..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileStatus, two_file_view};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Two files (1 hunk + 2 hunks), staged side empty.
    fn test_app() -> App {
        let mut app = App::new(
            two_file_view(),
            DiffView::default(),
            PathBuf::from("/unused"),
        );
        app.set_viewport_height(10);
        app
    }

    /// A staged view with one added file, for the split-layout tests.
    fn staged_view() -> DiffView {
        let line = |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>| DiffLine {
            kind,
            content: String::new(),
            file_idx,
            hunk_idx,
        };
        DiffView {
            lines: vec![
                line(LineKind::FileHeader, 0, None),
                line(LineKind::HunkHeader, 0, Some(0)),
                line(LineKind::Addition, 0, Some(0)),
            ],
            files: vec![FileInfo {
                path: "x.txt".into(),
                status: FileStatus::Added,
            }],
        }
    }

    /// Classic app with both sides populated, switched to split mode.
    fn split_app() -> App {
        let mut app = App::new(
            two_file_view(),
            staged_view(),
            PathBuf::from("/unused"),
        );
        app.set_viewport_height(10);
        app.toggle_mode();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn hunk(file_idx: usize, hunk_idx: usize) -> Option<HunkId> {
        Some(HunkId { file_idx, hunk_idx })
    }

    #[test]
    fn row_selection_moves_and_clamps() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_row, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_row, 1, "clamped at last row");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_row, 0);
    }

    #[test]
    fn enter_focuses_diff_and_cursor_moves_within_file() {
        let mut app = test_app();
        assert_eq!(app.focus, Focus::Files);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Diff);
        // File 0 displays 2 lines: the hunk header and one addition.
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor(), 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor(), 1, "clamped at last line");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn n_and_p_jump_between_hunks_of_the_file() {
        let mut app = test_app();
        // File 1 displays: [hunk0 header, deletion, hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.cursor(), 1, "snaps to the first changed line");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor(), 3, "lands on the next hunk's changed line");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor(), 3, "no next hunk");
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.cursor(), 1);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.cursor(), 1, "no previous hunk");
    }

    #[test]
    fn cursor_defines_the_current_hunk() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.current_hunk(), hunk(1, 0));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.current_hunk(), hunk(1, 1));
    }

    #[test]
    fn tab_switches_and_resets() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.side, Side::Staged);
        assert_eq!(app.selected_row, 0);
        assert_eq!(app.focus, Focus::Files);
        // Staged side is empty: Enter must not focus the diff.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(app.current_hunk(), None);
    }

    #[test]
    fn cursor_traverses_changed_lines_freely() {
        let mut app = test_app();
        // File 1 displays: [hunk0 header, deletion, hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.cursor(), 1, "snaps to the first changed line");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor(), 3, "j jumps to the next changed line");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor(), 3, "clamped at the last changed line");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn scroll_follows_the_cursor() {
        let mut app = test_app();
        app.set_viewport_height(1);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.cursor(), 3);
        assert_eq!(app.scroll(), 3);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.cursor(), 1);
        assert_eq!(app.scroll(), 1);
    }

    #[test]
    fn visual_selection_extends_and_stays_in_the_hunk() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.selection_range(), Some(1..2));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.selection_range(),
            Some(1..2),
            "the hunk has only one changed line"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visual_anchor(), None);
        assert_eq!(app.focus, Focus::Diff, "Esc only cancels the selection");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn discard_requires_confirmation() {
        let mut app = test_app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('d'));
        assert!(app.discard_confirm.is_some());

        // n cancels without touching anything.
        press(&mut app, KeyCode::Char('n'));
        assert!(app.discard_confirm.is_none());

        // y runs the operation (which fails here: /unused is not a repo).
        press(&mut app, KeyCode::Char('d'));
        assert!(app.discard_confirm.is_some());
        press(&mut app, KeyCode::Char('y'));
        assert!(app.discard_confirm.is_none());
        assert!(
            app.message
                .as_deref()
                .is_some_and(|m| m.starts_with("discard failed"))
        );
    }

    #[test]
    fn files_pane_discard_targets_the_selected_file() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('d'));
        let prompt = app.discard_confirm.take().unwrap();
        assert!(matches!(
            prompt.action,
            DiscardAction::File {
                file: ref f,
                side: Side::Unstaged
            } if f.path == "a.txt"
        ));
        assert_eq!(prompt.what, "file a.txt");
    }

    #[test]
    fn visual_selection_discards_selected_lines() {
        let mut app = test_app();
        // File 1: [@@, deletion, @@, deletion]; select the first deletion.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('d'));
        let prompt = app.discard_confirm.take().unwrap();
        assert!(matches!(
            prompt.action,
            DiscardAction::Lines {
                hunk: HunkId {
                    file_idx: 1,
                    hunk_idx: 0
                },
                side: Side::Unstaged,
                ..
            }
        ));
    }

    #[test]
    fn split_mode_merges_both_sides_into_the_tree() {
        let mut app = split_app();
        let paths: Vec<&str> = app.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "b.txt", "x.txt"]);
        assert_eq!(app.tree.len(), 3);

        // x.txt only exists on the staged side.
        let x = app.entries.iter().position(|e| e.path == "x.txt").unwrap();
        assert_eq!(app.entries[x].staged, Some(0));
        assert_eq!(app.entries[x].unstaged, None);

        // Selecting it shows a diff in the staged pane and nothing in the
        // unstaged one.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_file_index_in(Side::Staged), Some(0));
        assert_eq!(app.selected_file_index_in(Side::Unstaged), None);
        assert_eq!(app.display_lines_for(Side::Staged).len(), 2);
        assert!(app.display_lines_for(Side::Unstaged).is_empty());
    }

    #[test]
    fn split_mode_keeps_per_pane_cursors_when_switching_focus() {
        let mut app = split_app();
        // File a.txt (two hunks) on the unstaged side.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.side, Side::Unstaged);
        press(&mut app, KeyCode::Char('n'));
        let unstaged_cursor = app.cursor();
        assert!(unstaged_cursor > 0);

        // Tab moves the diff focus to the next visible pane.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.side, Side::Staged);
        assert_eq!(app.selected_row, 0, "selection preserved");
        assert_eq!(app.focus, Focus::Diff, "focus stays in the diff");

        // The staged pane has its own cursor (a.txt is not staged: empty).
        assert_eq!(app.cursor(), 0);

        // Tab past the last pane returns to the files pane; the unstaged
        // cursor is untouched.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(
            app.pane_of(Side::Unstaged).cursor,
            unstaged_cursor,
            "unstaged cursor preserved"
        );
    }

    #[test]
    fn tab_in_split_mode_keeps_the_selection() {
        let mut app = split_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_row, 1);
        // From the files pane, Tab enters the first visible diff pane.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.selected_row, 1, "selection kept when moving focus");
        assert_eq!(app.side, Side::Unstaged);
        assert_eq!(app.focus, Focus::Diff);
        // And keeps cycling through the visible panes.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.side, Side::Staged);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn tab_in_split_mode_skips_sides_without_content() {
        let mut app = test_app(); // staged side is empty
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.side, Side::Unstaged, "cannot focus the hidden staged pane");
        press(&mut app, KeyCode::Char('m'));
        assert_eq!(app.mode, Mode::Classic);
    }

    #[test]
    fn split_mode_space_dispatch_stages_or_unstages() {
        let mut app = split_app();
        // x.txt is staged-only: space dispatches an unstage.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        assert!(
            app.message
                .as_deref()
                .is_some_and(|m| m.starts_with("unstage failed"))
        );

        // a.txt is unstaged-only: space dispatches a stage.
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char(' '));
        assert!(
            app.message
                .as_deref()
                .is_some_and(|m| m.starts_with("stage failed"))
        );
    }

    /// src/app.rs, src/git/ops.rs, top.rs — one hunk each.
    fn nested_view() -> DiffView {
        let line = |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>| DiffLine {
            kind,
            content: String::new(),
            file_idx,
            hunk_idx,
        };
        let mut lines = Vec::new();
        for idx in 0..3 {
            lines.push(line(LineKind::FileHeader, idx, None));
            lines.push(line(LineKind::HunkHeader, idx, Some(0)));
            lines.push(line(LineKind::Addition, idx, Some(0)));
        }
        DiffView {
            lines,
            files: ["src/app.rs", "src/git/ops.rs", "top.rs"]
                .into_iter()
                .map(|path| FileInfo {
                    path: path.into(),
                    status: FileStatus::Modified,
                })
                .collect(),
        }
    }

    fn nested_app() -> App {
        // Rows (dirs first): [Dir src, Dir src/git, File ops.rs, File app.rs, File top.rs]
        let mut app = App::new(nested_view(), DiffView::default(), PathBuf::from("/unused"));
        app.set_viewport_height(10);
        app
    }

    #[test]
    fn enter_on_a_dir_collapses_and_expands_it() {
        let mut app = nested_app();
        assert_eq!(app.tree.len(), 5);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tree.len(), 2, "collapsed to [Dir src, File top.rs]");
        assert_eq!(app.selected_row, 0, "the dir stays selected");
        assert!(app.collapsed_dirs.contains("src"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tree.len(), 5);
    }

    #[test]
    fn h_on_a_file_moves_to_its_parent_dir() {
        let mut app = nested_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert!(matches!(app.selected_node(), Some(Node::File { .. })));
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.selected_row, 1, "parent dir src/git");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.tree.len(), 4, "h on the expanded dir collapses it");
    }

    #[test]
    fn h_collapses_an_expanded_dir_and_l_expands_it_back() {
        let mut app = nested_app();
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.tree.len(), 2);
        // h on a collapsed root dir has no parent to jump to.
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.selected_row, 0);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tree.len(), 5);
    }

    #[test]
    fn commit_input_edits_text_at_char_boundaries() {
        let mut input = CommitInput::default();
        for c in "héllo".chars() {
            input.insert(c);
        }
        input.left();
        input.left();
        input.left();
        input.backspace();
        assert_eq!(input.text, "hllo", "backspace removed the multibyte é");
        assert_eq!(input.cursor, 1);
        input.insert('e');
        assert_eq!(input.text, "hello");
        for _ in 0..4 {
            input.right();
        }
        input.insert('!');
        assert_eq!(input.text, "hello!");
        input.left();
        input.backspace();
        assert_eq!(input.text, "hell!", "removes the char before the cursor");
    }

    #[test]
    fn dir_selection_shows_the_aggregate_diff() {
        let mut app = nested_app();
        // Dir src aggregates app.rs + git/ops.rs (3 lines each, headers kept).
        assert_eq!(app.display_lines().len(), 6);
        press(&mut app, KeyCode::Char('j'));
        // Dir src/git aggregates ops.rs only.
        assert_eq!(app.display_lines().len(), 3);
        press(&mut app, KeyCode::Char('j'));
        // File row: header lines stripped (hunk header + addition).
        assert_eq!(app.display_lines().len(), 2);
    }
}
