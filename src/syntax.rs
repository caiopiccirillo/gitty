//! Tree-sitter syntax highlighting for diff lines.
//!
//! Each line of a diff is parsed on its own with the grammar inferred from
//! the file path. Parsing per line keeps rendering cheap; tokens inside
//! multi-line constructs (block comments, raw strings) are colored per
//! line, which is accurate enough for a diff view.

use ratatui::style::Color;
use tree_sitter::{Language, Parser};

/// The tree-sitter language for a file path, by extension.
pub fn language_of(path: &str) -> Option<Language> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" => Some(tree_sitter_python::LANGUAGE.into()),
        "json" => Some(tree_sitter_json::LANGUAGE.into()),
        _ => None,
    }
}

/// Colored token ranges of one line, as `(start, end)` byte offsets into
/// `line` (which must be the text actually rendered).
pub fn highlight(language: Language, line: &str) -> Vec<(usize, usize, Color)> {
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(line.as_bytes(), None) else {
        return Vec::new();
    };
    let mut tokens = Vec::new();
    let mut cursor = tree.root_node().walk();
    walk_tokens(tree.root_node(), &mut cursor, &mut tokens);
    tokens
        .into_iter()
        .filter_map(|(start, end, kind)| kind_color(kind).map(|color| (start, end, color)))
        .collect()
}

/// Collect every leaf token in byte order. Anonymous tokens (like the `let`
/// keyword) have their source text as their kind.
fn walk_tokens<'t>(
    node: tree_sitter::Node<'t>,
    cursor: &mut tree_sitter::TreeCursor<'t>,
    out: &mut Vec<(usize, usize, &'t str)>,
) {
    if node.child_count() == 0 {
        out.push((node.start_byte(), node.end_byte(), node.kind()));
    }
    if cursor.goto_first_child() {
        loop {
            walk_tokens(cursor.node(), cursor, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// The color for a token kind, matched by kind-name heuristics that hold
/// across the supported grammars.
fn kind_color(kind: &str) -> Option<Color> {
    if kind.contains("comment") {
        Some(Color::DarkGray)
    } else if kind.contains("string") {
        Some(Color::Yellow)
    } else if kind.contains("keyword") || KEYWORDS.contains(&kind) {
        Some(Color::Cyan)
    } else if kind.contains("number") || kind.contains("integer") || kind.contains("float") {
        Some(Color::Magenta)
    } else if kind.contains("type") {
        Some(Color::LightBlue)
    } else {
        None
    }
}

/// Anonymous keyword tokens, whose kind is their source text.
const KEYWORDS: &[&str] = &[
    "let", "if", "else", "fn", "return", "while", "for", "in", "match", "use", "mod", "impl",
    "trait", "struct", "enum", "async", "await", "loop", "break", "continue", "move", "mut",
    "pub", "true", "false", "self", "def", "class", "import", "from", "lambda", "pass", "elif",
    "not", "and", "or", "is", "yield", "with", "as", "try", "except", "finally", "global",
    "nonlocal", "del", "raise", "assert", "None", "True", "False",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages_by_extension() {
        assert!(language_of("src/main.rs").is_some());
        assert!(language_of("setup.py").is_some());
        assert!(language_of("data.json").is_some());
        assert!(language_of("README.md").is_none());
        assert!(language_of("no_extension").is_none());
    }

    #[test]
    fn highlights_keywords_and_numbers_in_rust() {
        let language = language_of("main.rs").unwrap();
        let line = "let x = 5;";
        let tokens = highlight(language, line);
        assert!(tokens.iter().any(|(_, _, c)| *c == Color::Cyan), "let");
        assert!(tokens.iter().any(|(_, _, c)| *c == Color::Magenta), "5");
        assert!(
            tokens.iter().all(|(start, end, _)| *start < *end && *end <= line.len()),
            "ranges stay within the line"
        );
    }

    #[test]
    fn highlights_comments_and_strings_in_python() {
        let language = language_of("app.py").unwrap();
        let tokens = highlight(language, "x = 'hi'  # TODO: fix");
        assert!(tokens.iter().any(|(_, _, c)| *c == Color::DarkGray), "comment");
        assert!(tokens.iter().any(|(_, _, c)| *c == Color::Yellow), "string");
    }

    #[test]
    fn json_keys_and_strings_are_highlighted() {
        let language = language_of("data.json").unwrap();
        let tokens = highlight(language, r#"{"name": "gitiff"}"#);
        assert!(tokens.iter().any(|(_, _, c)| *c == Color::Yellow), "string values");
    }
}
