# Status and comparison

gitty is experimental. It started as a tool for exploring a
staging-focused workflow, and it shows: layouts, keys and internals change
as ideas get worked out, and it has none of the maturity of the
established clients.

For most people the honest advice is to use
[lazygit](https://github.com/jesseduffield/lazygit) or
[gitui](https://github.com/gitui-org/gitui). They are mature, widely used,
and cover far more than staging: history, branches, stashes, rebases and
remotes. gitty does none of that.

What gitty offers instead is one workflow done to the exclusion of
everything else:

- **The whole screen is the staging area.** In the split layout the files,
  the unstaged changes and the staged changes are visible side by side, and
  panes with nothing to show collapse away.
- **Staging matches `git add -p` muscle memory.** Hunks stage with a key,
  lines stage through a visual selection (`v`), and discards are confirmed
  before anything is reverted.
- **Runs anywhere git runs.** Everything is pure Rust on top of gitoxide,
  so gitty needs no `git` binary and compiles no C code (lazygit shells out
  to git, gitui builds libgit2).

If you are comfortable with experimental software and curious about this
workflow, gitty is worth a try. If you need something dependable, or
anything beyond staging, use lazygit or gitui.
