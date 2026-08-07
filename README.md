# Git GUI

A small, fast commit-crafting GUI in the spirit of the `git gui` that ships with
Git for Windows — four panes, no ceremony, and **staging or unstaging individual
lines** as a first-class operation.

It also browses history and searches diffs. Runs natively on macOS and Windows
from one codebase (Rust + [egui]). One self-contained binary, no runtime to
install.

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
context — from Finder, the Dock, or Launchpad — it shows the welcome screen with
your recent repositories. You can also point it at one:

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
| Select a range | shift-click, or click and drag |
| Add/remove one line | ⌘-click (Ctrl-click on Windows) |
| Select a whole hunk | click the `@@ … @@` header |
| Select every change | ⌘A |
| Clear the selection | Esc |
| Stage / unstage the selection | Space or Enter |
| Stage / unstage the hunk at the cursor | H, or double-click |
| Discard the selection (unstaged pane) | Backspace |
| Undo the last discard | ⌘Z |
| Move / extend the cursor | ↑ ↓ (or J/K), Shift+↑ ↓ |

Whether Space stages or unstages depends on which pane you are looking at —
viewing unstaged changes it stages, viewing staged changes it unstages. Right
click for the same actions as a menu.

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

Other bits: `Rescan` (⌘R) re-reads status, `Stage Changed` is `git add -u`,
`Sign Off` appends a `Signed-off-by` trailer, `Amend Last Commit` loads the
previous message, and the Branch menu creates and switches branches.

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

107 tests: diff parsing, patch construction, porcelain-v2 status parsing, commit
log and name-status parsing, end-to-end staging against temporary repositories,
undo round-trips, history browsing (including merge, rename and root commits),
diff search, push against a local bare remote (success, up-to-date, rejected
non-fast-forward, force-with-lease, and every refusal case), `App`-level flows
for every gesture the UI exposes, and headless runs of the real widget tree
across every state (untracked, binary, deleted, conflicted, both panes, history,
find bar, push dialog, modals, welcome screen) to catch panics and egui id
collisions.

Push is tested for real: the suite creates a bare repository, wires it up as a
remote, and pushes to it. No network or credentials involved.

## Not implemented

Deliberate gaps, so you know what you are getting:

- **No fetch, pull or merge.** Push exists and runs on a background thread; the
  same plumbing would carry fetch and pull, but merge also needs conflict
  handling, which this has none of. Use the terminal for those.
- **No credential prompting.** By design — see *Pushing*. A repository that
  needs an interactive username and password will fail rather than hang; set up
  a credential helper or SSH key once and it works.
- **The history browser is read-only.** No cherry-pick, revert, reset,
  checking out a commit, or diffing two arbitrary commits against each other.
- **No stash, no rebase.**
- **No commit graph drawing.** Commits are a flat list, not gitk's branch lanes.
- **Search covers the open diff only**, not the whole repository or history.
  `git grep` and `git log -S` are the tools for that.
- **Conflicted files** can be viewed and staged whole once you have resolved the
  markers in an editor, but there is no merge tool.
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
src/app.rs          state, selection model, search, operations
src/ui/mod.rs       panes, commit box, history browser, menus, modals
src/ui/diff_view.rs the virtualised diff widget, selection gestures, find bar
```

The git layer shells out to `git` rather than linking libgit2. It keeps the
dependency tree small enough to cross-compile cleanly, and it means partial
staging is done by the same `git apply` the command line uses.

[egui]: https://github.com/emilk/egui
