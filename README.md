# Git GUI

A small, fast commit-crafting GUI in the spirit of the `git gui` that ships with
Git for Windows — four panes, no ceremony, and **staging or unstaging individual
lines** as a first-class operation.

It also browses history and stashes, searches diffs, and pushes. Runs natively on
macOS and Windows from one codebase (Rust + [egui]). One self-contained binary,
no runtime to install.

```
┌ Unstaged Changes ─────┬ diff ─────────────────────────────────┐
│  M  README.md         │ @@ -8,3 +9,4 @@ def start(host, port) │
│  ?  notes.txt         │       sock = socket.socket()          │
│  M  server.py         │ ▌+    sock.setsockopt(...)   selected │
├ Staged Changes ───────┤ ▌-    sock.listen(5)         selected │
│  M  server.py         │  +    sock.listen(backlog)            │
├───────────────────────┼ commit message ───────────────────────┤
│                       │ ○ New Commit  ○ Amend    [Commit]     │
└───────────────────────┴───────────────────────────────────────┘
```

## Build and run

Requires a Rust toolchain and `git` on `PATH`.

```bash
cargo run --release
```

Run from inside a checkout it opens that repository. Run with no repository in
context — from Finder, the Dock, Launchpad, or a pinned taskbar shortcut — it
shows the welcome screen with your recent repositories. A working directory that
is simply the binary's own folder does not count as a choice, since that is what
those launchers hand over; otherwise a build sitting in `target/release` would
always reopen the repository it was built from. You can also point it at one:

```bash
cargo run --release -- /path/to/repo
```

To install it as a command:

```bash
cargo install --path .
```

## Working with lines and hunks

Click a file in either pane to see its diff. Then:

| Action | Gesture |
| --- | --- |
| Select one line | click |
| Select a range | shift-click, or drag in the line-number gutter |
| Add/remove one line | ⌘-click (Ctrl-click on Windows) |
| Select a whole hunk | click the `@@ … @@` header |
| Select every change | ⌘A |
| Clear the selection | Esc |
| Stage / unstage the selection | Space or Enter |
| Stage / unstage the hunk at the cursor | H, or double-click |
| Discard the selection (unstaged pane) | Backspace |
| Undo the last discard | ⌘Z |
| Select text | drag across the text |
| Copy the text selection, or the selected lines | ⌘C |
| Open the cursor's line in your editor | E, or double-click the name above the diff |
| Move / extend the cursor | ↑ ↓ (or J/K), Shift+↑ ↓ |

Whether Space stages or unstages depends on which pane you are looking at —
viewing unstaged changes it stages, viewing staged changes it unstages. Right
click for the same actions as a menu.

Selecting text and picking lines are separate gestures, split by where the drag
begins. Dragging over the text highlights characters, as anywhere else; dragging
in the line-number gutter sweeps whole lines for staging. A plain click still
picks a line either way. `⌘C` copies the highlight when there is one and the
selected lines otherwise, and in both cases you get the code alone — no line
numbers, and no `+` or `-` markers to strip out afterwards. That is also why the
marker column itself never highlights: it belongs to the gutter, not the text.

With nothing selected, Space falls back to the hunk under the cursor, so
click-then-Space works without a precise selection.

Discarding lines and hunks takes effect immediately, with no confirmation
dialog. Instead there is an undo: every discard pushes what it removed onto a
stack, and `⌘Z` (or the **↺ Undo discard** button in the status bar) puts it
back. Repeated discards unwind one at a time, up to 50 deep.

Discarding a *whole file* from the file-list context menu still asks first,
because it drops every change at once and deletes untracked files outright — but
it is undoable too. Untracked files are held as raw bytes rather than a patch,
so deleting an untracked binary is recoverable as well.

Undo covers only discards. Staging and unstaging leave your content in the
working tree, so there is nothing to take back.

## Editing a line

Spotting a typo or a formatting slip while reading a diff is common, so `E` (or
**Edit at This Line** in the right-click menu) hands the file to your editor at
the line under the cursor. There is deliberately no editor inside this app: your
own one already has spell-check, formatting and syntax awareness, and a buffer
here would fight the line-staging model, since row and hunk indices shift as
text changes underneath them.

The command is resolved most-specific first:

