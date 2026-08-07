//! Parser for `git status --porcelain=v2 --branch -z --untracked-files=all`.
//!
//! Using `-z` matters: it makes git emit paths verbatim instead of applying
//! `core.quotepath` escaping, so the strings here can be handed straight back
//! to git as pathspec arguments.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Unchanged,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Untracked,
    Ignored,
}

impl Change {
    pub fn from_code(c: char) -> Change {
        match c {
            'M' => Change::Modified,
            'A' => Change::Added,
            'D' => Change::Deleted,
            'R' => Change::Renamed,
            'C' => Change::Copied,
            'T' => Change::TypeChanged,
            'U' => Change::Unmerged,
            '?' => Change::Untracked,
            '!' => Change::Ignored,
            _ => Change::Unchanged,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Change::Unchanged => " ",
            Change::Modified => "M",
            Change::Added => "A",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Copied => "C",
            Change::TypeChanged => "T",
            Change::Unmerged => "U",
            Change::Untracked => "?",
            Change::Ignored => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Change::Unchanged => "unchanged",
            Change::Modified => "modified",
            Change::Added => "new file",
            Change::Deleted => "deleted",
            Change::Renamed => "renamed",
            Change::Copied => "copied",
            Change::TypeChanged => "type changed",
            Change::Unmerged => "conflicted",
            Change::Untracked => "untracked",
            Change::Ignored => "ignored",
        }
    }

    pub fn is_some(self) -> bool {
        self != Change::Unchanged
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    /// Set for renames and copies: where the content came from.
    pub orig_path: Option<String>,
    /// Staged side (HEAD -> index).
    pub index: Change,
    /// Unstaged side (index -> worktree).
    pub worktree: Change,
    pub unmerged: bool,
    pub untracked: bool,
}

impl Entry {
    pub fn display_path(&self) -> String {
        match &self.orig_path {
            Some(o) => format!("{o} → {}", self.path),
            None => self.path.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub entries: Vec<Entry>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub unborn: bool,
}

impl Status {
    /// Files with something to stage. A file can legitimately appear in both
    /// lists (e.g. XY = "MM"), which is exactly what git-gui shows.
    pub fn unstaged(&self) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| e.unmerged || e.untracked || e.worktree.is_some())
            .collect()
    }

    pub fn staged(&self) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| !e.untracked && !e.unmerged && e.index.is_some())
            .collect()
    }
}

pub fn parse(raw: &[u8]) -> Status {
    let mut st = Status::default();
    let records: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let mut i = 0usize;
    while i < records.len() {
        let rec = records[i];
        i += 1;
        if rec.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(rec).into_owned();

        if let Some(hdr) = line.strip_prefix("# ") {
            parse_header(hdr, &mut st);
            continue;
        }

        match line.as_bytes()[0] {
            b'1' => {
                if let Some(e) = parse_ordinary(&line) {
                    st.entries.push(e);
                }
            }
            b'2' => {
                // A rename/copy record carries the original path in the *next*
                // NUL-separated field, so consume one extra record.
                let orig = records.get(i).map(|r| String::from_utf8_lossy(r).into_owned());
                i += 1;
                if let Some(mut e) = parse_renamed(&line) {
                    e.orig_path = orig.filter(|s| !s.is_empty());
                    st.entries.push(e);
                }
            }
            b'u' => {
                if let Some(e) = parse_unmerged(&line) {
                    st.entries.push(e);
                }
            }
            b'?' => {
                if let Some(p) = line.get(2..) {
                    st.entries.push(Entry {
                        path: p.to_string(),
                        orig_path: None,
                        index: Change::Unchanged,
                        worktree: Change::Untracked,
                        unmerged: false,
                        untracked: true,
                    });
                }
            }
            _ => {}
        }
    }
    st.entries.sort_by(|a, b| a.path.cmp(&b.path));
    st
}

