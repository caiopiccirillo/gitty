# Layouts

gitty has two layouts, toggled with `m`:

## Classic

One diff pane at a time. `Tab` switches between the staged and unstaged
views, and the file tree shows the focused side's changes only.

## Split

The unstaged pane in the middle with the staged pane to its right, sharing
one file tree that merges both sides. Rows carry two-letter badges like
`MM` (staged and unstaged changes) or `??` (untracked).

Panes with nothing to show are hidden entirely, so the split collapses to
`Files | Unstaged` while you stage, and back to `Files | Staged` once
everything is staged.

`Tab` cycles the focus through the visible panes (files first, then the
diff panes left to right), skipping hidden ones. After staging, `Tab`
lands in the staged pane where `u` unstages. Each pane keeps its own
cursor.

In the files pane, `Space` acts like lazygit: stage the selected file or
directory, and press it again to unstage. The `s`/`u`/`d` diff keys act on
the focused pane.
