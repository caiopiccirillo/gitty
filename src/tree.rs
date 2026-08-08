//! Directory tree view-model for the files pane.
//!
//! The flat list of changed files is grouped by directory into visible
//! rows; collapsed directories hide their children but stay selected-aware
//! (their row shows the recursive file count).
//!
//! Two sources feed the tree: a single side's file list (classic layout)
//! and the merge of both sides (split layout), where every row knows which
//! side(s) it has changes on.

use std::collections::{BTreeMap, HashSet};

use crate::diff::FileInfo;

/// One file of the merged tree: the same path in both diffs, or only in
/// one. The indices point into `DiffView::files` of the respective side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    /// Index into the staged diff's files, if staged.
    pub staged: Option<usize>,
    /// Index into the unstaged diff's files, if unstaged.
    pub unstaged: Option<usize>,
}

/// Merge both diffs' file lists by path, in path order.
#[must_use]
pub fn merge_files(staged: &[FileInfo], unstaged: &[FileInfo]) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = staged
        .iter()
        .enumerate()
        .map(|(i, f)| FileEntry {
            path: f.path.clone(),
            staged: Some(i),
            unstaged: None,
        })
        .collect();
    for (i, file) in unstaged.iter().enumerate() {
        match entries.iter_mut().find(|e| e.path == file.path) {
            Some(entry) => entry.unstaged = Some(i),
            None => entries.push(FileEntry {
                path: file.path.clone(),
                staged: None,
                unstaged: Some(i),
            }),
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// One visible row in the files pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A directory. `file_count` counts all files beneath it, recursively.
    Dir {
        path: String,
        name: String,
        depth: usize,
        collapsed: bool,
        file_count: usize,
    },
    /// A file, referencing the file list the tree was built from:
    /// `DiffView::files[file_idx]` in classic mode, `FileEntry::[file_idx]`
    /// in split mode.
    File { file_idx: usize, depth: usize },
}

impl Node {
    #[must_use]
    pub fn is_dir(&self) -> bool {
        matches!(self, Node::Dir { .. })
    }
}

/// The directory portion of a path (`a/b/c.rs` -> `a/b`), `None` at the top.
#[must_use]
pub fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

/// Build the visible rows from the merged file entries, honoring collapsed
/// directories. Directories sort before the files of their level; both
/// alphabetically. The classic layout feeds this a single-side merge.
#[must_use]
pub fn visible_rows_merged(entries: &[FileEntry], collapsed: &HashSet<String>) -> Vec<Node> {
    build_rows(entries.len(), collapsed, &|i| &entries[i].path)
}

/// Shared row building over `n` files whose paths come from `path_of`.
fn build_rows<'a>(
    n: usize,
    collapsed: &HashSet<String>,
    path_of: &dyn Fn(usize) -> &'a str,
) -> Vec<Node> {
    // children[dir] = direct subdirectory paths; own[dir] = file indices
    // directly inside dir. The root is the empty string.
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut own: BTreeMap<&str, Vec<usize>> = BTreeMap::new();

    for idx in 0..n {
        let path = path_of(idx);
        let parent = parent_dir(path).unwrap_or("");
        own.entry(parent).or_default().push(idx);
        // Register every ancestor directory as a child of its parent.
        let mut ancestor = parent;
        while !ancestor.is_empty() {
            let grandparent = parent_dir(ancestor).unwrap_or("");
            children.entry(grandparent).or_default().push(ancestor);
            ancestor = grandparent;
        }
    }
    for subdirs in children.values_mut() {
        subdirs.sort_unstable();
        subdirs.dedup();
    }

    let mut rows = Vec::new();
    flatten("", 0, collapsed, &children, &own, &mut rows);
    rows
}

fn count_under(
    dir: &str,
    children: &BTreeMap<&str, Vec<&str>>,
    own: &BTreeMap<&str, Vec<usize>>,
) -> usize {
    own.get(dir).map_or(0, Vec::len)
        + children
            .get(dir)
            .into_iter()
            .flatten()
            .map(|sub| count_under(sub, children, own))
            .sum::<usize>()
}

