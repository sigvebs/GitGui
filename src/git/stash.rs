//! Stash entries.
//!
//! A stash is a real commit with two or three parents: the HEAD it was taken
//! from, the index state, and — only when created with `-u` — a third holding
//! the untracked files. Those untracked files are *not* in the stash commit's
//! own tree, so a plain `diff <stash>^1 <stash>` never shows them; they have to
//! be read out of the third parent separately.

use super::status::Change;

/// Deliberately avoids `%gd`: that field is rendered through `--date`, so
/// asking for a short date turns `stash@{0}` into `stash@{2026-08-07}`, which
/// is not a usable ref. The index comes from list position instead.
pub const STASH_FORMAT: &str = "%H%x1f%s%x1f%as%x1f%P";

const US: char = '\u{1f}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    /// Position in the list; the ref is `stash@{index}`.
    pub index: usize,
    pub sha: String,
    /// e.g. "On main: fixing the widget".
    pub subject: String,
    pub date: String,
    pub parents: Vec<String>,
}

impl StashEntry {
    pub fn refname(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }

    /// True when the stash was taken with `--include-untracked`.
    pub fn has_untracked(&self) -> bool {
        self.parents.len() >= 3
    }

    /// The commit holding untracked files, if any.
    pub fn untracked_parent(&self) -> Option<&str> {
        self.parents.get(2).map(|s| s.as_str())
    }

    /// Strips git's "On <branch>: " prefix for a tidier display.
    pub fn message(&self) -> &str {
        match self.subject.split_once(": ") {
            Some((head, rest)) if head.starts_with("On ") || head.starts_with("WIP on ") => rest,
            _ => &self.subject,
        }
    }

    pub fn branch(&self) -> Option<&str> {
        let head = self.subject.split_once(": ")?.0;
        head.strip_prefix("WIP on ").or_else(|| head.strip_prefix("On "))
    }

    pub fn matches(&self, needle_lower: &str) -> bool {
        if needle_lower.is_empty() {
            return true;
        }
        self.subject.to_lowercase().contains(needle_lower)
            || self.sha.starts_with(needle_lower)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashFile {
    pub status: Change,
    pub path: String,
    pub orig_path: Option<String>,
    /// Came from the third parent, so it needs reading rather than diffing.
    pub untracked: bool,
}

pub fn parse_list(raw: &[u8]) -> Vec<StashEntry> {
    raw.split(|b| *b == 0)
        .filter_map(|rec| {
            let s = String::from_utf8_lossy(rec);
            let s = s.trim_start_matches(['\n', '\r']);
            if s.is_empty() {
                return None;
            }
            let f: Vec<&str> = s.splitn(4, US).collect();
            if f.len() < 4 {
                return None;
            }
            Some(StashEntry {
                index: 0, // filled in below
                sha: f[0].to_string(),
                subject: f[1].to_string(),
                date: f[2].to_string(),
                parents: f[3].split_whitespace().map(|p| p.to_string()).collect(),
            })
        })
        .enumerate()
        .map(|(i, mut e)| {
            e.index = i;
            e
        })
        .collect()
}

/// Splits a NUL-separated path list, as produced by `ls-tree -z`.
pub fn parse_paths(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter_map(|p| {
            let s = String::from_utf8_lossy(p).trim_matches(['\n', '\r']).to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(parts: &[&str]) -> Vec<u8> {
        parts.join("\0").into_bytes()
    }

    #[test]
    fn parses_list_and_numbers_entries_by_position() {
        let raw = z(&[
            "aaa\u{1f}On main: newest\u{1f}2026-08-07\u{1f}p1 p2",
            "bbb\u{1f}On main: with untracked\u{1f}2026-08-06\u{1f}p1 p2 p3",
            "ccc\u{1f}WIP on side: oldest\u{1f}2026-08-05\u{1f}p1 p2",
        ]);
        let v = parse_list(&raw);
        assert_eq!(v.len(), 3);

        assert_eq!(v[0].index, 0);
        assert_eq!(v[0].refname(), "stash@{0}");
        assert_eq!(v[0].sha, "aaa");
        assert!(!v[0].has_untracked());
        assert_eq!(v[0].untracked_parent(), None);

        assert_eq!(v[1].refname(), "stash@{1}");
        assert!(v[1].has_untracked(), "three parents means -u was used");
        assert_eq!(v[1].untracked_parent(), Some("p3"));

        assert_eq!(v[2].refname(), "stash@{2}");
    }

    #[test]
    fn strips_the_on_branch_prefix_for_display() {
        let e = StashEntry {
            index: 0,
            sha: "a".into(),
            subject: "On main: fixing the widget".into(),
            date: "2026-08-07".into(),
            parents: vec!["p1".into(), "p2".into()],
        };
        assert_eq!(e.message(), "fixing the widget");
        assert_eq!(e.branch(), Some("main"));

        let wip = StashEntry {
            subject: "WIP on feature/x: 1234567 some commit".into(),
            ..e.clone()
        };
        assert_eq!(wip.message(), "1234567 some commit");
        assert_eq!(wip.branch(), Some("feature/x"));

        // A message with no recognisable prefix is left alone.
        let bare = StashEntry {
            subject: "just a message".into(),
            ..e.clone()
        };
        assert_eq!(bare.message(), "just a message");
        assert_eq!(bare.branch(), None);
    }

    #[test]
    fn message_keeps_later_colons() {
        let e = StashEntry {
            index: 0,
            sha: "a".into(),
            subject: "On main: fix: the thing".into(),
            date: "d".into(),
            parents: vec![],
        };
        assert_eq!(e.message(), "fix: the thing");
    }

    #[test]
    fn filter_matches_subject_and_sha() {
        let e = StashEntry {
            index: 0,
            sha: "abc123".into(),
            subject: "On main: widget work".into(),
            date: "d".into(),
            parents: vec![],
        };
        assert!(e.matches(""));
        assert!(e.matches("widget"));
        assert!(e.matches("abc"));
        assert!(!e.matches("nothing"));
    }

    #[test]
    fn empty_list_parses_to_nothing() {
        assert!(parse_list(b"").is_empty());
        assert!(parse_list(b"\0").is_empty());
    }

    #[test]
    fn parses_nul_separated_paths() {
        assert_eq!(
            parse_paths(b"one.txt\0dir/two.txt\0"),
            vec!["one.txt", "dir/two.txt"]
        );
        assert!(parse_paths(b"").is_empty());
    }
}
