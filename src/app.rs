use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::config;
use crate::git::diff::{self, FileDiff, LineKind};
use crate::git::log::{CommitFile, CommitInfo, CommitMeta};
use crate::git::patch::{self, ApplyMode};
use crate::git::stash::{StashEntry, StashFile};
use crate::git::status::Status;
use crate::git::{PushOptions, Repo};

/// Files larger than this are summarised rather than rendered, so opening a
/// giant blob never wedges the UI.
const MAX_DISPLAY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    WorkingTree,
    History,
    Stashes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Unstaged,
    Staged,
    /// A file inside a historical commit. Read-only.
    Commit,
    /// A file inside a stash. Read-only.
    Stash,
}

impl Pane {
    pub fn read_only(self) -> bool {
        matches!(self, Pane::Commit | Pane::Stash)
    }
}

/// Incremental search over the visible diff.
#[derive(Default)]
pub struct Find {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    pub matches: Vec<FindMatch>,
    pub current: usize,
    /// Set to pull keyboard focus into the field on the next frame.
    pub request_focus: bool,
}

/// A hit, in character columns of the *rendered* row (marker column included).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FindMatch {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// Case folding that keeps one output char per input char, so match offsets
/// stay aligned with the rendered text. Full Unicode lowercasing can change
/// length and would desync the highlight rectangles.
fn fold(s: &str, case_sensitive: bool) -> Vec<char> {
    if case_sensitive {
        s.chars().collect()
    } else {
        s.chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect()
    }
}

fn find_sub(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

#[derive(Clone, PartialEq, Debug)]
pub struct FileSel {
    pub pane: Pane,
    pub path: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Stage,
    Unstage,
    Discard,
}

impl Op {
    pub fn verb(self) -> &'static str {
        match self {
            Op::Stage => "Stage",
            Op::Unstage => "Unstage",
            Op::Discard => "Discard",
        }
    }
}

/// What a row in the diff view represents.
#[derive(Clone, Debug)]
pub enum Row {
    Hunk(usize),
    Line { hunk: usize, line: usize },
    Note(String),
}

/// The currently displayed diff plus its selection state.
pub struct Loaded {
    pub fd: FileDiff,
    pub rows: Vec<Row>,
    /// Row indices the user has selected. Only change rows are ever inserted.
    pub sel: HashSet<usize>,
    pub anchor: Option<usize>,
    pub cursor: usize,
    pub dragging: bool,
    /// Widest row in characters, for horizontal scroll extent.
    pub max_chars: usize,
    pub untracked: bool,
    /// Scroll bookkeeping, so keyboard navigation can keep the cursor in view.
    pub scroll_y: f32,
    pub viewport_h: f32,
    pub scroll_to_cursor: bool,
}

impl Loaded {
    fn build(fd: FileDiff, untracked: bool, note: Option<String>) -> Loaded {
        let mut rows = Vec::new();
        let mut max_chars = 0usize;
        if let Some(n) = note {
            max_chars = max_chars.max(n.chars().count());
            rows.push(Row::Note(n));
        }
        for (hi, h) in fd.hunks.iter().enumerate() {
            max_chars = max_chars.max(h.header().chars().count());
            rows.push(Row::Hunk(hi));
            for (li, l) in h.lines.iter().enumerate() {
                max_chars = max_chars.max(l.text.chars().count() + 1);
                rows.push(Row::Line { hunk: hi, line: li });
            }
        }
        Loaded {
            fd,
            rows,
            sel: HashSet::new(),
            anchor: None,
            cursor: 0,
            dragging: false,
            max_chars,
            untracked,
            scroll_y: 0.0,
            viewport_h: 0.0,
            scroll_to_cursor: false,
        }
    }

    pub fn pair_at(&self, row: usize) -> Option<(usize, usize)> {
        match self.rows.get(row) {
            Some(Row::Line { hunk, line }) => Some((*hunk, *line)),
            _ => None,
        }
    }

    /// True when this row is a selectable change (not context, not a header).
    pub fn is_change_row(&self, row: usize) -> bool {
        self.pair_at(row)
            .and_then(|(h, l)| self.fd.hunks.get(h).and_then(|hk| hk.lines.get(l)))
            .map(|l| l.is_change())
            .unwrap_or(false)
    }

    pub fn hunk_of_row(&self, row: usize) -> Option<usize> {
        match self.rows.get(row) {
            Some(Row::Hunk(h)) => Some(*h),
            Some(Row::Line { hunk, .. }) => Some(*hunk),
            _ => None,
        }
    }

    pub fn select_only(&mut self, row: usize) {
        self.sel.clear();
        if self.is_change_row(row) {
            self.sel.insert(row);
        }
        self.anchor = Some(row);
        self.cursor = row;
    }

    pub fn toggle(&mut self, row: usize) {
        if self.is_change_row(row) {
            if !self.sel.remove(&row) {
                self.sel.insert(row);
            }
        }
        self.anchor = Some(row);
        self.cursor = row;
    }

    /// Replaces the selection with every change row between the anchor and
    /// `row`, inclusive.
    pub fn extend_to(&mut self, row: usize) {
        let a = self.anchor.unwrap_or(row);
        let (lo, hi) = if a <= row { (a, row) } else { (row, a) };
        self.sel.clear();
        for r in lo..=hi.min(self.rows.len().saturating_sub(1)) {
            if self.is_change_row(r) {
                self.sel.insert(r);
            }
        }
        self.cursor = row;
    }