1. `GITGUI_EDITOR`, or an `editor.txt` file in the settings directory
2. git's `core.editor`
3. `$VISUAL`, then `$EDITOR`
4. failing all of those, whatever the system opens the file with

Line numbers are passed the way each editor expects — `-g file:N` for VS Code,
`-nN` for Notepad++, `+N` for vi-family editors, `--line N` for JetBrains IDEs.
An unrecognised editor is simply handed the file, with no invented flag.

Use the override when `core.editor` is set up for commit messages rather than for
browsing source — flags like Notepad++'s `-notabbar` make sense for a commit
buffer and not much else:

```
GITGUI_EDITOR=code
```

The line is the *new* side's number, which is the working-tree line. A deleted
line has no new-side number, so the nearest one above it is used.

## Finishing a merge

Starting a merge is still a terminal job, but once `git merge` has stopped on
conflicts this app can carry it to the commit. While `MERGE_HEAD` exists the
status bar says so — `merging side — 2 conflict(s) left` — and offers
**Abort Merge**, which asks first because it throws the resolutions away.

Conflicted files show with a `U` in the unstaged pane. Right-click one for:

| Action | What it does |
| --- | --- |
| Use Ours (this branch) | `git checkout --ours`, then stages it |
| Use Theirs (incoming) | `git checkout --theirs`, then stages it |
| Mark Resolved (stage as-is) | stages whatever is on disk, markers fixed by hand |

`E` opens the file in your editor at the cursor's line, which is the quickest way
to deal with the conflict markers themselves.

The commit message is prefilled from git's own `MERGE_MSG`, comment lines
stripped. Commit is refused while any conflict is unresolved, naming the first
one, rather than letting git fail later.

One case worth knowing: resolving every conflict in favour of *ours* leaves the
index identical to `HEAD`, so the staged pane can be completely empty. The commit
is still both necessary and allowed, because it is what records the second
parent — so the usual "nothing staged" guard is lifted during a merge.

A file added on only one side has no `--ours` or `--theirs` version; taking a
side there fails, and the message says to stage or remove it instead.

## Searching a diff

`⌘F` (or the **Find** button) opens an incremental search over the diff on
screen, in either mode.

| Action | Key |
| --- | --- |
| Open the find bar | ⌘F |
| Next / previous match | ⏎ / ⇧⏎, or ⌘G / ⌘⇧G |
| Toggle case sensitivity | the **Aa** checkbox |
| Close | Esc |

Every hit is tinted, the current one more strongly, and the counter reads
`3 of 12`. Context lines are searched too, not just added and removed ones —
often you want to find the unchanged line next to a change.

Searching moves the cursor but deliberately leaves your line selection alone, so
you can hunt for a line and then stage what you had already picked out.

Case folding is done one character at a time, which keeps highlight columns
aligned with the rendered text. Full Unicode lowercasing can change a string's
length and would smear the highlight rectangles.

## Stashes

The **Stashes** tab lists every entry with its message, originating branch and
date, an `+untracked` marker where relevant, and a filter box. Selecting one
shows the files it holds and their diffs; **Apply**, **Pop** and **Drop…** are on
the detail panel and the right-click menu. **Stash…** in the Working Tree
toolbar puts the current changes aside, with an option to include untracked
files.

Stash contents are read-only, like commits.

Dropping asks first *and* is undoable with `⌘Z` — the entry disappears but its
commit does not, so `git stash store` can put it back.

Three things needed care here, each with a test:

- **`%gd` is rendered through `--date`.** Asking for a short date turns
  `stash@{0}` into `stash@{2026-08-07}`, which is not a usable ref. Refs are
  derived from list position instead.
- **`git stash show -p -- <path>` does not work** — it rejects the pathspec as a
  second revision. Per-file diffs come from `git diff <stash>^1 <stash> -- <path>`.
- **Untracked files live in a third parent** and are absent from the stash
  commit's own tree, so no diff against the first parent can show them. They are
  listed from `ls-tree` on that parent and rendered by reading the blob, the same
  way an untracked working-tree file is shown.

A failed apply or pop (a conflict, typically) reports git's message in full and
leaves the entry in place.

## Pushing

**Push…** in the toolbar opens a dialog with the remote, whether to set upstream
(`-u`), whether to include tags, and force-with-lease. The button shows `↑n` when
the branch is ahead. Recent repositories are also under **Repository ▸ Recent**.

