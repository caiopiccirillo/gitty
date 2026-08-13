# Key bindings

## Global

| Key            | Action                              |
| -------------- | ----------------------------------- |
| `q`, `Ctrl+C`  | Quit                                |
| `?`            | Show or close the help overlay      |
| `Tab`          | Classic: switch the shown side, keeping each side's selection. Split: cycle the focus through the visible panes |
| `c`            | Open the commit message box         |
| `m`            | Toggle between the classic and split layouts |
| `[` / `]`      | Narrow / widen the files pane       |
| `z`            | Undo the last staging or discard    |

## Commit message box

| Key                   | Action                       |
| --------------------- | ---------------------------- |
| `Enter`               | Commit                       |
| `Esc`                 | Cancel                       |
| `←` `→`, `Home`, `End` | Move the text cursor         |
| `Backspace`, `Delete` | Delete backwards / forwards  |
| `Ctrl+A` `Ctrl+E`     | Jump to the start / end      |
| `Ctrl+W`, `Alt+Backspace` | Delete the previous word |
| `Ctrl+K`              | Delete to the end of the line|
| `Ctrl+U`              | Clear the whole message      |

The box title shows how many files the commit will contain.

## Files pane

| Key                     | Action                                   |
| ----------------------- | ---------------------------------------- |
| `j` `k` / `↓` `↑`       | Move the selection                       |
| `g` `G` / `Home` `End`  | Jump to the first / last row             |
| `Enter`                 | Expand or collapse a directory           |
| `Enter` on a file       | Open the file's diff                     |
| `l` / `→`               | Expand a collapsed directory, open a file|
| `h` / `←`               | Collapse a directory, move to its parent |
| `Space`                 | Classic: stage/unstage the selected file or directory. Split: toggle it (stage if it has unstaged changes, otherwise unstage) |
| `d`                     | Discard the selected file or directory (asks for confirmation) |

## Diff pane

| Key                     | Action                                   |
| ----------------------- | ---------------------------------------- |
| `j` `k` / `↓` `↑`       | Move to the next / previous changed line |
| `Ctrl+D` `Ctrl+U`       | Move down / up half a page               |
| `PgDn` `PgUp`           | Move down / up a page                    |
| `g` `G`                 | Jump to the first / last changed line    |
| `n` `p`                 | Jump to the next / previous hunk         |
| `v`                     | Start or end a visual line selection     |
| `s` / `Space`           | Stage the hunk (or the selected lines)   |
| `u` / `Space`           | Unstage the hunk (or the selected lines) |
| `d`                     | Discard the hunk (or the selected lines), with confirmation |
| `h` / `←`               | Back to the files pane                   |
| `Esc`                   | Cancel the selection, then back to files |

The cursor moves between changed lines only (`+`/`-`). A visual selection
(`v`) cannot leave its hunk, so it always maps to a single, well-formed
patch.