    pub fn select_hunk(&mut self, hunk: usize) {
        self.sel.clear();
        for (i, r) in self.rows.iter().enumerate() {
            if let Row::Line { hunk: h, .. } = r {
                if *h == hunk {
                    self.sel.insert(i);
                }
            }
        }
        self.sel.retain(|r| {
            matches!(self.rows.get(*r), Some(Row::Line { hunk: h, line: l })
                if *h == hunk && self.fd.hunks[*h].lines[*l].is_change())
        });
        if let Some(first) = self.rows.iter().position(|r| matches!(r, Row::Hunk(h) if *h == hunk)) {
            self.anchor = Some(first);
            self.cursor = first;
        }
    }

    pub fn select_all(&mut self) {
        self.sel.clear();
        for i in 0..self.rows.len() {
            if self.is_change_row(i) {
                self.sel.insert(i);
            }
        }
    }

    /// The (hunk, line) pairs the current selection covers.
    pub fn selected_pairs(&self) -> HashSet<(usize, usize)> {
        self.sel.iter().filter_map(|r| self.pair_at(*r)).collect()
    }

    pub fn selected_change_count(&self) -> usize {
        self.sel.iter().filter(|r| self.is_change_row(**r)).count()
    }

    /// Moves the cursor to the next/previous row, optionally extending.
    pub fn move_cursor(&mut self, delta: i64, extend: bool) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i64 - 1;
        let next = (self.cursor as i64 + delta).clamp(0, last) as usize;
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.extend_to(next);
        } else {
            self.select_only(next);
        }
    }
}

/// Line and hunk discards apply immediately; only whole-file discards ask,
/// since those throw away every change at once and can delete an untracked
/// file outright.
pub enum Confirm {
    DiscardFile { path: String, untracked: bool },
    DropStash { refname: String, label: String },
}

/// A git command running off the UI thread. Only network operations need this;
/// status and diffs are fast enough to run inline.
pub struct RunningTask {
    pub label: String,
    rx: Receiver<std::result::Result<String, String>>,
}

/// Outcome of the last push, kept so its output can be shown in full.
pub struct TaskOutcome {
    pub ok: bool,
    pub title: String,
    pub output: String,
}

/// How to put back what a discard threw away.
enum UndoAction {
    /// Re-apply the discarded patch forwards onto the working tree.
    ApplyPatch(String),
    /// Rewrite a file that was deleted wholesale. Raw bytes rather than a patch
    /// so untracked binaries survive too.
    RestoreFile { path: String, bytes: Vec<u8> },
    /// Put a dropped stash entry back; its commit outlives the entry.
    RestoreStash { sha: String, message: String },
}

pub struct UndoEntry {
    action: UndoAction,
    pub label: String,
}

/// Bounds how much discarded content is held in memory.
const UNDO_LIMIT: usize = 50;

/// Counts real changed lines in a patch, skipping the `---`/`+++` headers.
fn patch_change_count(patch: &str) -> usize {
    patch
        .lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count()
}

pub struct App {
    pub repo: Option<Repo>,
    pub status: Status,
    pub sel_file: Option<FileSel>,
    pub diff: Option<Loaded>,
    pub commit_msg: String,
    pub amend: bool,
    pub error: Option<String>,
    pub info: Option<String>,
    pub git_version: String,
    pub recent: Vec<PathBuf>,
    pub open_input: String,
    pub confirm: Option<Confirm>,
    pub branches: Vec<String>,
    pub new_branch: String,
    pub show_new_branch: bool,
    pub font_size: f32,
    /// Set when the diff should regain keyboard focus next frame.
    pub focus_diff: bool,
    /// Most recent discards, newest last. Discards are the only destructive
    /// operation, so they are the only thing worth being able to take back.
    pub undo: Vec<UndoEntry>,

    pub mode: Mode,
    pub find: Find,

    // History mode.
    pub commits: Vec<CommitInfo>,
    pub commit_filter: String,
    pub sel_commit: Option<String>,
    pub commit_files: Vec<CommitFile>,
    pub commit_meta: Option<CommitMeta>,
    pub log_limit: usize,
    pub log_all_refs: bool,

    // Push.
    pub remotes: Vec<String>,
    pub show_push: bool,
    pub push_remote: String,
    pub push_set_upstream: bool,
    pub push_force_lease: bool,
    pub push_tags: bool,
    pub running: Option<RunningTask>,
    pub outcome: Option<TaskOutcome>,

    // Stashes.
    pub stashes: Vec<StashEntry>,
    pub stash_filter: String,
    pub sel_stash: Option<usize>,
    pub stash_files: Vec<StashFile>,
    pub show_stash_dialog: bool,
    pub stash_message: String,
    pub stash_untracked: bool,
}

impl Default for App {
    fn default() -> Self {
        App {
            repo: None,
            status: Status::default(),
            sel_file: None,
            diff: None,
            commit_msg: String::new(),
            amend: false,
            error: None,
            info: None,
            git_version: String::new(),
            recent: Vec::new(),
            open_input: String::new(),
            confirm: None,
            branches: Vec::new(),
            new_branch: String::new(),
            show_new_branch: false,
            font_size: 12.0,
            focus_diff: false,
            undo: Vec::new(),
            mode: Mode::WorkingTree,
            find: Find::default(),
            commits: Vec::new(),
            commit_filter: String::new(),
            sel_commit: None,
            commit_files: Vec::new(),
            commit_meta: None,
            log_limit: 200,
            log_all_refs: false,
            remotes: Vec::new(),
            show_push: false,
            push_remote: String::new(),
            push_set_upstream: false,
            push_force_lease: false,
            push_tags: false,
            running: None,
            outcome: None,
            stashes: Vec::new(),
            stash_filter: String::new(),
            sel_stash: None,
            stash_files: Vec::new(),
            show_stash_dialog: false,
            stash_message: String::new(),
            stash_untracked: true,
        }
    }
}