This is the one operation that does not run inline on the UI thread. A push can
block on the network or on a credential helper, so it runs on a background
thread; the window stays live and the status bar shows a spinner. git's full
output is shown when it finishes, success or failure — that is where
`Everything up-to-date` or a rejection message appears.

Two deliberate choices:

- **`GIT_TERMINAL_PROMPT=0` is set.** With no terminal attached, a repository
  needing a username and password would otherwise wait forever with the app
  looking frozen. Instead it fails immediately and the error explains how to set
  up a credential helper or an SSH key. Credential helpers and SSH keys already
  in the agent work silently, which is the normal case.
- **Force is always `--force-with-lease`**, never plain `--force`. It refuses if
  the remote moved in a way you have not fetched, so it cannot quietly discard
  someone else's commits.

There is still no fetch, pull or merge — see *Not implemented*.

## Browsing commits

The **History** tab replaces the working-tree panes with a commit browser:
commits at the top left, the selected commit's files below, its diff on the
right, and author, date and full message underneath.

Everything is read-only — staging, unstaging and discarding are inert there,
since a commit that already exists cannot be restaged.

- The filter box narrows by message, author, hash or ref name.
- **All branches** includes commits not reachable from `HEAD` (`git log --all`).
- 200 commits load at a time; **Load more…** doubles that.

Two cases needed care, and there are tests pinning both:

- **Merge commits.** Plain `git show` prints *nothing* for a merge, so the file
  list would silently come up empty. History uses `-m --first-parent`, showing
  the diff against the first parent.
- **Renames.** Asking for just the new path makes git report a rename as a fresh
  add. The pre-rename path is passed alongside it so `--find-renames` can still
  see it, and the file shows as `old → new`.

The one case undo cannot help with: if you edit the file after discarding, the
stored patch no longer applies. It says so and keeps the entry rather than
dropping the content silently.

## Amending

`Amend Last Commit` loads the previous message and re-bases the staged pane on
the commit's parent instead of `HEAD`, so the pane lists everything the
rewritten commit will contain — the changes already in it as well as anything
newly staged on top. Clicking one of those files shows its diff against the
parent, and unstaging reaches the same base: a line, hunk, or whole file taken
out of the pane leaves the amended commit and reappears as unstaged work. The
worktree is never touched. Amending a root commit diffs against the empty tree,
so its entire content is listed.

Other bits: `Rescan` (⌘R) re-reads status, `Stage Changed` is `git add -u`,
`Sign Off` appends a `Signed-off-by` trailer, `Hide Untracked` leaves new,
never-committed files out of the unstaged list so only changes to tracked files
show (remembered between runs), and the Branch menu creates and switches
branches.

## How partial staging works

The interesting part, and the part to be careful with when changing this code.

Staging a subset of lines means synthesising a patch that contains only those
lines and feeding it to `git apply`. Lines the user did *not* select have to be
rewritten, and the rule depends on the direction the patch will be applied,
because the side that must match the target flips:

| | unselected `+` | unselected `-` |
| --- | --- | --- |
| **Forward** — stage (patch applied to the index, old side must match it) | dropped | becomes context |
| **Reverse** — unstage or discard (new side must match the target) | becomes context | dropped |

On top of that:

- Hunk positions are recomputed. The old-side start never moves, but the
  new-side start is `old_start + Σ(new_count − old_count)` over the hunks
  already emitted, so staging only the *second* hunk of a file still lands in
  the right place.
- `@@ -S,0` means "after line S", so a zero count shifts the stated start by
  one. Positions are normalised before the arithmetic and converted back after.
- A `new file mode` / `deleted file mode` header stops being true when only part
  of the change is taken — a partly-unstaged new file has content on its old
  side. Those headers, and the now-meaningless `index` blob hashes, are stripped
  and `/dev/null` is replaced with the real path.
- Diffs are split on `\n` only. `str::lines()` also eats a trailing `\r`, which
  would silently corrupt every patch in a CRLF checkout.
- `\ No newline at end of file` is emitted only when the line it describes
  survived.

`src/git/patch.rs` carries unit tests for each of these; `tests/staging.rs`
proves them against real repositories, since the failure mode that matters
(a patch that applies but stages the wrong line) only shows up against git
itself.

