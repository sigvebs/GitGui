//! Unified-diff parser.
//!
//! Diffs are always requested one path at a time, so this never has to
//! disambiguate the `diff --git a/x b/y` line for paths containing spaces.
//! Header lines are kept verbatim so they can be replayed into `git apply`
//! without re-quoting anything.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Context,
    Addition,
    Deletion,
    /// The `\ No newline at end of file` marker; belongs to the line above it.
    NoNewline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Content without the leading +/-/space marker. For `NoNewline`, the
    /// whole marker line verbatim.
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

impl DiffLine {
    pub fn is_change(&self) -> bool {
        matches!(self.kind, LineKind::Addition | LineKind::Deletion)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// Everything after the closing `@@`, including its leading space.
    pub section: String,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn header(&self) -> String {
        format!(
            "@@ -{} +{} @@{}",
            range(self.old_start, self.old_count),
            range(self.new_start, self.new_count),
            self.section
        )
    }

    pub fn change_count(&self) -> usize {
        self.lines.iter().filter(|l| l.is_change()).count()
    }
}

pub fn range(start: u32, count: u32) -> String {
    if count == 1 {
        format!("{start}")
    } else {
        format!("{start},{count}")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Verbatim lines preceding the first `@@` hunk header.
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    pub is_new: bool,
    pub is_delete: bool,
    /// Present when the diff carried `rename from`/`rename to`.
    pub rename: bool,
    /// Set when the diff had a header but no hunks (pure mode/rename change).
    pub header_only: bool,
}

impl FileDiff {
    pub fn total_changes(&self) -> usize {
        self.hunks.iter().map(|h| h.change_count()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty() && self.header.is_empty()
    }

    /// The `a/` side path, taken from the `--- ` line.
    pub fn old_path(&self) -> Option<String> {
        self.header
            .iter()
            .find_map(|l| l.strip_prefix("--- "))
            .filter(|p| *p != "/dev/null")
            .map(|p| p.trim_start_matches("a/").to_string())
    }

    /// The `b/` side path, taken from the `+++ ` line.
    pub fn new_path(&self) -> Option<String> {
        self.header
            .iter()
            .find_map(|l| l.strip_prefix("+++ "))
            .filter(|p| *p != "/dev/null")
            .map(|p| p.trim_start_matches("b/").to_string())
    }
}

fn parse_range(spec: &str) -> (u32, u32) {
    match spec.split_once(',') {
        Some((s, c)) => (s.parse().unwrap_or(0), c.parse().unwrap_or(0)),
        None => (spec.parse().unwrap_or(0), 1),
    }
}

/// `@@ -12,7 +12,9 @@ optional section heading`
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_spec, rest) = rest.split_once(" +")?;
    let (new_spec, section) = match rest.split_once(" @@") {
        Some(v) => v,
        None => (rest.trim_end_matches(" @@"), ""),
    };
    let (old_start, old_count) = parse_range(old_spec);
    let (new_start, new_count) = parse_range(new_spec);
    Some(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section: section.to_string(),
        lines: Vec::new(),
    })
}

/// Splits on `\n` only. `str::lines()` also strips a trailing `\r`, which would
/// drop the CR from every line of a CRLF-checked-out file and make the
/// reconstructed patch fail to apply.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut v: Vec<&str> = text.split('\n').collect();
    if v.last() == Some(&"") {
        v.pop();
    }
    v
}

