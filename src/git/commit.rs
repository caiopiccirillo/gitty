//! Creating commits from the index.

use std::path::Path;

use anyhow::{Context, Result};
use gix::bstr::BString;
use gix::index::entry::Mode;
use gix::object::tree::EntryKind;

use super::open_repo;
use super::staging::owned_index;

/// Commit the index on top of HEAD (or as the root commit on an unborn
/// branch) with the user's configured identity. Returns the short id.
///
/// # Errors
/// Returns an error if the repository cannot be opened, the git identity is not configured, or the commit cannot be written.
pub fn commit(path: &Path, message: &str) -> Result<String> {
    let repo = open_repo(path)?;
    let index = owned_index(&repo)?;
    let tree = index_to_tree(&repo, &index)?;
    let sig = repo
        .committer()
        .context("git identity not configured (user.name/user.email)")??;
    let parents = match repo.head_commit() {
        Ok(commit) => vec![commit.id],
        Err(_) if repo.head().is_ok_and(|head| head.is_unborn()) => Vec::new(),
        Err(err) => return Err(err.into()),
    };
    let reference = match repo.head_name()? {
        Some(name) => name,
        None => gix::refs::FullName::try_from("HEAD")?,
    };
    let id = repo.commit_as(sig, sig, reference, message, tree, parents)?;
    let id = id.to_string();
    Ok(id.get(..7).unwrap_or(&id).to_string())
}

/// Write the index as a tree object and return its id.
fn index_to_tree(repo: &gix::Repository, index: &gix::index::State) -> Result<gix::ObjectId> {
    fn build_tree(
        repo: &gix::Repository,
        paths: &[(&[u8], gix::ObjectId, Mode)],
    ) -> Result<gix::ObjectId> {
        type DirGroup<'a> = (&'a [u8], Vec<(&'a [u8], gix::ObjectId, Mode)>);
        let mut files = Vec::new();
        let mut dirs: Vec<DirGroup<'_>> = Vec::new();
        for (path, id, mode) in paths {
            match path.iter().position(|&b| b == b'/') {
                None => files.push((*path, *id, *mode)),
                Some(idx) => {
                    let (head, rest) = path.split_at(idx);
                    let rest = &rest[1..];
                    match dirs.iter_mut().find(|(name, _)| *name == head) {
                        Some((_, group)) => group.push((rest, *id, *mode)),
                        None => dirs.push((head, vec![(rest, *id, *mode)])),
                    }
                }
            }
        }

        let mut entries = Vec::new();
        for (name, id, mode) in files {
            entries.push(gix::objs::tree::Entry {
                mode: mode.to_tree_entry_mode().with_context(|| {
                    format!(
                        "unsupported index mode for {}",
                        String::from_utf8_lossy(name)
                    )
                })?,
                filename: BString::from(name),
                oid: id,
            });
        }
        for (name, group) in dirs {
            entries.push(gix::objs::tree::Entry {
                mode: EntryKind::Tree.into(),
                filename: BString::from(name),
                oid: build_tree(repo, &group)?,
            });
        }
        entries.sort();
        Ok(repo.write_object(gix::objs::Tree { entries })?.detach())
    }

    let paths: Vec<(&[u8], gix::ObjectId, Mode)> = index
        .entries()
        .iter()
        .map(|e| (e.path(index).as_ref(), e.id, e.mode))
        .collect();
    build_tree(repo, &paths)
}
