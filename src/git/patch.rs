//! Builds a patch containing only a chosen subset of a diff's changed lines,
//! suitable for `git apply` (optionally `--cached` and/or `--reverse`).
//!
//! The transformation for lines the user did *not* select depends on which
//! direction the patch will be applied, because the side that must match the
//! target flips:
//!
//! * Forward (stage: index -> worktree patch applied to the index). The target
//!   is the patch's *old* side, which is the index. Unselected deletions still
//!   exist there and must survive, so they become context. Unselected additions
//!   do not exist there at all, so they are dropped.
//! * Reverse (unstage, or discard from the worktree). The target is the patch's
//!   *new* side. Unselected additions exist there and must survive, so they
//!   become context; unselected deletions are dropped.

use super::diff::{range, DiffLine, FileDiff, LineKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApplyMode {
    Forward,
    Reverse,
}

/// Builds a patch for the lines `selected(hunk_index, line_index)` accepts.
/// Returns `None` when the selection contains no actual changes.
pub fn build_patch(
    fd: &FileDiff,
    mode: ApplyMode,
    selected: &dyn Fn(usize, usize) -> bool,
) -> Option<String> {
    let mut body: Vec<String> = Vec::new();
    let mut delta: i64 = 0;
    let mut total_old = 0u32;
    let mut total_new = 0u32;

    for (hi, hunk) in fd.hunks.iter().enumerate() {
        let mut out: Vec<String> = Vec::new();
        let mut old_count = 0u32;
        let mut new_count = 0u32;
        let mut changes = 0usize;
        // A "\ No newline" marker is only meaningful if the line it describes
        // was itself emitted.
        let mut kept_previous = false;

        for (li, line) in hunk.lines.iter().enumerate() {
            match line.kind {
                LineKind::Context => {
                    out.push(format!(" {}", line.text));
                    old_count += 1;
                    new_count += 1;
                    kept_previous = true;
                }
                LineKind::NoNewline => {
                    if kept_previous {
                        out.push(line.text.clone());
                    }
                }
                LineKind::Addition => {
                    if selected(hi, li) {
                        out.push(format!("+{}", line.text));
                        new_count += 1;
                        changes += 1;
                        kept_previous = true;
                    } else if mode == ApplyMode::Reverse {
                        out.push(format!(" {}", line.text));
                        old_count += 1;
                        new_count += 1;
                        kept_previous = true;
                    } else {
                        kept_previous = false;
                    }
                }
                LineKind::Deletion => {
                    if selected(hi, li) {
                        out.push(format!("-{}", line.text));
                        old_count += 1;
                        changes += 1;
                        kept_previous = true;
                    } else if mode == ApplyMode::Forward {
                        out.push(format!(" {}", line.text));
                        old_count += 1;
                        new_count += 1;
                        kept_previous = true;
                    } else {
                        kept_previous = false;
                    }
                }
            }
        }

        if changes == 0 {
            continue;
        }

        // `@@ -S,0 ... @@` means "after line S", so a zero count shifts the
        // stated start by one. Normalise to a real 1-based position, do the
        // arithmetic, then convert back.
        let old_pos = if hunk.old_count == 0 {
            hunk.old_start as i64 + 1
        } else {
            hunk.old_start as i64
        };
        let new_pos = old_pos + delta;

        let old_field = if old_count == 0 { old_pos - 1 } else { old_pos };
        let new_field = if new_count == 0 { new_pos - 1 } else { new_pos };

        body.push(format!(
            "@@ -{} +{} @@{}",
            range(old_field.max(0) as u32, old_count),
            range(new_field.max(0) as u32, new_count),
            hunk.section
        ));
        body.extend(out);

        delta += new_count as i64 - old_count as i64;
        total_old += old_count;
        total_new += new_count;
    }

    if body.is_empty() {
        return None;
    }

    let header = rewrite_header(fd, total_old, total_new);
    let mut patch = String::new();
    for line in header.iter().chain(body.iter()) {
        patch.push_str(line);
        patch.push('\n');
    }
    Some(patch)
}

/// Whole-file patch, used for staging a file that has no diff subtleties.
pub fn build_full_patch(fd: &FileDiff, mode: ApplyMode) -> Option<String> {
    build_patch(fd, mode, &|_, _| true)
}

pub fn build_hunk_patch(fd: &FileDiff, mode: ApplyMode, hunk: usize) -> Option<String> {
    build_patch(fd, mode, &|h, _| h == hunk)
}

/// A create/delete header stops being true once only part of the change is
/// taken: a partly-unstaged new file still has content on the old side, and a
/// partly-staged deletion still has content on the new side.
fn rewrite_header(fd: &FileDiff, total_old: u32, total_new: u32) -> Vec<String> {
    let partial_new = fd.is_new && total_old > 0;
    let partial_delete = fd.is_delete && total_new > 0;
    if !partial_new && !partial_delete {
        return fd.header.clone();
    }

    // Both sides name the same path in this situation, so whichever line is
    // not /dev/null supplies it.
    let path = fd
        .new_path()
        .or_else(|| fd.old_path())
        .unwrap_or_else(|| "unknown".to_string());

    let mut out = Vec::with_capacity(fd.header.len());
    for line in &fd.header {
        if line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            // Blob hashes no longer describe either side of a partial patch.
            || line.starts_with("index ")
        {
            continue;
        }
        if partial_new && line == "--- /dev/null" {
            out.push(format!("--- a/{path}"));
            continue;
        }
        if partial_delete && line == "+++ /dev/null" {
            out.push(format!("+++ b/{path}"));
            continue;
        }
        out.push(line.clone());
    }
    out
}

/// Line indices of every change in a hunk, for "select whole hunk".
pub fn hunk_change_lines(lines: &[DiffLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.is_change())
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::parse;

    const TWO_HUNKS: &str = "\
diff --git a/f b/f
index 1111111..2222222 100644
--- a/f
+++ b/f
@@ -10,3 +10,4 @@ ctx heading
 keep1
-old2
+new2
+new3
 keep4
@@ -40,2 +41,2 @@
 keep5
-old6
+new6
";

    fn only(hunk: usize, lines: &[usize]) -> impl Fn(usize, usize) -> bool + '_ {
        move |h, l| h == hunk && lines.contains(&l)
    }

    #[test]
    fn full_forward_patch_reproduces_original() {
        let fd = parse(TWO_HUNKS);
        let p = build_full_patch(&fd, ApplyMode::Forward).unwrap();
        assert_eq!(p, TWO_HUNKS);
    }

    #[test]
    fn full_reverse_patch_is_also_the_original() {
        // Everything is selected, so no line needs converting either way.
        let fd = parse(TWO_HUNKS);
        let p = build_full_patch(&fd, ApplyMode::Reverse).unwrap();
        assert_eq!(p, TWO_HUNKS);
    }

    #[test]
    fn forward_drops_unselected_additions_and_keeps_deletions_as_context() {
        let fd = parse(TWO_HUNKS);
        // hunk 0 lines: 0=keep1 1=-old2 2=+new2 3=+new3 4=keep4
        // Select only the deletion.
        let p = build_patch(&fd, ApplyMode::Forward, &only(0, &[1])).unwrap();
        assert_eq!(
            p,
            "\
diff --git a/f b/f
index 1111111..2222222 100644
--- a/f
+++ b/f
@@ -10,3 +10,2 @@ ctx heading
 keep1
-old2
 keep4
"
        );
    }

    #[test]
    fn forward_selecting_one_addition_converts_the_deletion_to_context() {
        let fd = parse(TWO_HUNKS);
        let p = build_patch(&fd, ApplyMode::Forward, &only(0, &[2])).unwrap();
        assert_eq!(
            p,
            "\
diff --git a/f b/f
index 1111111..2222222 100644
--- a/f
+++ b/f
@@ -10,3 +10,4 @@ ctx heading
 keep1
 old2
+new2
 keep4
"
        );
    }

    #[test]
    fn reverse_selecting_one_addition_converts_the_other_addition_to_context() {
        let fd = parse(TWO_HUNKS);
        // Unstage just "+new2": "+new3" must stay in the index, and "-old2"
        // must not come back.
        let p = build_patch(&fd, ApplyMode::Reverse, &only(0, &[2])).unwrap();
        assert_eq!(
            p,
            "\
diff --git a/f b/f
index 1111111..2222222 100644
--- a/f
+++ b/f
@@ -10,3 +10,4 @@ ctx heading
 keep1
+new2
 new3
 keep4
"
        );
    }

    #[test]
    fn second_hunk_new_start_accounts_for_the_first() {
        let fd = parse(TWO_HUNKS);

        // Only hunk 1 staged: hunk 0's +1 line never happens, so the new-side
        // start falls back to the old-side start.
        let p = build_patch(&fd, ApplyMode::Forward, &|h, _| h == 1).unwrap();
        assert!(p.contains("@@ -40,2 +40,2 @@"), "got: {p}");

        // Both hunks staged: hunk 0 nets +1, so hunk 1 shifts to 41 as in the
        // original diff.
        let p = build_full_patch(&fd, ApplyMode::Forward).unwrap();
        assert!(p.contains("@@ -40,2 +41,2 @@"), "got: {p}");
    }

    #[test]
    fn empty_selection_yields_nothing() {
        let fd = parse(TWO_HUNKS);
        assert!(build_patch(&fd, ApplyMode::Forward, &|_, _| false).is_none());
        // Selecting a context line is not a change either.
        assert!(build_patch(&fd, ApplyMode::Forward, &only(0, &[0])).is_none());
    }

    #[test]
    fn hunk_with_no_selected_change_is_omitted_entirely() {
        let fd = parse(TWO_HUNKS);
        let p = build_patch(&fd, ApplyMode::Forward, &only(1, &[1])).unwrap();
        assert_eq!(p.matches("@@ ").count(), 1);
        assert!(!p.contains("new2"));
    }

    const NEW_FILE: &str = "\
diff --git a/n b/n
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/n
@@ -0,0 +1,3 @@
+one
+two
+three
";

    #[test]
    fn partial_forward_stage_of_new_file_keeps_the_create_header() {
        let fd = parse(NEW_FILE);
        // Old side stays empty, so this is still a legitimate new-file patch.
        let p = build_patch(&fd, ApplyMode::Forward, &only(0, &[0, 2])).unwrap();
        assert!(p.contains("new file mode 100644"), "got: {p}");
        assert!(p.contains("--- /dev/null"));
        assert!(p.contains("@@ -0,0 +1,2 @@"), "got: {p}");
        assert!(p.contains("+one") && p.contains("+three") && !p.contains("+two"));
    }

    #[test]
    fn partial_reverse_unstage_of_new_file_rewrites_the_create_header() {
        let fd = parse(NEW_FILE);
        // Keeping "two" and "three" in the index means the old side is no
        // longer empty, so this cannot claim to create the file.
        let p = build_patch(&fd, ApplyMode::Reverse, &only(0, &[0])).unwrap();
        assert!(!p.contains("new file mode"), "got: {p}");
        assert!(!p.contains("/dev/null"), "got: {p}");
        assert!(!p.contains("index 0000000"), "got: {p}");
        assert!(p.contains("--- a/n") && p.contains("+++ b/n"), "got: {p}");
        assert!(p.contains("@@ -1,2 +1,3 @@"), "got: {p}");
        assert!(p.contains("+one") && p.contains(" two") && p.contains(" three"));
    }

    #[test]
    fn fully_reversed_new_file_keeps_the_create_header() {
        let fd = parse(NEW_FILE);
        let p = build_full_patch(&fd, ApplyMode::Reverse).unwrap();
        assert!(p.contains("new file mode 100644"));
        assert!(p.contains("--- /dev/null"));
    }

    const DELETED_FILE: &str = "\
diff --git a/d b/d
deleted file mode 100644
index 4444444..0000000
--- a/d
+++ /dev/null
@@ -1,3 +0,0 @@
-one
-two
-three
";

    #[test]
    fn partial_forward_stage_of_deletion_rewrites_the_delete_header() {
        let fd = parse(DELETED_FILE);
        let p = build_patch(&fd, ApplyMode::Forward, &only(0, &[0])).unwrap();
        assert!(!p.contains("deleted file mode"), "got: {p}");
        assert!(!p.contains("/dev/null"), "got: {p}");
        assert!(p.contains("+++ b/d"), "got: {p}");
        assert!(p.contains("@@ -1,3 +1,2 @@"), "got: {p}");
        assert!(p.contains("-one") && p.contains(" two") && p.contains(" three"));
    }

    #[test]
    fn partial_reverse_of_deletion_stays_a_delete_patch() {
        let fd = parse(DELETED_FILE);
        // New side is still empty, so the create-on-reverse header holds.
        let p = build_patch(&fd, ApplyMode::Reverse, &only(0, &[0])).unwrap();
        assert!(p.contains("deleted file mode"), "got: {p}");
        assert!(p.contains("+++ /dev/null"));
        assert!(p.contains("@@ -1 +0,0 @@"), "got: {p}");
    }

    #[test]
    fn fully_staged_deletion_keeps_the_delete_header() {
        let fd = parse(DELETED_FILE);
        let p = build_full_patch(&fd, ApplyMode::Forward).unwrap();
        assert!(p.contains("deleted file mode 100644"));
        assert!(p.contains("+++ /dev/null"));
        assert!(p.contains("@@ -1,3 +0,0 @@"), "got: {p}");
    }

    #[test]
    fn crlf_content_is_preserved_byte_for_byte() {
        let src = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n ctx\r\n-old\r\n+new\r\n";
        let fd = parse(src);
        assert_eq!(fd.hunks[0].lines[0].text, "ctx\r");
        let p = build_full_patch(&fd, ApplyMode::Forward).unwrap();
        assert!(p.contains("-old\r\n"), "CR must survive: {p:?}");
        assert_eq!(p, src);
    }

    #[test]
    fn no_newline_marker_is_dropped_with_its_line() {
        let src = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n";
        let fd = parse(src);
        // Take only the addition: the deletion becomes context and keeps its
        // marker; selecting the "+c" keeps the second marker too.
        let p = build_patch(&fd, ApplyMode::Forward, &|_, l| l == 3).unwrap();
        assert_eq!(p.matches("No newline").count(), 2, "got: {p}");

        // Take only the deletion: the dropped addition must not leave its
        // marker behind.
        let p = build_patch(&fd, ApplyMode::Forward, &|_, l| l == 1).unwrap();
        assert_eq!(p.matches("No newline").count(), 1, "got: {p}");
    }

    #[test]
    fn mid_file_pure_insertion_positions_correctly() {
        let src = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -10,0 +11,2 @@\n+x\n+y\n";
        let fd = parse(src);
        let p = build_patch(&fd, ApplyMode::Forward, &|_, l| l == 0).unwrap();
        assert!(p.contains("@@ -10,0 +11 @@"), "got: {p}");
    }
}