pub fn parse(text: &str) -> FileDiff {
    let mut fd = FileDiff::default();
    let mut lines = split_lines(text).into_iter().peekable();

    // Header: everything up to the first hunk.
    while let Some(&line) = lines.peek() {
        if line.starts_with("@@ ") {
            break;
        }
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            fd.binary = true;
        }
        if line.starts_with("new file mode ") {
            fd.is_new = true;
        }
        if line.starts_with("deleted file mode ") {
            fd.is_delete = true;
        }
        if line.starts_with("rename from ") || line.starts_with("rename to ") {
            fd.rename = true;
        }
        fd.header.push(line.to_string());
        lines.next();
    }

    let mut old_no = 0u32;
    let mut new_no = 0u32;

    for line in lines {
        if line.starts_with("@@ ") {
            if let Some(h) = parse_hunk_header(line) {
                old_no = h.old_start;
                new_no = h.new_start;
                fd.hunks.push(h);
            }
            continue;
        }
        // A `diff --git` here would mean a second file in the stream, which we
        // never ask for; stop rather than mis-attribute its lines.
        if line.starts_with("diff --git ") {
            break;
        }
        let Some(hunk) = fd.hunks.last_mut() else {
            continue;
        };
        let (kind, text) = match line.as_bytes().first() {
            Some(b' ') => (LineKind::Context, &line[1..]),
            Some(b'+') => (LineKind::Addition, &line[1..]),
            Some(b'-') => (LineKind::Deletion, &line[1..]),
            Some(b'\\') => (LineKind::NoNewline, line),
            // git renders an empty context line as a lone space, but be
            // lenient about a stripped one.
            None => (LineKind::Context, line),
            _ => continue,
        };
        let (l_old, l_new) = match kind {
            LineKind::Context => {
                let v = (Some(old_no), Some(new_no));
                old_no += 1;
                new_no += 1;
                v
            }
            LineKind::Addition => {
                let v = (None, Some(new_no));
                new_no += 1;
                v
            }
            LineKind::Deletion => {
                let v = (Some(old_no), None);
                old_no += 1;
                v
            }
            LineKind::NoNewline => (None, None),
        };
        hunk.lines.push(DiffLine {
            kind,
            text: text.to_string(),
            old_no: l_old,
            new_no: l_new,
        });
    }

    fd.header_only = fd.hunks.is_empty() && !fd.header.is_empty() && !fd.binary;
    fd
}