impl App {
    pub fn new() -> App {
        let mut app = App::default();
        match crate::git::git_version() {
            Ok(v) => app.git_version = v,
            Err(e) => app.error = Some(e.to_string()),
        }
        app.recent = config::load_recent();
        app.font_size = config::load_font_size().unwrap_or(12.0);

        // An explicit path wins, then the working directory so `gitgui` inside a
        // checkout just opens it. With neither — a Finder or Dock launch — stop
        // at the welcome screen so the recent list is what you see, rather than
        // silently reopening whatever was last used.
        let candidates = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .into_iter()
            .chain(std::env::current_dir().ok());
        for p in candidates {
            if let Ok(repo) = Repo::discover(&p) {
                app.open(repo.root.clone());
                break;
            }
        }
        app
    }

    fn repo(&self) -> Option<Repo> {
        self.repo.clone()
    }

    pub fn open(&mut self, path: PathBuf) {
        match Repo::discover(&path) {
            Ok(repo) => {
                self.repo = Some(repo.clone());
                self.sel_file = None;
                self.diff = None;
                self.commit_msg.clear();
                self.amend = false;
                self.error = None;
                config::push_recent(&mut self.recent, &repo.root);
                self.rescan();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn open_dialog(&mut self) {
        #[cfg(feature = "dialogs")]
        {
            let start = self
                .repo
                .as_ref()
                .map(|r| r.root.clone())
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            if let Some(dir) = rfd::FileDialog::new().set_directory(start).pick_folder() {
                self.open(dir);
            }
        }
        #[cfg(not(feature = "dialogs"))]
        {
            self.error = Some(
                "This build has no native folder picker; type a path in the box instead."
                    .to_string(),
            );
        }
    }

    pub fn rescan(&mut self) {
        let Some(repo) = self.repo() else { return };
        match repo.status() {
            Ok(st) => {
                self.status = st;
                self.branches = repo.branches().unwrap_or_default();
                self.remotes = repo.remotes().unwrap_or_default();
                self.reload_diff();
                if self.mode == Mode::History {
                    self.load_log();
                }
                // Row indices may have shifted under the old hit list.
                self.find_recompute();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn select_file(&mut self, pane: Pane, path: &str) {
        self.sel_file = Some(FileSel {
            pane,
            path: path.to_string(),
        });
        self.load_diff();
        self.focus_diff = true;
        // Keep hit positions valid for the newly loaded content.
        self.find_recompute();
    }

    // ---- history --------------------------------------------------------

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.sel_file = None;
        self.diff = None;
        self.find.matches.clear();
        match mode {
            Mode::History if self.commits.is_empty() => self.load_log(),
            // Always reload stashes: they change from outside far more often
            // than history does, and the list is cheap.
            Mode::Stashes => self.load_stashes(),
            _ => {}
        }
    }

    pub fn load_log(&mut self) {
        let Some(repo) = self.repo() else { return };
        match repo.log(self.log_limit, self.log_all_refs) {
            Ok(c) => {
                self.commits = c;
                // Drop a selection that the new query no longer contains.
                if let Some(sha) = self.sel_commit.clone() {
                    if !self.commits.iter().any(|c| c.sha == sha) {
                        self.clear_commit_selection();
                    }
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn clear_commit_selection(&mut self) {
        self.sel_commit = None;
        self.commit_files.clear();
        self.commit_meta = None;
        self.sel_file = None;
        self.diff = None;
    }

    pub fn load_more_commits(&mut self) {
        self.log_limit = (self.log_limit * 2).min(20_000);
        self.load_log();
    }

    pub fn set_log_all_refs(&mut self, all: bool) {
        self.log_all_refs = all;
        self.load_log();
    }

    pub fn select_commit(&mut self, sha: &str) {
        let Some(repo) = self.repo() else { return };
        self.sel_commit = Some(sha.to_string());
        self.sel_file = None;
        self.diff = None;
        self.find.matches.clear();

        match repo.commit_files(sha) {
            Ok(f) => self.commit_files = f,
            Err(e) => {
                self.commit_files.clear();
                self.error = Some(e.to_string());
            }
        }
        match repo.commit_meta(sha) {
            Ok(m) => self.commit_meta = Some(m),
            Err(e) => {
                self.commit_meta = None;
                self.error = Some(e.to_string());
            }
        }
    }

    pub fn filtered_commits(&self) -> Vec<&CommitInfo> {
        let needle = self.commit_filter.trim().to_lowercase();
        self.commits.iter().filter(|c| c.matches(&needle)).collect()
    }

    // ---- stashes --------------------------------------------------------

    pub fn load_stashes(&mut self) {
        let Some(repo) = self.repo() else { return };
        match repo.stash_list() {
            Ok(list) => {
                self.stashes = list;
                // Indices shift whenever an entry is added or removed, so a
                // held selection can no longer be trusted.
                if self
                    .sel_stash
                    .map(|i| i >= self.stashes.len())
                    .unwrap_or(false)
                {
                    self.clear_stash_selection();
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn clear_stash_selection(&mut self) {
        self.sel_stash = None;
        self.stash_files.clear();
        if self.sel_file.as_ref().map(|s| s.pane) == Some(Pane::Stash) {
            self.sel_file = None;
            self.diff = None;
        }
    }

    pub fn select_stash(&mut self, index: usize) {
        let Some(repo) = self.repo() else { return };
        let Some(entry) = self.stashes.get(index).cloned() else {
            return;
        };
        self.sel_stash = Some(index);
        self.sel_file = None;
        self.diff = None;
        self.find.matches.clear();
        match repo.stash_files(&entry) {
            Ok(f) => self.stash_files = f,
            Err(e) => {
                self.stash_files.clear();
                self.error = Some(e.to_string());
            }
        }
    }

    pub fn selected_stash(&self) -> Option<&StashEntry> {
        self.stashes.get(self.sel_stash?)
    }

    pub fn filtered_stashes(&self) -> Vec<&StashEntry> {
        let needle = self.stash_filter.trim().to_lowercase();
        self.stashes.iter().filter(|s| s.matches(&needle)).collect()
    }

    pub fn open_stash_dialog(&mut self) {
        if self.status.unstaged().is_empty() && self.status.staged().is_empty() {
            self.error = Some("Nothing to stash — the working tree is clean.".into());
            return;
        }
        self.stash_message.clear();
        self.stash_untracked = self.status.entries.iter().any(|e| e.untracked);
        self.show_stash_dialog = true;
    }

    pub fn create_stash(&mut self) {
        let Some(repo) = self.repo() else { return };
        let msg = self.stash_message.clone();
        let untracked = self.stash_untracked;
        match repo.stash_push(&msg, untracked) {
            Ok(_) => {
                self.show_stash_dialog = false;
                self.stash_message.clear();
                self.info = Some("Stashed the working tree.".into());
                self.sel_file = None;
                self.diff = None;
                self.rescan();
                self.load_stashes();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// `pop` also removes the entry once it applies cleanly.
    pub fn apply_stash(&mut self, index: usize, pop: bool) {
        let Some(repo) = self.repo() else { return };
        let Some(entry) = self.stashes.get(index).cloned() else {
            return;
        };
        let refname = entry.refname();
        let res = if pop {
            repo.stash_pop(&refname)
        } else {
            repo.stash_apply(&refname)
        };
        match res {
            Ok(out) => {
                let verb = if pop { "Popped" } else { "Applied" };
                self.info = Some(format!("{verb} {refname}."));
                if !out.trim().is_empty() {
                    self.outcome = Some(TaskOutcome {
                        ok: true,
                        title: format!("{verb} {refname}"),
                        output: out.trim().to_string(),
                    });
                }
                self.clear_stash_selection();
                self.rescan();
                self.load_stashes();
            }
            Err(e) => {
                // Conflicts land here; the message from git explains what to do.
                self.outcome = Some(TaskOutcome {
                    ok: false,
                    title: format!("Could not {} {refname}", if pop { "pop" } else { "apply" }),
                    output: e.to_string(),
                });
                self.error = Some(format!("{refname} did not apply cleanly."));
                self.rescan();
                self.load_stashes();
            }
        }
    }

    pub fn ask_drop_stash(&mut self, index: usize) {
        let Some(entry) = self.stashes.get(index) else {
            return;
        };
        self.confirm = Some(Confirm::DropStash {
            refname: entry.refname(),
            label: format!("{} — {}", entry.refname(), entry.message()),
        });
    }

    fn do_drop_stash(&mut self, refname: &str) {
        let Some(repo) = self.repo() else { return };
        // Capture the commit first: the entry disappears but the commit does
        // not, so the drop can be undone.
        let entry = self
            .stashes
            .iter()
            .find(|s| s.refname() == refname)
            .cloned();

        match repo.stash_drop(refname) {
            Ok(_) => {
                if let Some(e) = entry {
                    let label = format!("{refname} — {}", e.message());
                    self.push_undo(
                        UndoAction::RestoreStash {
                            sha: e.sha.clone(),
                            message: e.subject.clone(),
                        },
                        label,
                    );
                    self.info = Some(format!("Dropped {refname}. ⌘Z to undo."));
                } else {
                    self.info = Some(format!("Dropped {refname}."));
                }
                self.clear_stash_selection();
                self.load_stashes();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    // ---- diff search ----------------------------------------------------

    pub fn find_open(&mut self) {
        self.find.open = true;
        self.find.request_focus = true;
        self.find_recompute();
    }

    pub fn find_close(&mut self) {
        self.find.open = false;
        self.find.matches.clear();
    }

    pub fn find_recompute(&mut self) {
        self.find.matches.clear();
        self.find.current = 0;
        if self.find.query.is_empty() {
            return;
        }
        let Some(d) = self.diff.as_ref() else { return };

        let cs = self.find.case_sensitive;
        let needle = fold(&self.find.query, cs);
        let mut hits = Vec::new();
        for (i, row) in d.rows.iter().enumerate() {
            let Row::Line { hunk, line } = row else {
                continue;
            };
            let l = &d.fd.hunks[*hunk].lines[*line];
            if l.kind == LineKind::NoNewline {
                continue;
            }
            let hay = fold(&l.text, cs);
            let mut from = 0usize;
            while from <= hay.len() {
                let Some(pos) = find_sub(&hay[from..], &needle) else {
                    break;
                };
                let start = from + pos;
                // Rendered rows carry a leading +/-/space marker column.
                hits.push(FindMatch {
                    row: i,
                    start: start + 1,
                    end: start + needle.len() + 1,
                });
                from = start + needle.len();
            }
        }
        self.find.matches = hits;
        self.jump_to_match();
    }

    pub fn find_next(&mut self) {
        if self.find.matches.is_empty() {
            return;
        }
        self.find.current = (self.find.current + 1) % self.find.matches.len();
        self.jump_to_match();
    }

    pub fn find_prev(&mut self) {
        if self.find.matches.is_empty() {
            return;
        }
        self.find.current = if self.find.current == 0 {
            self.find.matches.len() - 1
        } else {
            self.find.current - 1
        };
        self.jump_to_match();
    }

    /// Moves the cursor without touching the line selection, so searching never
    /// disturbs what is staged next.
    fn jump_to_match(&mut self) {
        let Some(m) = self.find.matches.get(self.find.current).copied() else {
            return;
        };
        if let Some(d) = self.diff.as_mut() {
            d.cursor = m.row.min(d.rows.len().saturating_sub(1));
            d.scroll_to_cursor = true;
        }
    }

    pub fn find_status(&self) -> String {
        if self.find.query.is_empty() {
            String::new()
        } else if self.find.matches.is_empty() {
            "no matches".to_string()
        } else {
            format!("{} of {}", self.find.current + 1, self.find.matches.len())
        }
    }

    /// Reloads the diff after a mutation, keeping the file selected only if it
    /// still has changes on that side.
    fn reload_diff(&mut self) {
        let Some(sel) = self.sel_file.clone() else {
            self.diff = None;
            return;
        };
        let still_there = match sel.pane {
            Pane::Unstaged => self.status.unstaged().iter().any(|e| e.path == sel.path),
            Pane::Staged => self.status.staged().iter().any(|e| e.path == sel.path),
            // Commit and stash contents are immutable, so a rescan never
            // invalidates them.
            Pane::Commit | Pane::Stash => true,
        };
        if still_there {
            let cursor = self.diff.as_ref().map(|d| d.cursor).unwrap_or(0);
            self.load_diff();
            if let Some(d) = self.diff.as_mut() {
                d.cursor = cursor.min(d.rows.len().saturating_sub(1));
            }
        } else {
            self.sel_file = None;
            self.diff = None;
        }
    }

    fn load_diff(&mut self) {
        let Some(repo) = self.repo() else {
            self.diff = None;
            return;
        };
        let Some(sel) = self.sel_file.clone() else {
            self.diff = None;
            return;
        };

        let entry = self
            .status
            .entries
            .iter()
            .find(|e| e.path == sel.path)
            .cloned();
        let untracked = entry.as_ref().map(|e| e.untracked).unwrap_or(false);
        let unmerged = entry.as_ref().map(|e| e.unmerged).unwrap_or(false);

        // Untracked files inside a stash are not in its tree at all, so they
        // have to be read out of the third parent instead of diffed.
        let stash_untracked = sel.pane == Pane::Stash
            && self
                .stash_files
                .iter()
                .any(|f| f.path == sel.path && f.untracked);
        let stash_ctx: Option<(String, Option<String>)> = self.selected_stash().map(|e| {
            let orig = self
                .stash_files
                .iter()
                .find(|f| f.path == sel.path)
                .and_then(|f| f.orig_path.clone());
            (e.sha.clone(), orig)
        });
        let stash_parent = self
            .selected_stash()
            .and_then(|e| e.untracked_parent().map(|p| p.to_string()));

        let result: Result<(FileDiff, Option<String>), String> = if sel.pane == Pane::Unstaged
            && untracked
        {
            self.read_untracked(&repo.root, &sel.path)
        } else if stash_untracked {
            match stash_parent {
                Some(parent) => read_stash_blob(&repo, &parent, &sel.path),
                None => Ok((
                    FileDiff::default(),
                    Some("This stash holds no untracked files.".to_string()),
                )),
            }
        } else {
            let text = match sel.pane {
                Pane::Unstaged => repo.diff_unstaged(&sel.path),
                Pane::Staged => repo.diff_staged(
                    &sel.path,
                    entry.as_ref().and_then(|e| e.orig_path.as_deref()),
                ),
                Pane::Commit => match self.sel_commit.as_deref() {
                    Some(sha) => {
                        let orig = self
                            .commit_files
                            .iter()
                            .find(|f| f.path == sel.path)
                            .and_then(|f| f.orig_path.clone());
                        repo.diff_commit(sha, &sel.path, orig.as_deref())
                    }
                    None => Ok(String::new()),
                },
                Pane::Stash => match &stash_ctx {
                    Some((sha, orig)) => repo.diff_stash(sha, &sel.path, orig.as_deref()),
                    None => Ok(String::new()),
                },
            };
            text.map(|t| {
                let fd = diff::parse(&t);
                let note = if fd.binary {
                    Some("Binary file — stage or unstage the whole file.".to_string())
                } else if unmerged {
                    Some(
                        "Conflicted file. Resolve the markers, then stage the whole file."
                            .to_string(),
                    )
                } else if fd.hunks.is_empty() {
                    Some(if fd.rename {
                        "Rename only — no content change.".to_string()
                    } else if fd.header_only {
                        "No content change (mode change only).".to_string()
                    } else {
                        "No changes to show.".to_string()
                    })
                } else {
                    None
                };
                (fd, note)
            })
            .map_err(|e| e.to_string())
        };

        match result {
            Ok((fd, note)) => self.diff = Some(Loaded::build(fd, untracked, note)),
            Err(e) => {
                self.error = Some(e);
                self.diff = None;
            }
        }
    }

    fn read_untracked(
        &self,
        root: &Path,
        rel: &str,
    ) -> Result<(FileDiff, Option<String>), String> {
        let full = root.join(rel);
        let meta = std::fs::metadata(&full).map_err(|e| format!("{rel}: {e}"))?;
        if meta.is_dir() {
            return Ok((
                FileDiff::default(),
                Some("Untracked directory — stage it to add its contents.".to_string()),
            ));
        }
        if meta.len() as usize > MAX_DISPLAY_BYTES {
            return Ok((
                FileDiff::default(),
                Some(format!(
                    "File is {:.1} MB — too large to display. Stage the whole file instead.",
                    meta.len() as f64 / (1024.0 * 1024.0)
                )),
            ));
        }
        let bytes = std::fs::read(&full).map_err(|e| format!("{rel}: {e}"))?;
        if bytes.contains(&0) {
            return Ok((
                FileDiff::default(),
                Some("Binary file — stage the whole file.".to_string()),
            ));
        }
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                return Ok((
                    FileDiff::default(),
                    Some("Not valid UTF-8 — stage the whole file.".to_string()),
                ))
            }
        };
        let exec = is_executable(&meta);
        let fd = diff::synth_new_file(rel, &text, exec);
        let note = if fd.hunks.is_empty() {
            Some("Empty file.".to_string())
        } else {
            None
        };
        Ok((fd, note))
    }

    // ---- operations ----------------------------------------------------

    /// Which operation the primary action performs, given the visible pane.
    pub fn primary_op(&self) -> Option<Op> {
        match self.sel_file.as_ref()?.pane {
            Pane::Unstaged => Some(Op::Stage),
            Pane::Staged => Some(Op::Unstage),
            Pane::Commit | Pane::Stash => None,
        }
    }

    pub fn read_only(&self) -> bool {
        self.sel_file
            .as_ref()
            .map(|s| s.pane.read_only())
            .unwrap_or(false)
    }

    pub fn apply_lines(&mut self, op: Op) {
        if self.read_only() {
            return;
        }
        let Some(pairs) = self.diff.as_ref().map(|d| d.selected_pairs()) else {
            return;
        };
        if pairs.is_empty() {
            // Nothing explicitly selected: fall back to the hunk at the cursor,
            // which is what makes single-click-then-stage feel natural.
            if let Some(h) = self.diff.as_ref().and_then(|d| d.hunk_of_row(d.cursor)) {
                self.apply_hunk(op, h);
            }
            return;
        }
        self.run_patch(op, &move |h, l| pairs.contains(&(h, l)));
    }

    pub fn apply_hunk(&mut self, op: Op, hunk: usize) {
        if self.read_only() {
            return;
        }
        self.run_patch(op, &move |h, _| h == hunk);
    }

    pub fn confirm_yes(&mut self) {
        let Some(c) = self.confirm.take() else { return };
        match c {
            Confirm::DiscardFile { path, untracked } => {
                self.do_discard_file(&path, untracked);
            }
            Confirm::DropStash { refname, .. } => {
                self.do_drop_stash(&refname);
            }
        }
    }

    fn run_patch(&mut self, op: Op, selected: &dyn Fn(usize, usize) -> bool) {
        let Some(repo) = self.repo() else { return };
        let Some(loaded) = self.diff.as_ref() else { return };

        if loaded.fd.hunks.is_empty() {
            self.error =
                Some("This file has no line-level changes; use the whole-file action.".into());
            return;
        }

        // Forward moves the patch's old side toward its new side; that is what
        // staging does. Unstaging and discarding both run a patch backwards.
        let mode = match op {
            Op::Stage => ApplyMode::Forward,
            Op::Unstage | Op::Discard => ApplyMode::Reverse,
        };
        // Only staging and unstaging touch the index; discard rewrites the file.
        let cached = op != Op::Discard;

        let Some(p) = patch::build_patch(&loaded.fd, mode, selected) else {
            self.error = Some("Nothing selected to apply.".into());
            return;
        };

        match repo.apply_patch(&p, mode, cached) {
            Ok(()) => {
                self.info = Some(match op {
                    Op::Stage => "Staged selected lines.".to_string(),
                    Op::Unstage => "Unstaged selected lines.".to_string(),
                    Op::Discard => {
                        let n = patch_change_count(&p);
                        let path = self
                            .sel_file
                            .as_ref()
                            .map(|s| s.path.clone())
                            .unwrap_or_default();
                        let label = format!("{n} line(s) in {path}");
                        self.push_undo(UndoAction::ApplyPatch(p), label);
                        format!("Discarded {n} line(s). ⌘Z to undo.")
                    }
                });
                self.rescan();
            }
            Err(e) => self.error = Some(format!("{} failed: {e}", op.verb().to_lowercase())),
        }
    }

    fn push_undo(&mut self, action: UndoAction, label: String) {
        self.undo.push(UndoEntry { action, label });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|e| e.label.clone())
    }

    pub fn undo_last_discard(&mut self) {
        let Some(repo) = self.repo() else { return };
        let Some(entry) = self.undo.pop() else { return };

        let res = match &entry.action {
            // Forward, uncached: the discard reverse-applied this same patch to
            // the working tree, so its old side is what is on disk now.
            UndoAction::ApplyPatch(p) => repo
                .apply_patch(p, ApplyMode::Forward, false)
                .map_err(|e| e.to_string()),
            UndoAction::RestoreFile { path, bytes } => {
                let full = repo.root.join(path);
                if let Some(dir) = full.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&full, bytes).map_err(|e| e.to_string())
            }
            UndoAction::RestoreStash { sha, message } => repo
                .stash_store(sha, message)
                .map(|_| ())
                .map_err(|e| e.to_string()),
        };

        match res {
            Ok(()) => {
                self.info = Some(format!("Restored {}.", entry.label));
                self.rescan();
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not undo: {e}. The file may have changed since."
                ));
                // Keep it available rather than silently dropping the content.
                self.undo.push(entry);
            }
        }
    }

    // ---- whole-file operations -----------------------------------------

    pub fn stage_file(&mut self, path: &str) {
        let Some(repo) = self.repo() else { return };
        match repo.stage_paths(&[path.to_string()]) {
            Ok(()) => self.rescan(),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn unstage_file(&mut self, path: &str) {
        let Some(repo) = self.repo() else { return };
        // A staged rename has to release both names or the old path stays gone.
        let mut paths = vec![path.to_string()];
        if let Some(e) = self.status.entries.iter().find(|e| e.path == path) {
            if let Some(o) = &e.orig_path {
                paths.push(o.clone());
            }
        }
        match repo.unstage_paths(&paths) {
            Ok(()) => self.rescan(),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn ask_discard_file(&mut self, path: &str) {
        let untracked = self
            .status
            .entries
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.untracked)
            .unwrap_or(false);
        self.confirm = Some(Confirm::DiscardFile {
            path: path.to_string(),
            untracked,
        });
    }

    fn do_discard_file(&mut self, path: &str, untracked: bool) {
        let Some(repo) = self.repo() else { return };

        // Capture what is about to be lost before losing it.
        let undo = if untracked {
            std::fs::read(repo.root.join(path))
                .ok()
                .map(|bytes| UndoAction::RestoreFile {
                    path: path.to_string(),
                    bytes,
                })
        } else {
            repo.diff_unstaged(path).ok().and_then(|t| {
                patch::build_full_patch(&diff::parse(&t), ApplyMode::Forward)
                    .map(UndoAction::ApplyPatch)
            })
        };

        let res = if untracked {
            std::fs::remove_file(repo.root.join(path)).map_err(|e| e.to_string())
        } else {
            repo.checkout_paths(&[path.to_string()])
                .map_err(|e| e.to_string())
        };
        match res {
            Ok(()) => {
                let recoverable = undo.is_some();
                if let Some(action) = undo {
                    self.push_undo(action, format!("{path}"));
                }
                self.info = Some(if recoverable {
                    format!("Discarded changes in {path}. ⌘Z to undo.")
                } else {
                    format!("Discarded changes in {path}.")
                });
                self.rescan();
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn stage_changed(&mut self) {
        let Some(repo) = self.repo() else { return };
        // Matches git-gui's "Stage Changed": tracked modifications only.
        match repo.run(&["add", "--update"]) {
            Ok(_) => self.rescan(),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn unstage_all(&mut self) {
        let paths: Vec<String> = self.status.staged().iter().map(|e| e.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        let Some(repo) = self.repo() else { return };
        match repo.unstage_paths(&paths) {
            Ok(()) => self.rescan(),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    // ---- commit ---------------------------------------------------------

    pub fn set_amend(&mut self, on: bool) {
        self.amend = on;
        if on && self.commit_msg.trim().is_empty() {
            if let Some(repo) = self.repo() {
                if let Ok(m) = repo.last_commit_message() {
                    self.commit_msg = m.trim_end().to_string();
                }
            }
        }
    }

    pub fn sign_off(&mut self) {
        let Some(repo) = self.repo() else { return };
        match repo.user_signature() {
            Ok(line) => {
                if self.commit_msg.contains(&line) {
                    return;
                }
                if !self.commit_msg.is_empty() && !self.commit_msg.ends_with('\n') {
                    self.commit_msg.push('\n');
                }
                if !self.commit_msg.is_empty() && !self.commit_msg.ends_with("\n\n") {
                    self.commit_msg.push('\n');
                }
                self.commit_msg.push_str(&line);
                self.commit_msg.push('\n');
            }
            Err(e) => self.error = Some(format!("user.name/user.email not set: {e}")),
        }
    }

    pub fn can_commit(&self) -> bool {
        !self.commit_msg.trim().is_empty()
            && (self.amend || !self.status.staged().is_empty())
    }

    pub fn commit(&mut self) {
        let Some(repo) = self.repo() else { return };
        if self.commit_msg.trim().is_empty() {
            self.error = Some("Enter a commit message first.".into());
            return;
        }
        if !self.amend && self.status.staged().is_empty() {
            self.error = Some("Nothing staged to commit.".into());
            return;
        }
        match repo.commit(&self.commit_msg, self.amend) {
            Ok(out) => {
                let summary = out.lines().next().unwrap_or("committed").to_string();
                self.info = Some(summary);
                self.commit_msg.clear();
                self.amend = false;
                self.rescan();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    // ---- push -----------------------------------------------------------

    /// The remote the current branch already tracks, e.g. "origin/main" -> "origin".
    fn upstream_remote(&self) -> Option<String> {
        let up = self.status.upstream.as_ref()?;
        up.split_once('/').map(|(r, _)| r.to_string())
    }

    pub fn open_push_dialog(&mut self) {
        if self.running.is_some() {
            return;
        }
        if self.status.detached {
            self.error = Some("Cannot push a detached HEAD. Check out a branch first.".into());
            return;
        }
        if self.status.branch.is_none() || self.status.unborn {
            self.error = Some("Nothing to push yet — make a commit first.".into());
            return;
        }
        if self.remotes.is_empty() {
            self.error =
                Some("This repository has no remotes. Add one with `git remote add`.".into());
            return;
        }

        // Prefer the branch's own upstream, then origin, then whatever exists.
        self.push_remote = self
            .upstream_remote()
            .filter(|r| self.remotes.contains(r))
            .or_else(|| {
                self.remotes
                    .iter()
                    .find(|r| *r == "origin")
                    .cloned()
            })
            .or_else(|| self.remotes.first().cloned())
            .unwrap_or_default();
        // Only offer to set upstream when there is not one already.
        self.push_set_upstream = self.status.upstream.is_none();
        self.push_force_lease = false;
        self.push_tags = false;
        self.show_push = true;
    }

    pub fn start_push(&mut self) {
        if self.running.is_some() {
            return;
        }
        let Some(repo) = self.repo() else { return };
        let Some(branch) = self.status.branch.clone() else {
            self.error = Some("No current branch to push.".into());
            return;
        };
        let opts = PushOptions {
            remote: self.push_remote.clone(),
            branch: branch.clone(),
            set_upstream: self.push_set_upstream,
            force_with_lease: self.push_force_lease,
            tags: self.push_tags,
        };
        let label = format!("Pushing {branch} to {}…", opts.remote);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // Send may fail if the app closed; nothing useful to do about it.
            let _ = tx.send(repo.push(&opts));
        });

        self.show_push = false;
        self.outcome = None;
        self.error = None;
        self.info = Some(label.clone());
        self.running = Some(RunningTask { label, rx });
    }

    /// Collects a finished background task. Returns true while one is still
    /// running, so the caller knows to keep repainting.
    pub fn poll_task(&mut self) -> bool {
        let Some(task) = self.running.as_ref() else {
            return false;
        };
        match task.rx.try_recv() {
            Err(TryRecvError::Empty) => true,
            Ok(result) => {
                let label = task.label.clone();
                self.running = None;
                match result {
                    Ok(out) => {
                        let body = if out.trim().is_empty() {
                            "Done.".to_string()
                        } else {
                            out.trim().to_string()
                        };
                        self.outcome = Some(TaskOutcome {
                            ok: true,
                            title: "Push complete".into(),
                            output: body,
                        });
                        self.info = Some("Push complete.".into());
                        // Picks up the moved remote-tracking ref, so ahead/behind
                        // settles back to zero.
                        self.rescan();
                    }
                    Err(out) => {
                        self.outcome = Some(TaskOutcome {
                            ok: false,
                            title: "Push failed".into(),
                            output: out,
                        });
                        self.info = None;
                        self.error = Some(format!("{label} failed."));
                    }
                }
                false
            }
            Err(TryRecvError::Disconnected) => {
                self.running = None;
                self.info = None;
                self.error = Some("The push task ended unexpectedly.".into());
                false
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    pub fn checkout_branch(&mut self, name: &str) {
        let Some(repo) = self.repo() else { return };
        match repo.checkout_branch(name) {
            Ok(()) => {
                self.info = Some(format!("Switched to {name}."));
                self.sel_file = None;
                self.diff = None;
                self.rescan();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn create_branch(&mut self) {
        let name = self.new_branch.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(repo) = self.repo() else { return };
        match repo.create_branch(&name) {
            Ok(()) => {
                self.info = Some(format!("Created and switched to {name}."));
                self.new_branch.clear();
                self.show_new_branch = false;
                self.rescan();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn branch_label(&self) -> String {
        if self.status.detached {
            return "detached HEAD".to_string();
        }
        let mut s = self
            .status
            .branch
            .clone()
            .unwrap_or_else(|| "(no branch)".to_string());
        if self.status.unborn {
            s.push_str(" (unborn)");
        }
        if self.status.ahead > 0 || self.status.behind > 0 {
            s.push_str(&format!(
                "  ↑{} ↓{}",
                self.status.ahead, self.status.behind
            ));
        }
        s
    }

    /// Summary of what the current line selection would affect.
    pub fn selection_summary(&self) -> Option<String> {
        let d = self.diff.as_ref()?;
        let n = d.selected_change_count();
        if n == 0 {
            return None;
        }
        let (adds, dels) = d.selected_pairs().iter().fold((0, 0), |(a, r), (h, l)| {
            match d.fd.hunks[*h].lines[*l].kind {
                LineKind::Addition => (a + 1, r),
                LineKind::Deletion => (a, r + 1),
                _ => (a, r),
            }
        });
        Some(format!("{n} line(s) selected  +{adds} −{dels}"))
    }
}

/// Presents an untracked file stored in a stash as an add-everything diff, the
/// same way an untracked file in the working tree is shown.
fn read_stash_blob(
    repo: &Repo,
    parent: &str,
    path: &str,
) -> Result<(FileDiff, Option<String>), String> {
    let bytes = repo
        .stash_untracked_blob(parent, path)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_DISPLAY_BYTES {
        return Ok((
            FileDiff::default(),
            Some(format!(
                "File is {:.1} MB — too large to display.",
                bytes.len() as f64 / (1024.0 * 1024.0)
            )),
        ));
    }
    if bytes.contains(&0) {
        return Ok((
            FileDiff::default(),
            Some("Binary file — stored in the stash but not shown.".to_string()),
        ));
    }
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => {
            return Ok((
                FileDiff::default(),
                Some("Not valid UTF-8 — not shown.".to_string()),
            ))
        }
    };
    let fd = diff::synth_new_file(path, &text, false);
    let note = if fd.hunks.is_empty() {
        Some("Empty file.".to_string())
    } else {
        None
    };
    Ok((fd, note))
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}
