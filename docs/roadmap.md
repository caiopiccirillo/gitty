# Roadmap

This roadmap covers the git commands and workflows a developer uses day to
day, mapped against what gitty already does, what lazygit and gitui offer,
and what the underlying gitoxide library can actually support. Each item
carries an effort estimate: S (a few hours), M (a day or two), L (a week
or more).

Two constraints shape the plan:

- **gitoxide is the limit.** Some features are simply not implementable
  today: `git push`, interactive rebase, `git apply`, cherry-pick, stash
  and hooks have no gitoxide plumbing yet (the relevant crates are
  placeholders). Those are marked `blocked`. Anything else is buildable on
  top of what gitty already has.
- **gitty stays focused.** The plan grows the tool toward a complete local
  workflow (history, branches, stash, reset, conflicts) without turning it
  into lazygit. Things that belong to the shell or to other tools are
  listed as out of scope.

## Feasibility, verified

The feasibility claims below were checked against the installed `gix`
0.86.0 source and crates.io:

- `push`: only URL configuration builders exist, no push implementation.
  Blocked.
- `fetch`: fully implemented in `gix::remote`; the work is UI (progress,
  remote selection), not plumbing.
- Commit log: `Repository::rev_walk` provides the traversal.
- Branch checkout: available via `gix-worktree-state`, which requires
  enabling the `worktree-mutation` feature at build time.
- Merge: three-way blob, tree and commit merge exist in `gix::merge`.
- Blame: early plumbing, expected to be rough.
- Stash, apply, rebase, cherry-pick, hooks, bisect: the corresponding
  crates are unpublished or placeholder (`gix-rebase`, `gix-sequencer` are
  published only as `0.0.0`). Blocked.

## What gitty already covers

- Working tree review: unstaged and staged diffs, untracked files, file
  tree with status badges.
- Staging and unstaging of files, directories, hunks and lines.
- Discarding changes (worktree and index) with confirmation.
- Committing from the TUI (single-line message, no hooks).
- Two layouts, mouse support, optional syntax highlighting, background
  refresh.

## Phase 0: Foundation (do first)

Everything in this phase affects correctness or first impressions, and
nothing depends on the larger phases.

| Item | Why | Effort |
| ---- | --- | ------ |
| CLI arguments: `--help`, `--version`, bad-argument handling | The published binary currently ignores arguments it doesn't understand | S |
| Run git hooks around commits (`pre-commit`, `commit-msg`, `prepare-commit-msg`, `post-commit`) | gitty creates commits without running hooks, which silently breaks hook-based workflows | M |
| Multi-line commit message editor | Commits deserve proper messages, not one line | M |
| `commit --amend` (message and staged changes) | One of the most common corrections | S |
| Commit message template (`commit.template`) | Cheap, pairs with the editor work | S |
| Document the two known edge cases (staged-hunk discard alignment, trailing-newline hunks) | Users should know the limits before relying on them | S |

## Phase 1: History

The biggest gap. gitty can only see the present; a developer's day is
half spent in the past. gitoxide's revwalk and tree diff make this phase
fully feasible.

| Item | Why | Effort |
| ---- | --- | ------ |
| Commit log view with graph | `git log` in the TUI; the base for everything else in this phase | L |
| Show a commit's diff and details (message, author, date) | `git show`, reusing the existing diff pane | M |
| Diff two commits (mark a base, then select) | The lazygit "compare two commits" workflow | M |
| Copy a commit id, check out a commit | Small but constantly needed | S |
| Filter the log (author, message, path) | Search, like `git log --grep` | M |

## Phase 2: Branches and the daily loop

With history visible, branches become actionable.

| Item | Why | Effort |
| ---- | --- | ------ |
| Branches view: list, current marker, create, rename, delete, checkout | `git switch`/`git branch` from the TUI; checkout is feasible via gitoxide's worktree state (needs the `worktree-mutation` build feature) | L |
| Diff a branch against HEAD | See what merging a branch would bring | S |
| Tags: list, create, delete (lightweight) | Part of the release flow | S |
| `git fetch` with progress | gitoxide's fetch implementation is complete; the work is UI and remote selection | M |
| Reflog view | Needed later for undo | M |
| `push` | **blocked**: gitoxide has no push | - |

## Phase 3: Index and worktree surgery

The phase closest to gitty's soul: manipulating what is staged and what is
kept, and surviving merge conflicts.

| Item | Why | Effort |
| ---- | --- | ------ |
| Stash: push, list, apply, pop, drop | The standard "park my work" workflow; implementable by hand on gitoxide (snapshot worktree and index into a commit on `refs/stash`) | L |
| Reset menu: soft, mixed, hard | gitty already has the pieces (discard and unstage); expose the full matrix | M |
| Merge with conflict resolution | gitoxide has three-way blob and tree merge; navigating and resolving conflicts in the TUI is the hard part | L |
| Cherry-pick | **blocked**: no sequencer in gitoxide (`gix-cherry-pick` is unpublished); a manual diff-and-apply version is possible but fragile | L |

## Phase 4: Stretch goals

| Item | Why | Effort |
| ---- | --- | ------ |
| Undo / redo (reflog based, like lazygit) | Safety net for the destructive operations | L |
| Blame view | gitoxide's blame is early plumbing, so expect rough edges | M |
| Worktree creation | gitoxide can read worktrees but not create them yet | L |
| Submodule status in the file tree | gitty currently skips submodules | M |
| Interactive rebase, `git apply`, patch editing | **blocked** by missing gitoxide plumbing | - |
| GPG-signed commits | **blocked**: gitoxide cannot create signatures | - |

## Out of scope

- `git clone`, remotes management, `git push`, PR integrations, git-flow,
  custom command systems, `git bisect` UI, git-lfs, sparse checkouts.
- These belong to lazygit, gitui, or the shell. The roadmap deliberately
  stops where gitty would stop being about the local change workflow.

## Suggested order of work

1. Phase 0, released as 0.4.0 (correctness and CLI polish).
2. Phase 1, released as 0.5.0 (history: log, show, diff commits).
3. Phase 2 as 0.6.0 (branches, tags, fetch).
4. Phase 3 as 0.7.0 (stash, reset, merge conflicts).
5. Phase 4 only if the earlier phases hold up and users ask for it.

Each phase ends with a release and a round of feedback. The tool stays
usable at every step, and the experimental status is revisited as the
foundation phases land.