fn parse_header(hdr: &str, st: &mut Status) {
    let mut parts = hdr.splitn(2, ' ');
    let key = parts.next().unwrap_or("");
    let val = parts.next().unwrap_or("").trim();
    match key {
        "branch.oid" => st.unborn = val == "(initial)",
        "branch.head" => {
            if val == "(detached)" {
                st.detached = true;
            } else {
                st.branch = Some(val.to_string());
            }
        }
        "branch.upstream" => st.upstream = Some(val.to_string()),
        "branch.ab" => {
            for tok in val.split_whitespace() {
                if let Some(n) = tok.strip_prefix('+') {
                    st.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = tok.strip_prefix('-') {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        }
        _ => {}
    }
}

fn xy(field: &str) -> (Change, Change) {
    let mut cs = field.chars();
    let x = cs.next().map(Change::from_code).unwrap_or(Change::Unchanged);
    let y = cs.next().map(Change::from_code).unwrap_or(Change::Unchanged);
    (x, y)
}

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
fn parse_ordinary(line: &str) -> Option<Entry> {
    let f: Vec<&str> = line.splitn(9, ' ').collect();
    if f.len() < 9 {
        return None;
    }
    let (index, worktree) = xy(f[1]);
    Some(Entry {
        path: f[8].to_string(),
        orig_path: None,
        index,
        worktree,
        unmerged: false,
        untracked: false,
    })
}

/// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>`
fn parse_renamed(line: &str) -> Option<Entry> {
    let f: Vec<&str> = line.splitn(10, ' ').collect();
    if f.len() < 10 {
        return None;
    }
    let (index, worktree) = xy(f[1]);
    Some(Entry {
        path: f[9].to_string(),
        orig_path: None,
        index,
        worktree,
        unmerged: false,
        untracked: false,
    })
}

/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
fn parse_unmerged(line: &str) -> Option<Entry> {
    let f: Vec<&str> = line.splitn(11, ' ').collect();
    if f.len() < 11 {
        return None;
    }
    let (index, worktree) = xy(f[1]);
    Some(Entry {
        path: f[10].to_string(),
        orig_path: None,
        index,
        worktree,
        unmerged: true,
        untracked: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(parts: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for p in parts {
            v.extend_from_slice(p.as_bytes());
            v.push(0);
        }
        v
    }

    #[test]
    fn parses_branch_and_ordinary_entries() {
        let raw = z(&[
            "# branch.oid abc123",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 MM N... 100644 100644 100644 aaa bbb src/app.rs",
            "1 .M N... 100644 100644 100644 aaa bbb src/ui.rs",
            "? notes.txt",
        ]);
        let st = parse(&raw);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        assert_eq!(st.entries.len(), 3);

        let app = st.entries.iter().find(|e| e.path == "src/app.rs").unwrap();
        assert_eq!(app.index, Change::Modified);
        assert_eq!(app.worktree, Change::Modified);
        // "MM" means it belongs in both panes.
        assert!(st.unstaged().iter().any(|e| e.path == "src/app.rs"));
        assert!(st.staged().iter().any(|e| e.path == "src/app.rs"));

        let ui = st.entries.iter().find(|e| e.path == "src/ui.rs").unwrap();
        assert_eq!(ui.index, Change::Unchanged);
        assert!(!st.staged().iter().any(|e| e.path == "src/ui.rs"));
    }

    #[test]
    fn rename_record_consumes_original_path() {
        let raw = z(&[
            "# branch.head main",
            "2 R. N... 100644 100644 100644 aaa bbb R100 new/name.rs",
            "old/name.rs",
            "1 .M N... 100644 100644 100644 aaa bbb after.rs",
        ]);
        let st = parse(&raw);
        assert_eq!(st.entries.len(), 2, "orig path must not become its own entry");
        let r = st.entries.iter().find(|e| e.path == "new/name.rs").unwrap();
        assert_eq!(r.orig_path.as_deref(), Some("old/name.rs"));
        assert_eq!(r.index, Change::Renamed);
        assert!(st.entries.iter().any(|e| e.path == "after.rs"));
    }

    #[test]
    fn paths_with_spaces_survive() {
        let raw = z(&["1 .M N... 100644 100644 100644 aaa bbb dir with spaces/a b.txt"]);
        let st = parse(&raw);
        assert_eq!(st.entries[0].path, "dir with spaces/a b.txt");
    }

    #[test]
    fn unmerged_entries_go_to_unstaged_only() {
        let raw = z(&["u UU N... 100644 100644 100644 100644 a b c d conflict.rs"]);
        let st = parse(&raw);
        assert!(st.entries[0].unmerged);
        assert_eq!(st.unstaged().len(), 1);
        assert_eq!(st.staged().len(), 0);
    }
}