Untracked files have no diff against the index, so one is synthesised as a
single add-everything hunk. That makes line-level staging of a brand-new file
work like anything else, and avoids needing `/dev/null` as a path on Windows.

## Cross-compiling to Windows from macOS

This is the tested path and it produces a single `.exe` that links only stock
Windows system DLLs — no mingw runtime to ship alongside it.

```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

Output: `target/x86_64-pc-windows-gnu/release/gitgui.exe` (~5 MB). The linker is
wired up in `.cargo/config.toml`. Verified as `PE32+ executable (GUI) x86-64`;
the whole test suite also type-checks for that target.

The UCRT stubs it imports (`api-ms-win-crt-*`) are part of Windows 10 and later.

If a future dependency ever breaks the mingw build, `--no-default-features`
drops the native folder picker (the app keeps a path text box), which is the
most likely culprit.

## macOS app bundle

```bash
./scripts/bundle-macos.sh
```

Produces `dist/Git GUI.app`, which you can drag to `/Applications`. Drop an
`icon.icns` next to the script to give it an icon.

## Tests

```bash
cargo test
```

136 tests. Everything is exercised against real temporary repositories — no
mocked git.

- **Parsing**: unified diffs, partial-patch construction, porcelain-v2 status,
  commit log and name-status, stash list.
- **Staging**: lines, hunks, replacement pairs, deletions, untracked files, CRLF,
  files with no trailing newline, and undo round-trips.
- **History**: merge, rename and root commits; diff search.
- **Stashes**: tracked-only, `-u` with untracked files from the third parent,
  apply, pop, conflicting pop, drop with undo, and every refusal case.
- **Push**: against a bare repository created by the suite — success,
  up-to-date, rejected non-fast-forward, force-with-lease. No network or
  credentials involved.
- **Clicking** (`tests/clicking.rs`): synthetic pointer events through the real
  widget tree. These exist because the state-level tests all called
  `select_file()` directly and so could not see a bug where list rows were only
  clickable on their text. One test clicks far to the right of a short filename
  specifically to guard that.
- **Rendering**: headless runs across every state — untracked, binary, deleted,
  conflicted, both panes, history, stashes, find bar, push dialog, modals,
  welcome screen — to catch panics and egui id collisions.

## Not implemented

Deliberate gaps, so you know what you are getting:

- **No fetch, pull or merge command.** Push exists and runs on a background
  thread; the same plumbing would carry fetch and pull. Starting a merge is still
  a terminal job, but a merge already underway can be finished here - see
  *Finishing a merge*.
- **No credential prompting.** By design — see *Pushing*. A repository that
  needs an interactive username and password will fail rather than hang; set up
  a credential helper or SSH key once and it works.
- **The history browser is read-only.** No cherry-pick, revert, reset,
  checking out a commit, or diffing two arbitrary commits against each other.
- **No rebase, no cherry-pick, no revert.**
- **No partial stashing.** A stash takes the whole working tree; git only offers
  `stash push -p` interactively, which does not fit this UI.
- **No commit graph drawing.** Commits are a flat list, not gitk's branch lanes.
- **Search covers the open diff only**, not the whole repository or history.
  `git grep` and `git log -S` are the tools for that.
- **No visual merge tool.** Conflicts can be resolved by taking a whole side, or
  by fixing the markers in your editor and staging the file; there is no
  three-pane ours/base/theirs view.
- Renames are detected and displayed; line-level unstaging of a rename *with*
  content changes passes the rename header through to `git apply` and mostly
  works, but is the least exercised path here.

## Layout

```
src/git/mod.rs      running git, repo discovery, all commands
src/git/status.rs   porcelain-v2 -z parser
src/git/diff.rs     unified-diff parser, synthetic new-file diffs
src/git/patch.rs    partial-patch construction (the core of this app)
src/git/log.rs      commit log and per-commit file list parsing
src/git/stash.rs    stash entries and their three-parent structure
src/app.rs          state, selection model, search, operations
src/ui/mod.rs       panes, commit box, history browser, menus, modals
src/ui/diff_view.rs the virtualised diff widget, selection gestures, find bar
```

The git layer shells out to `git` rather than linking libgit2. It keeps the
dependency tree small enough to cross-compile cleanly, and it means partial
staging is done by the same `git apply` the command line uses.

[egui]: https://github.com/emilk/egui
