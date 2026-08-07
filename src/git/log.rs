//! Commit history: parsing `git log` and per-commit file lists.

use super::status::Change;

/// Fields are separated by US (0x1f) and records by NUL, so neither a subject
/// containing newlines nor a path containing spaces can confuse the split.
pub const LOG_FORMAT: &str = "%H%x1f%h%x1f%an%x1f%ad%x1f%D%x1f%s";

const US: char = '\u{1f}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub date: String,
    /// Decorations from %D, e.g. "HEAD -> main, origin/main".
    pub refs: String,
    pub subject: String,
}

impl CommitInfo {
    /// Case-insensitive match used by the commit filter box.
    pub fn matches(&self, needle_lower: &str) -> bool {
        if needle_lower.is_empty() {
            return true;
        }
        self.subject.to_lowercase().contains(needle_lower)
            || self.author.to_lowercase().contains(needle_lower)
            || self.sha.starts_with(needle_lower)
            || self.refs.to_lowercase().contains(needle_lower)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CommitMeta {
    pub author: String,
    pub email: String,
    pub date: String,
    pub refs: String,
    pub parents: Vec<String>,
    pub body: String,
}

impl CommitMeta {
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFile {
    pub status: Change,
    pub path: String,
    pub orig_path: Option<String>,
}

pub fn parse_log(raw: &[u8]) -> Vec<CommitInfo> {
    raw.split(|b| *b == 0)
        .filter_map(|rec| {
            let s = String::from_utf8_lossy(rec);
            let s = s.trim_start_matches(['\n', '\r']);
            if s.is_empty() {
                return None;
            }
            // Subject last, so a stray separator in it stays attached.
            let f: Vec<&str> = s.splitn(6, US).collect();
            if f.len() < 6 {
                return None;
            }
            Some(CommitInfo {
                sha: f[0].to_string(),
                short: f[1].to_string(),
                author: f[2].to_string(),
                date: f[3].to_string(),
                refs: f[4].to_string(),
                subject: f[5].to_string(),
            })
        })
        .collect()
}

pub fn parse_meta(raw: &str) -> CommitMeta {
    let f: Vec<&str> = raw.splitn(6, US).collect();
    if f.len() < 6 {
        return CommitMeta::default();
    }
    CommitMeta {
        author: f[0].to_string(),
        email: f[1].to_string(),
        date: f[2].to_string(),
        refs: f[3].to_string(),
        parents: f[4]
            .split_whitespace()
            .map(|s| s.to_string())
            .collect(),
        body: f[5].trim_end().to_string(),
    }
}

/// Parses `--name-status -z` output. Renames and copies carry two paths, so the
/// token stream has to be walked rather than chunked in pairs.
pub fn parse_name_status(raw: &[u8]) -> Vec<CommitFile> {
    let tokens: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let status = String::from_utf8_lossy(tokens[i]);
        let status = status.trim_start_matches(['\n', '\r']).trim();
        i += 1;
        if status.is_empty() {
            continue;
        }
        let code = status.chars().next().unwrap_or(' ');
        let change = Change::from_code(code);

        if matches!(code, 'R' | 'C') {
            let Some(orig) = tokens.get(i) else { break };
            let Some(path) = tokens.get(i + 1) else { break };
            i += 2;
            out.push(CommitFile {
                status: change,
                path: String::from_utf8_lossy(path).into_owned(),
                orig_path: Some(String::from_utf8_lossy(orig).into_owned()),
            });
        } else {
            let Some(path) = tokens.get(i) else { break };
            i += 1;
            let p = String::from_utf8_lossy(path).into_owned();
            if p.is_empty() {
                continue;
            }
            out.push(CommitFile {
                status: change,
                path: p,
                orig_path: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(parts: &[&str]) -> Vec<u8> {
        parts.join("\0").into_bytes()
    }

    #[test]
    fn parses_log_records() {
        let raw = z(&[
            "abc123\u{1f}abc\u{1f}Alice\u{1f}2026-08-07\u{1f}HEAD -> main\u{1f}merge side",
            "def456\u{1f}def\u{1f}Bob\u{1f}2026-08-06\u{1f}\u{1f}fix: a thing",
        ]);
        let v = parse_log(&raw);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].sha, "abc123");
        assert_eq!(v[0].short, "abc");
        assert_eq!(v[0].author, "Alice");
        assert_eq!(v[0].refs, "HEAD -> main");
        assert_eq!(v[0].subject, "merge side");
        assert_eq!(v[1].refs, "");
        assert_eq!(v[1].subject, "fix: a thing");
    }

    #[test]
    fn subject_may_contain_separators_and_colons() {
        let raw = z(&["s\u{1f}s\u{1f}A\u{1f}d\u{1f}\u{1f}weird\u{1f}subject"]);
        let v = parse_log(&raw);
        assert_eq!(v[0].subject, "weird\u{1f}subject");
    }

    #[test]
    fn parses_name_status_with_rename() {
        // Exactly what `git show --name-status -z --find-renames` emits.
        let raw = z(&["R073", "a.txt", "renamed.txt", "M", "other.rs", ""]);
        let v = parse_name_status(&raw);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].status, Change::Renamed);
        assert_eq!(v[0].orig_path.as_deref(), Some("a.txt"));
        assert_eq!(v[0].path, "renamed.txt");
        assert_eq!(v[1].status, Change::Modified);
        assert_eq!(v[1].path, "other.rs");
        assert_eq!(v[1].orig_path, None);
    }

    #[test]
    fn parses_added_and_deleted() {
        let raw = z(&["A", "new.rs", "D", "gone.rs"]);
        let v = parse_name_status(&raw);
        assert_eq!(v[0].status, Change::Added);
        assert_eq!(v[1].status, Change::Deleted);
    }

    #[test]
    fn empty_name_status_yields_nothing() {
        assert!(parse_name_status(b"").is_empty());
        assert!(parse_name_status(b"\0").is_empty());
    }

    #[test]
    fn parses_meta_with_parents() {
        let raw = "Alice\u{1f}a@e.com\u{1f}2026-08-07 10:00:00 +0200\u{1f}HEAD -> main\u{1f}p1 p2\u{1f}subject\n\nbody text\n";
        let m = parse_meta(raw);
        assert_eq!(m.author, "Alice");
        assert_eq!(m.email, "a@e.com");
        assert_eq!(m.parents, vec!["p1", "p2"]);
        assert!(m.is_merge());
        assert_eq!(m.body, "subject\n\nbody text");
    }

    #[test]
    fn single_parent_is_not_a_merge() {
        let raw = "A\u{1f}a@e\u{1f}d\u{1f}\u{1f}p1\u{1f}s";
        assert!(!parse_meta(raw).is_merge());
    }

    #[test]
    fn commit_filter_matches_subject_author_sha_and_refs() {
        let c = CommitInfo {
            sha: "abc123def".into(),
            short: "abc123d".into(),
            author: "Alice Smith".into(),
            date: "2026-08-07".into(),
            refs: "origin/main".into(),
            subject: "Fix the Widget".into(),
        };
        assert!(c.matches(""));
        assert!(c.matches("widget"));
        assert!(c.matches("alice"));
        assert!(c.matches("abc12"));
        assert!(c.matches("origin/"));
        assert!(!c.matches("nonexistent"));
    }
}
