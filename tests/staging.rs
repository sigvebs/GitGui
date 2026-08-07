//! End-to-end checks that partial patches actually round-trip through
//! `git apply`. These drive real temporary repositories, because the failure
//! mode that matters ("patch does not apply", or worse, silently staging the
//! wrong line) only shows up against git itself.

mod common;

use common::{locate, TempRepo, BASE};

use gitgui::git::diff;
use gitgui::git::patch::{self, ApplyMode};

/// Selects a set of (hunk, line) pairs.
fn pick(pairs: &'static [(usize, usize)]) -> impl Fn(usize, usize) -> bool {
    move |h, l| pairs.contains(&(h, l))
}

#[test]
fn stages_a_single_added_line_leaving_the_rest_unstaged() {
    let t = TempRepo::new("stage-one");
    t.write("f.txt", BASE);
    t.commit_all("base");

    // Three separate edits in one file.
    t.write("f.txt", "one\nINSERT-A\ntwo\nthree\nINSERT-B\nfour\nfive\nINSERT-C\n");

    let fd = t.unstaged_diff("f.txt");
    assert_eq!(fd.total_changes(), 3, "expected three added lines");

    // Find the line index of INSERT-B and stage only that.
    let (hi, li) = locate(&fd, "INSERT-B");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(
        t.indexed("f.txt"),
        "one\ntwo\nthree\nINSERT-B\nfour\nfive\n",
        "index should contain exactly one of the three new lines"
    );
    // Worktree untouched, and the other two are still pending.
    assert_eq!(
        t.worktree("f.txt"),
        "one\nINSERT-A\ntwo\nthree\nINSERT-B\nfour\nfive\nINSERT-C\n"
    );
    let after = t.unstaged_diff("f.txt");
    assert_eq!(after.total_changes(), 2);
}

#[test]
fn stages_a_single_deleted_line() {
    let t = TempRepo::new("stage-del");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nthree\nfive\n"); // removed two and four

    let fd = t.unstaged_diff("f.txt");
    let (hi, li) = locate(&fd, "two");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(t.indexed("f.txt"), "one\nthree\nfour\nfive\n");
}

#[test]
fn stages_one_line_of_a_replacement_pair() {
    let t = TempRepo::new("stage-replace");
    t.write("f.txt", BASE);
    t.commit_all("base");
    // A modification is a delete+add pair on adjacent lines.
    t.write("f.txt", "one\nTWO\nthree\nfour\nfive\n");

    let fd = t.unstaged_diff("f.txt");
    // Take only the addition. The unselected deletion becomes context, and git
    // emits "-two" before "+TWO", so the surviving "two" lands above "TWO".
    let (hi, li) = locate(&fd, "TWO");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(t.indexed("f.txt"), "one\ntwo\nTWO\nthree\nfour\nfive\n");

    // The other half of the pair is still pending.
    let rest = t.unstaged_diff("f.txt");
    assert_eq!(rest.total_changes(), 1);
    let p = patch::build_full_patch(&rest, ApplyMode::Forward).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();
    assert_eq!(t.indexed("f.txt"), t.worktree("f.txt"));
}

#[test]
fn unstages_a_single_line_from_the_index() {
    let t = TempRepo::new("unstage-one");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nADD-A\ntwo\nADD-B\nthree\nfour\nfive\n");
    t.repo.run(&["add", "f.txt"]).unwrap();
    assert_eq!(t.indexed("f.txt"), "one\nADD-A\ntwo\nADD-B\nthree\nfour\nfive\n");

    let fd = t.staged_diff("f.txt");
    assert_eq!(fd.total_changes(), 2);

    // Unstage ADD-A only; ADD-B must stay staged.
    let (hi, li) = locate(&fd, "ADD-A");
    let p = patch::build_patch(&fd, ApplyMode::Reverse, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Reverse, true).unwrap();

    assert_eq!(t.indexed("f.txt"), "one\ntwo\nADD-B\nthree\nfour\nfive\n");
    assert_eq!(t.head("f.txt"), BASE, "HEAD must not move");
    // Worktree still has both, so ADD-A shows up as unstaged again.
    let un = t.unstaged_diff("f.txt");
    assert_eq!(un.total_changes(), 1);
}

#[test]
fn discards_a_single_line_from_the_worktree() {
    let t = TempRepo::new("discard-one");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nKEEP\ntwo\nTOSS\nthree\nfour\nfive\n");

    let fd = t.unstaged_diff("f.txt");
    let (hi, li) = locate(&fd, "TOSS");
    let p = patch::build_patch(&fd, ApplyMode::Reverse, &|h, l| (h, l) == (hi, li)).unwrap();
    // No --cached: this rewrites the file on disk.
    t.repo.apply_patch(&p, ApplyMode::Reverse, false).unwrap();

    assert_eq!(t.worktree("f.txt"), "one\nKEEP\ntwo\nthree\nfour\nfive\n");
}