/// Untracked files have no diff against the index, so present the whole file
/// as one add-everything hunk. Built by hand rather than via
/// `git diff --no-index /dev/null <path>` because `/dev/null` has no portable
/// equivalent on Windows.
pub fn synth_new_file(path: &str, content: &str, executable: bool) -> FileDiff {
    let mode = if executable { "100755" } else { "100644" };
    let header = vec![
        format!("diff --git a/{path} b/{path}"),
        format!("new file mode {mode}"),
        "--- /dev/null".to_string(),
        format!("+++ b/{path}"),
    ];

    let ends_with_newline = content.ends_with('\n');
    let body: Vec<&str> = if content.is_empty() {
        Vec::new()
    } else {
        let mut v: Vec<&str> = content.split('\n').collect();
        if ends_with_newline {
            v.pop();
        }
        v
    };

    let mut lines: Vec<DiffLine> = body
        .iter()
        .enumerate()
        .map(|(i, l)| DiffLine {
            kind: LineKind::Addition,
            text: (*l).to_string(),
            old_no: None,
            new_no: Some(i as u32 + 1),
        })
        .collect();
    if !body.is_empty() && !ends_with_newline {
        lines.push(DiffLine {
            kind: LineKind::NoNewline,
            text: "\\ No newline at end of file".to_string(),
            old_no: None,
            new_no: None,
        });
    }

    let n = body.len() as u32;
    let hunks = if n == 0 {
        Vec::new()
    } else {
        vec![Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: n,
            section: String::new(),
            lines,
        }]
    };

    FileDiff {
        header,
        hunks,
        binary: false,
        is_new: true,
        is_delete: false,
        rename: false,
        header_only: n == 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/app.rs b/src/app.rs
index 1111111..2222222 100644
--- a/src/app.rs
+++ b/src/app.rs
@@ -10,3 +10,4 @@ fn main() {
 let a = 1;
-let b = 2;
+let b = 3;
+let c = 4;
 let d = 5;
@@ -40,2 +41,2 @@ fn other() {
 keep
-drop me
+add me
";

    #[test]
    fn parses_two_hunks_with_line_numbers() {
        let fd = parse(SAMPLE);
        assert_eq!(fd.hunks.len(), 2);
        assert!(!fd.is_new && !fd.is_delete && !fd.binary);
        assert_eq!(fd.old_path().as_deref(), Some("src/app.rs"));
        assert_eq!(fd.new_path().as_deref(), Some("src/app.rs"));

        let h = &fd.hunks[0];
        assert_eq!((h.old_start, h.old_count), (10, 3));
        assert_eq!((h.new_start, h.new_count), (10, 4));
        assert_eq!(h.section, " fn main() {");
        assert_eq!(h.lines.len(), 5);
        assert_eq!(h.lines[0].kind, LineKind::Context);
        assert_eq!(h.lines[0].old_no, Some(10));
        assert_eq!(h.lines[0].new_no, Some(10));
        assert_eq!(h.lines[1].kind, LineKind::Deletion);
        assert_eq!(h.lines[1].text, "let b = 2;");
        assert_eq!(h.lines[1].old_no, Some(11));
        assert_eq!(h.lines[1].new_no, None);
        assert_eq!(h.lines[2].kind, LineKind::Addition);
        assert_eq!(h.lines[2].new_no, Some(11));
        assert_eq!(h.lines[3].new_no, Some(12));
        assert_eq!(h.lines[4].old_no, Some(12));
        assert_eq!(h.lines[4].new_no, Some(13));
        assert_eq!(fd.total_changes(), 5);
    }

    #[test]
    fn round_trips_hunk_headers() {
        let fd = parse(SAMPLE);
        assert_eq!(fd.hunks[0].header(), "@@ -10,3 +10,4 @@ fn main() {");
        assert_eq!(fd.hunks[1].header(), "@@ -40,2 +41,2 @@ fn other() {");
    }

    #[test]
    fn single_line_range_omits_count() {
        assert_eq!(range(5, 1), "5");
        assert_eq!(range(5, 2), "5,2");
        assert_eq!(range(0, 0), "0,0");
    }

    #[test]
    fn detects_new_and_deleted_and_binary() {
        let new = parse("diff --git a/x b/x\nnew file mode 100644\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+hi\n");
        assert!(new.is_new);
        assert_eq!(new.old_path(), None);
        assert_eq!(new.hunks[0].old_count, 0);

        let del = parse("diff --git a/x b/x\ndeleted file mode 100644\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n");
        assert!(del.is_delete);
        assert_eq!(del.new_path(), None);

        let bin = parse("diff --git a/x.png b/x.png\nindex 1..2 100644\nBinary files a/x.png and b/x.png differ\n");
        assert!(bin.binary);
        assert!(bin.hunks.is_empty());
    }

    #[test]
    fn no_newline_marker_attaches_to_hunk() {
        let fd = parse("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n");
        let l = &fd.hunks[0].lines;
        assert_eq!(l[0].kind, LineKind::Deletion);
        assert_eq!(l[1].kind, LineKind::NoNewline);
        assert_eq!(l[1].text, "\\ No newline at end of file");
        assert_eq!(l[2].kind, LineKind::Addition);
    }

    #[test]
    fn empty_context_line_is_context_not_skipped() {
        let fd = parse("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n\n-b\n+c\n");
        let kinds: Vec<LineKind> = fd.hunks[0].lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::Context,
                LineKind::Context,
                LineKind::Deletion,
                LineKind::Addition
            ]
        );
    }

    #[test]
    fn synthesised_untracked_diff() {
        let fd = synth_new_file("notes.txt", "one\ntwo\n", false);
        assert!(fd.is_new);
        assert_eq!(fd.hunks.len(), 1);
        assert_eq!(fd.hunks[0].lines.len(), 2);
        assert_eq!(fd.hunks[0].header(), "@@ -0,0 +1,2 @@");
        assert!(fd.hunks[0].lines.iter().all(|l| l.kind == LineKind::Addition));

        let no_nl = synth_new_file("a", "solo", false);
        assert_eq!(no_nl.hunks[0].lines.len(), 2);
        assert_eq!(no_nl.hunks[0].lines[1].kind, LineKind::NoNewline);
        assert_eq!(no_nl.hunks[0].header(), "@@ -0,0 +1 @@");

        let empty = synth_new_file("a", "", false);
        assert!(empty.hunks.is_empty());
        assert!(empty.header_only);
    }
}