fn flatten(
    dir: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    children: &BTreeMap<&str, Vec<&str>>,
    own: &BTreeMap<&str, Vec<usize>>,
    rows: &mut Vec<Node>,
) {
    for subdir in children.get(dir).into_iter().flatten() {
        let is_collapsed = collapsed.contains(*subdir);
        rows.push(Node::Dir {
            path: subdir.to_string(),
            name: subdir.rsplit('/').next().unwrap_or(subdir).to_string(),
            depth,
            collapsed: is_collapsed,
            file_count: count_under(subdir, children, own),
        });
        if !is_collapsed {
            flatten(subdir, depth + 1, collapsed, children, own, rows);
        }
    }
    for &file_idx in own.get(dir).into_iter().flatten() {
        rows.push(Node::File { file_idx, depth });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileStatus;

    fn files(paths: &[&str]) -> Vec<FileInfo> {
        paths
            .iter()
            .map(|p| FileInfo {
                path: p.to_string(),
                status: FileStatus::Modified,
            })
            .collect()
    }

    /// src/app.rs, src/git/{ops,x}.rs, tests/t.rs, a.txt
    fn nested_files() -> Vec<FileInfo> {
        files(&[
            "a.txt",
            "src/app.rs",
            "src/git/ops.rs",
            "src/git/x.rs",
            "tests/t.rs",
        ])
    }

    /// Entries for a single side (the classic layout feeds these to the
    /// merged builder).
    fn side_entries(files: &[FileInfo]) -> Vec<FileEntry> {
        merge_files(&[], files)
    }

    fn dir_paths(rows: &[Node]) -> Vec<&str> {
        rows.iter()
            .filter_map(|n| match n {
                Node::Dir { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn builds_nested_rows_dirs_first() {
        let rows = visible_rows_merged(&side_entries(&nested_files()), &HashSet::new());
        let outline: Vec<(String, usize)> = rows
            .iter()
            .map(|n| match n {
                Node::Dir { name, depth, .. } => (format!("{name}/"), *depth),
                Node::File { file_idx, depth } => (format!("f{file_idx}"), *depth),
            })
            .collect();
        assert_eq!(
            outline,
            vec![
                ("src/".into(), 0),
                ("git/".into(), 1),
                ("f2".into(), 2), // src/git/ops.rs
                ("f3".into(), 2), // src/git/x.rs
                ("f1".into(), 1), // src/app.rs
                ("tests/".into(), 0),
                ("f4".into(), 1), // tests/t.rs
                ("f0".into(), 0), // a.txt
            ]
        );
        // Recursive file counts on directory rows.
        match &rows[0] {
            Node::Dir { file_count, .. } => assert_eq!(*file_count, 3),
            _ => panic!(),
        }
        match &rows[1] {
            Node::Dir { file_count, .. } => assert_eq!(*file_count, 2),
            _ => panic!(),
        }
    }

    #[test]
    fn collapsed_dirs_hide_their_children() {
        let collapsed: HashSet<String> = ["src".to_string()].into_iter().collect();
        let rows = visible_rows_merged(&side_entries(&nested_files()), &collapsed);
        assert_eq!(dir_paths(&rows), vec!["src", "tests"]);
        assert_eq!(
            rows.len(),
            4,
            "src children hidden: src, tests, t.rs, a.txt"
        );
        match &rows[0] {
            Node::Dir {
                collapsed,
                file_count,
                ..
            } => {
                assert!(collapsed);
                assert_eq!(*file_count, 3);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn nested_collapse_hides_only_that_subtree() {
        let collapsed: HashSet<String> = ["src/git".to_string()].into_iter().collect();
        let rows = visible_rows_merged(&side_entries(&nested_files()), &collapsed);
        assert_eq!(dir_paths(&rows), vec!["src", "src/git", "tests"]);
        assert!(!rows.iter().any(|n| matches!(
            n,
            Node::File {
                file_idx: 2 | 3,
                ..
            }
        )));
    }

    #[test]
    fn parent_dir_of_top_level_is_none() {
        assert_eq!(parent_dir("a/b/c.rs"), Some("a/b"));
        assert_eq!(parent_dir("a.rs"), None);
    }

    #[test]
    fn merge_files_joins_sides_by_path() {
        let staged = files(&["b.txt", "c.txt"]);
        let unstaged = files(&["a.txt", "c.txt"]);
        let entries = merge_files(&staged, &unstaged);
        assert_eq!(
            entries,
            vec![
                FileEntry {
                    path: "a.txt".into(),
                    staged: None,
                    unstaged: Some(0)
                },
                FileEntry {
                    path: "b.txt".into(),
                    staged: Some(0),
                    unstaged: None
                },
                FileEntry {
                    path: "c.txt".into(),
                    staged: Some(1),
                    unstaged: Some(1)
                },
            ]
        );
    }

    #[test]
    fn merged_tree_groups_dirs_like_the_single_side_tree() {
        let entries = merge_files(
            &files(&["src/app.rs"]),
            &files(&["src/git/ops.rs", "top.rs"]),
        );
        let rows = visible_rows_merged(&entries, &HashSet::new());
        let outline: Vec<String> = rows
            .iter()
            .map(|n| match n {
                Node::Dir { name, .. } => format!("{name}/"),
                Node::File { file_idx, .. } => entries[*file_idx].path.clone(),
            })
            .collect();
        assert_eq!(
            outline,
            vec!["src/", "git/", "src/git/ops.rs", "src/app.rs", "top.rs"]
        );
    }
}
