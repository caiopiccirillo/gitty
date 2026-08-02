//! Directory tree view-model for the files pane.
//!
//! The flat list of changed files is grouped by directory into visible
//! rows; collapsed directories hide their children but stay selected-aware
//! (their row shows the recursive file count).

use std::collections::{BTreeMap, HashSet};

use crate::diff::FileInfo;

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
    /// A file, referencing `DiffView::files[file_idx]`.
    File { file_idx: usize, depth: usize },
}

impl Node {
    pub fn is_dir(&self) -> bool {
        matches!(self, Node::Dir { .. })
    }
}

/// The directory portion of a path (`a/b/c.rs` -> `a/b`), `None` at the top.
pub fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

/// Build the visible rows from a file list, honoring collapsed directories.
/// Directories sort before the files of their level; both alphabetically.
pub fn visible_rows(files: &[FileInfo], collapsed: &HashSet<String>) -> Vec<Node> {
    // children[dir] = direct subdirectory paths; own[dir] = file indices
    // directly inside dir. The root is the empty string.
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut own: BTreeMap<&str, Vec<usize>> = BTreeMap::new();

    for (idx, file) in files.iter().enumerate() {
        let parent = parent_dir(&file.path).unwrap_or("");
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
        let rows = visible_rows(&nested_files(), &HashSet::new());
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
        let rows = visible_rows(&nested_files(), &collapsed);
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
        let rows = visible_rows(&nested_files(), &collapsed);
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
}
