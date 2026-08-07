//! Git operations via gitoxide (`gix` crate).
//!
//! Diffs are computed with `gix`'s status/diff machinery ([`diff`]) and
//! rendered into the flat line model of [`crate::diff`] ([`render`]).
//! Staging and unstaging don't use patch application (gitoxide has none):
//! the new index blob content is rebuilt from the hunk material with
//! [`splice`], then written to the index ([`staging`]). Commits are created
//! from the index in [`commit`].
//!
//! The public functions below are the whole surface the rest of the app
//! uses; everything else in this module is an implementation detail.

mod commit;
mod diff;
mod model;
mod render;
mod splice;
mod staging;

pub use commit::commit;
pub use diff::{load_staged_diff, load_unstaged_diff};
pub use staging::{
    stage_file, stage_hunk, stage_lines, unstage_file, unstage_hunk, unstage_lines,
};

use std::path::Path;

use anyhow::{Context, Result};

pub(super) fn open_repo(path: &Path) -> Result<gix::Repository> {
    gix::discover(path)
        .with_context(|| format!("no git repository found at or above {}", path.display()))
}
