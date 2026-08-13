//! Semantic colors of the UI, so every palette decision lives in one
//! place. The TUI sticks to the base 16 ANSI colors for compatibility
//! with 8-color terminals.

use ratatui::style::Color;

// File status badges.
pub const ADDED: Color = Color::Green;
pub const DELETED: Color = Color::Red;
pub const MODIFIED: Color = Color::Yellow;
pub const RENAMED: Color = Color::Blue;
pub const TYPE_CHANGE: Color = Color::Magenta;
pub const UNTRACKED: Color = Color::Cyan;

// Diff line kinds.
pub const FILE_HEADER: Color = Color::Yellow;
pub const HUNK_HEADER: Color = Color::Cyan;
pub const META: Color = Color::DarkGray;

// Panes and selection.
pub const PANE_BORDER_FOCUSED: Color = Color::White;
pub const PANE_BORDER: Color = Color::DarkGray;
pub const SELECTED_BG: Color = Color::DarkGray;
pub const SELECTION_END_BG: Color = Color::Gray;

// Status bar.
pub const STATUS_BG: Color = Color::Gray;
pub const STATUS_FG: Color = Color::Black;
pub const HINT: Color = Color::DarkGray;
pub const ERROR: Color = Color::Red;
pub const DISCARD_BG: Color = Color::Red;

// Empty panes.
pub const EMPTY: Color = Color::DarkGray;

// Two-letter badges of merged files in the split layout.
pub const MERGED_STAGED: Color = Color::Green;
pub const MERGED_UNSTAGED: Color = Color::Red;
