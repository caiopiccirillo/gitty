//! Shared helpers for integration tests.

use std::fs;
use std::path::Path;

use git2::{Repository, Signature};

pub const BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

/// Write `name` with `content` and commit it (as a root commit when there is
/// no HEAD yet, otherwise on top of it).
pub fn commit_file(repo: &Repository, dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(name)).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = Signature::now("t", "t@t").unwrap();
    let parent = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| repo.find_commit(oid).unwrap());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parents)
        .unwrap();
}