#[test]
fn stages_part_of_an_untracked_file() {
    let t = TempRepo::new("untracked");
    t.write("seed.txt", "x\n");
    t.commit_all("base");
    t.write("new.txt", "alpha\nbeta\ngamma\n");

    let content = t.worktree("new.txt");
    let fd = diff::synth_new_file("new.txt", &content, false);
    assert_eq!(fd.total_changes(), 3);

    // Stage alpha and gamma only.
    let p = patch::build_patch(&fd, ApplyMode::Forward, &pick(&[(0, 0), (0, 2)])).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(t.indexed("new.txt"), "alpha\ngamma\n");
    assert_eq!(t.worktree("new.txt"), "alpha\nbeta\ngamma\n");

    let st = t.repo.status().unwrap();
    let e = st.entries.iter().find(|e| e.path == "new.txt").unwrap();
    assert!(!e.untracked, "file is tracked once part of it is staged");
}

#[test]
fn unstages_part_of_a_newly_added_file() {
    let t = TempRepo::new("partial-new");
    t.write("seed.txt", "x\n");
    t.commit_all("base");
    t.write("new.txt", "alpha\nbeta\ngamma\n");
    t.repo.run(&["add", "new.txt"]).unwrap();

    let fd = t.staged_diff("new.txt");
    assert!(fd.is_new);

    // Unstage just beta. The create-header must be rewritten or git apply
    // refuses, since the old side is no longer empty.
    let (hi, li) = locate(&fd, "beta");
    let p = patch::build_patch(&fd, ApplyMode::Reverse, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Reverse, true).unwrap();

    assert_eq!(t.indexed("new.txt"), "alpha\ngamma\n");
}

#[test]
fn stages_part_of_a_deletion() {
    let t = TempRepo::new("partial-del");
    t.write("f.txt", BASE);
    t.commit_all("base");
    std::fs::remove_file(t.dir.join("f.txt")).unwrap();

    let fd = t.unstaged_diff("f.txt");
    assert!(fd.is_delete);
    assert_eq!(fd.total_changes(), 5);

    // Stage the removal of "two" and "four" only; the file must still exist in
    // the index with the remaining lines.
    let a = locate(&fd, "two");
    let b = locate(&fd, "four");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == a || (h, l) == b).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(t.indexed("f.txt"), "one\nthree\nfive\n");
}

#[test]
fn stages_a_line_in_a_crlf_file() {
    let t = TempRepo::new("crlf");
    t.write("f.txt", "one\r\ntwo\r\nthree\r\n");
    t.commit_all("base");
    t.write("f.txt", "one\r\nINSERT\r\ntwo\r\nthree\r\n");

    let fd = t.unstaged_diff("f.txt");
    assert!(
        fd.hunks[0].lines.iter().any(|l| l.text.ends_with('\r')),
        "parser must retain CR"
    );

    let (hi, li) = locate(&fd, "INSERT");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(t.indexed("f.txt"), "one\r\nINSERT\r\ntwo\r\nthree\r\n");
}

#[test]
fn stages_a_line_in_a_file_without_a_trailing_newline() {
    let t = TempRepo::new("no-eol");
    t.write("f.txt", "one\ntwo\nthree");
    t.commit_all("base");
    t.write("f.txt", "one\nINSERT\ntwo\nthree");

    let fd = t.unstaged_diff("f.txt");
    let (hi, li) = locate(&fd, "INSERT");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    assert_eq!(t.indexed("f.txt"), "one\nINSERT\ntwo\nthree");
}

#[test]
fn stages_one_hunk_out_of_several_far_apart() {
    let t = TempRepo::new("multi-hunk");
    let mut base = String::new();
    for i in 1..=60 {
        base.push_str(&format!("line {i}\n"));
    }
    t.write("f.txt", &base);
    t.commit_all("base");

    let edited = base
        .replace("line 5\n", "line 5\nEARLY\n")
        .replace("line 50\n", "line 50\nLATE\n");
    t.write("f.txt", &edited);

    let fd = t.unstaged_diff("f.txt");
    assert_eq!(fd.hunks.len(), 2, "edits should be far enough apart");

    // Stage the *later* hunk only. Its new-side start must fall back, since the
    // earlier hunk's inserted line is not being applied.
    let p = patch::build_hunk_patch(&fd, ApplyMode::Forward, 1).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();

    let staged = t.indexed("f.txt");
    assert!(staged.contains("LATE"));
    assert!(!staged.contains("EARLY"));
}

#[test]
fn round_trip_stage_then_unstage_restores_the_index() {
    let t = TempRepo::new("round-trip");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nA\ntwo\nB\nthree\nfour\nfive\n");

    let fd = t.unstaged_diff("f.txt");
    let (hi, li) = locate(&fd, "A");
    let p = patch::build_patch(&fd, ApplyMode::Forward, &|h, l| (h, l) == (hi, li)).unwrap();
    t.repo.apply_patch(&p, ApplyMode::Forward, true).unwrap();
    assert_ne!(t.indexed("f.txt"), BASE);

    // Now unstage the same line via the staged diff.
    let sd = t.staged_diff("f.txt");
    let p2 = patch::build_full_patch(&sd, ApplyMode::Reverse).unwrap();
    t.repo.apply_patch(&p2, ApplyMode::Reverse, true).unwrap();

    assert_eq!(t.indexed("f.txt"), BASE, "index is back to HEAD");
}
