//! Drives `App` the way the UI does, plus a headless run of the real widget
//! tree. Together these cover everything a mouse click reaches except the
//! pixel hit-test itself.

mod common;

use common::{TempRepo, BASE};

use gitgui::app::{App, Confirm, Mode, Op, Pane, Row, TextPos};

fn app_on(t: &TempRepo) -> App {
    let mut app = App::default();
    app.open(t.dir.clone());
    assert!(app.repo.is_some(), "repo should open: {:?}", app.error);
    app
}

/// Row index of the changed line whose text matches.
fn row_of(app: &App, needle: &str) -> usize {
    let d = app.diff.as_ref().expect("a diff is loaded");
    for (i, r) in d.rows.iter().enumerate() {
        if let Row::Line { hunk, line } = r {
            let l = &d.fd.hunks[*hunk].lines[*line];
            if l.is_change() && l.text == needle {
                return i;
            }
        }
    }
    panic!("no changed row {needle:?}");
}

#[test]
fn selecting_a_line_and_staging_moves_only_that_line() {
    let t = TempRepo::new("app-stage");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    assert_eq!(app.status.unstaged().len(), 1);
    assert_eq!(app.status.staged().len(), 0);

    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(row);
    assert_eq!(app.diff.as_ref().unwrap().selected_change_count(), 1);

    app.apply_lines(Op::Stage);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(t.indexed("f.txt"), "one\nAAA\ntwo\nthree\nfour\nfive\n");
    // The file is now in both panes: one line staged, one still pending.
    assert_eq!(app.status.staged().len(), 1);
    assert_eq!(app.status.unstaged().len(), 1);
}

#[test]
fn shift_range_selection_stages_a_contiguous_block() {
    let t = TempRepo::new("app-range");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\nCCC\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");

    let first = row_of(&app, "AAA");
    let last = row_of(&app, "BBB");
    {
        let d = app.diff.as_mut().unwrap();
        d.select_only(first);
        d.extend_to(last); // shift-click
        assert_eq!(d.selected_change_count(), 2);
    }

    app.apply_lines(Op::Stage);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.indexed("f.txt"), "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n");
}

#[test]
fn cmd_click_toggle_builds_a_discontiguous_selection() {
    let t = TempRepo::new("app-toggle");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nCCC\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");

    let a = row_of(&app, "AAA");
    let c = row_of(&app, "CCC");
    {
        let d = app.diff.as_mut().unwrap();
        d.select_only(a);
        d.toggle(c);
        assert_eq!(d.selected_change_count(), 2);
        // Toggling again removes it.
        d.toggle(c);
        assert_eq!(d.selected_change_count(), 1);
        d.toggle(c);
    }

    app.apply_lines(Op::Stage);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(
        t.indexed("f.txt"),
        "one\nAAA\ntwo\nthree\nCCC\nfour\nfive\n",
        "BBB must stay unstaged"
    );
}

#[test]
fn unstaging_a_selected_line_returns_it_to_the_working_tree_only() {
    let t = TempRepo::new("app-unstage");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");
    t.repo.run(&["add", "f.txt"]).unwrap();

    let mut app = app_on(&t);
    app.select_file(Pane::Staged, "f.txt");
    let row = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(row);

    app.apply_lines(Op::Unstage);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(t.indexed("f.txt"), "one\ntwo\nBBB\nthree\nfour\nfive\n");
    assert_eq!(t.head("f.txt"), BASE, "HEAD must not move");
}

#[test]
fn staging_a_hunk_takes_every_change_in_it() {
    let t = TempRepo::new("app-hunk");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    app.apply_hunk(Op::Stage, 0);

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.indexed("f.txt"), t.worktree("f.txt"));
    assert_eq!(
        app.status.unstaged().len(),
        0,
        "nothing left pending once the only hunk is staged"
    );
}

#[test]
fn staging_with_no_selection_falls_back_to_the_cursor_hunk() {
    let t = TempRepo::new("app-fallback");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    {
        let d = app.diff.as_mut().unwrap();
        d.sel.clear();
        d.cursor = row;
    }
    app.apply_lines(Op::Stage);

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.indexed("f.txt"), t.worktree("f.txt"));
}

#[test]
fn discarding_lines_applies_immediately_without_a_prompt() {
    let t = TempRepo::new("app-discard");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(row);

    app.apply_lines(Op::Discard);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert!(app.confirm.is_none(), "line discards must not prompt");
    assert_eq!(
        t.worktree("f.txt"),
        "one\ntwo\nBBB\nthree\nfour\nfive\n",
        "only the selected line is reverted"
    );
}

#[test]
fn discarding_a_hunk_applies_immediately_without_a_prompt() {
    let t = TempRepo::new("app-discard-hunk");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    app.apply_hunk(Op::Discard, 0);

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert!(app.confirm.is_none(), "hunk discards must not prompt");
    assert_eq!(t.worktree("f.txt"), BASE, "hunk reverted");
}

#[test]
fn undo_restores_discarded_lines() {
    let t = TempRepo::new("undo-lines");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let edited = "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n";
    t.write("f.txt", edited);

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(row);

    app.apply_lines(Op::Discard);
    assert_eq!(t.worktree("f.txt"), "one\ntwo\nBBB\nthree\nfour\nfive\n");
    assert!(app.undo_label().is_some(), "undo should be offered");

    app.undo_last_discard();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("f.txt"), edited, "the discarded line is back");
    assert!(app.undo_label().is_none(), "stack is empty again");
}

#[test]
fn undo_walks_back_several_discards_in_turn() {
    let t = TempRepo::new("undo-stack");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let edited = "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n";
    t.write("f.txt", edited);

    let mut app = app_on(&t);

    // Discard AAA, then BBB, as two separate operations.
    app.select_file(Pane::Unstaged, "f.txt");
    let a = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(a);
    app.apply_lines(Op::Discard);

    app.select_file(Pane::Unstaged, "f.txt");
    let b = row_of(&app, "BBB");
    app.diff.as_mut().unwrap().select_only(b);
    app.apply_lines(Op::Discard);

    assert_eq!(t.worktree("f.txt"), BASE, "both gone");
    assert_eq!(app.undo.len(), 2);

    app.undo_last_discard();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("f.txt"), "one\ntwo\nBBB\nthree\nfour\nfive\n");

    app.undo_last_discard();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("f.txt"), edited, "back to where we started");
    assert!(app.undo.is_empty());
}

#[test]
fn undo_restores_a_discarded_hunk() {
    let t = TempRepo::new("undo-hunk");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let edited = "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n";
    t.write("f.txt", edited);

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    app.apply_hunk(Op::Discard, 0);
    assert_eq!(t.worktree("f.txt"), BASE);

    app.undo_last_discard();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("f.txt"), edited);
}

#[test]
fn undo_restores_a_discarded_tracked_file() {
    let t = TempRepo::new("undo-file");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let edited = "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n";
    t.write("f.txt", edited);

    let mut app = app_on(&t);
    app.ask_discard_file("f.txt");
    app.confirm_yes();
    assert_eq!(t.worktree("f.txt"), BASE, "all changes gone");

    app.undo_last_discard();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("f.txt"), edited);
}

#[test]
fn undo_recreates_a_deleted_untracked_file_including_binary() {
    let t = TempRepo::new("undo-untracked");
    t.write("seed.txt", "x\n");
    t.commit_all("base");

    // A binary payload has no usable diff, so this exercises the raw-bytes path.
    let blob: Vec<u8> = vec![0x89, 0x50, 0x4e, 0x47, 0x00, 0x01, 0x02, 0xff, 0x00, 0xfe];
    std::fs::write(t.dir.join("logo.png"), &blob).unwrap();

    let mut app = app_on(&t);
    app.ask_discard_file("logo.png");
    app.confirm_yes();
    assert!(
        !t.dir.join("logo.png").exists(),
        "untracked file should be deleted"
    );
    assert!(
        app.undo_label().is_some(),
        "binary untracked files must still be recoverable"
    );

    app.undo_last_discard();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(
        std::fs::read(t.dir.join("logo.png")).unwrap(),
        blob,
        "bytes restored exactly"
    );
}

#[test]
fn undo_keeps_the_entry_when_it_cannot_be_applied() {
    let t = TempRepo::new("undo-fail");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(row);
    app.apply_lines(Op::Discard);

    // Rewrite the file so the stored patch no longer applies.
    t.write("f.txt", "completely\ndifferent\ncontent\n");
    app.undo_last_discard();

    assert!(app.error.is_some(), "should report the failure");
    assert!(
        app.undo_label().is_some(),
        "the entry must survive so the content is not silently dropped"
    );
}

#[test]
fn staging_and_unstaging_do_not_create_undo_entries() {
    let t = TempRepo::new("undo-none");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    app.apply_hunk(Op::Stage, 0);
    app.select_file(Pane::Staged, "f.txt");
    app.apply_hunk(Op::Unstage, 0);

    assert!(
        app.undo.is_empty(),
        "these are not destructive, so nothing to take back"
    );
}

#[test]
fn discarding_a_whole_file_still_asks_first() {
    let t = TempRepo::new("app-discard-file");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");
    let before = t.worktree("f.txt");

    let mut app = app_on(&t);
    app.ask_discard_file("f.txt");
    assert!(
        matches!(app.confirm, Some(Confirm::DiscardFile { .. })),
        "whole-file discard is coarse enough to keep a prompt"
    );
    assert_eq!(t.worktree("f.txt"), before, "untouched before confirm");

    app.confirm_yes();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("f.txt"), BASE);
}

#[test]
fn cancelling_a_file_discard_changes_nothing() {
    let t = TempRepo::new("app-cancel");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");
    let before = t.worktree("f.txt");

    let mut app = app_on(&t);
    app.ask_discard_file("f.txt");
    app.confirm = None; // user hit Cancel

    assert_eq!(t.worktree("f.txt"), before);
}

#[test]
fn untracked_file_shows_as_all_additions_and_stages_partially() {
    let t = TempRepo::new("app-untracked");
    t.write("seed.txt", "x\n");
    t.commit_all("base");
    t.write("new.txt", "alpha\nbeta\ngamma\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "new.txt");
    {
        let d = app.diff.as_ref().unwrap();
        assert!(d.untracked);
        assert_eq!(d.fd.total_changes(), 3);
    }

    let a = row_of(&app, "alpha");
    let g = row_of(&app, "gamma");
    {
        let d = app.diff.as_mut().unwrap();
        d.select_only(a);
        d.toggle(g);
    }
    app.apply_lines(Op::Stage);

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.indexed("new.txt"), "alpha\ngamma\n");
}

#[test]
fn committing_clears_the_message_and_empties_the_staged_pane() {
    let t = TempRepo::new("app-commit");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    app.apply_hunk(Op::Stage, 0);

    assert!(!app.can_commit(), "no message yet");
    app.commit_msg = "Add AAA".to_string();
    assert!(app.can_commit());

    app.commit();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert!(app.commit_msg.is_empty());
    assert_eq!(app.status.staged().len(), 0);
    assert_eq!(t.head("f.txt"), "one\nAAA\ntwo\nthree\nfour\nfive\n");
}

#[test]
fn commit_is_refused_with_nothing_staged() {
    let t = TempRepo::new("app-nostage");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.commit_msg = "nope".into();
    assert!(!app.can_commit());
    app.commit();
    assert!(app.error.is_some(), "should complain about an empty index");
}

#[test]
fn selection_is_dropped_when_the_file_leaves_the_pane() {
    let t = TempRepo::new("app-vanish");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    assert!(app.diff.is_some());

    // Staging the whole hunk empties the unstaged side for this file.
    app.apply_hunk(Op::Stage, 0);
    assert!(app.sel_file.is_none(), "selection should clear");
    assert!(app.diff.is_none());
}

#[test]
fn move_cursor_with_shift_grows_the_selection() {
    let t = TempRepo::new("app-keys");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\nCCC\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let start = row_of(&app, "AAA");

    let d = app.diff.as_mut().unwrap();
    d.select_only(start);
    assert_eq!(d.selected_change_count(), 1);
    d.move_cursor(1, true);
    assert_eq!(d.selected_change_count(), 2);
    d.move_cursor(1, true);
    assert_eq!(d.selected_change_count(), 3);
    // Without shift the selection collapses to the new row.
    d.move_cursor(1, false);
    assert!(d.selected_change_count() <= 1);
}

#[test]
fn select_all_covers_every_change_but_no_context() {
    let t = TempRepo::new("app-selectall");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let total = app.diff.as_ref().unwrap().fd.total_changes();

    let d = app.diff.as_mut().unwrap();
    d.select_all();
    assert_eq!(d.selected_change_count(), total);
    assert_eq!(d.sel.len(), total, "context rows must not be selected");
}

// ---- headless run of the actual widget tree ---------------------------------

fn run_frames(app: &mut App, ctx: &egui::Context, n: usize) {
    for _ in 0..n {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 840.0),
            )),
            ..Default::default()
        };
        // Panics here would be id collisions, layout assertions or bad indexing.
        let _ = ctx.run(input, |ctx| gitgui::ui::draw(ctx, app));
    }
}

#[test]
fn the_real_ui_renders_every_state_without_panicking() {
    let t = TempRepo::new("ui-render");
    t.write("f.txt", BASE);
    t.write("keep.txt", "kept\n");
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");
    t.write("untracked.txt", "fresh\nlines\n");
    t.write("bin.dat", "a\0b\0c");
    std::fs::remove_file(t.dir.join("keep.txt")).unwrap();
    t.repo.run(&["add", "f.txt"]).unwrap();
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nCCC\nfour\nfive\n");

    let ctx = egui::Context::default();
    let mut app = app_on(&t);

    // No file selected.
    run_frames(&mut app, &ctx, 2);

    // Every kind of entry, in both panes.
    for (pane, path) in [
        (Pane::Unstaged, "f.txt"),
        (Pane::Staged, "f.txt"),
        (Pane::Unstaged, "untracked.txt"),
        (Pane::Unstaged, "bin.dat"),
        (Pane::Unstaged, "keep.txt"),
        (Pane::Staged, "keep.txt"),
    ] {
        app.select_file(pane, path);
        run_frames(&mut app, &ctx, 2);
        if let Some(d) = app.diff.as_mut() {
            d.select_all();
        }
        run_frames(&mut app, &ctx, 2);
    }

    app.set_amend(true);
    run_frames(&mut app, &ctx, 2);
    if let Some(p) = app.staged().first().map(|e| e.path.clone()) {
        app.select_file(Pane::Staged, &p);
        run_frames(&mut app, &ctx, 2);
    }
    app.set_amend(false);
    run_frames(&mut app, &ctx, 2);

    app.hide_untracked = true;
    run_frames(&mut app, &ctx, 2);
    app.hide_untracked = false;
    run_frames(&mut app, &ctx, 2);

    // The merge banner and its confirmation.
    app.merging = Some("side".to_string());
    run_frames(&mut app, &ctx, 2);
    app.confirm = Some(Confirm::AbortMerge);
    run_frames(&mut app, &ctx, 2);
    app.confirm = None;
    app.merging = None;
    run_frames(&mut app, &ctx, 2);

    // Modal states.
    app.ask_discard_file("f.txt");
    assert!(app.confirm.is_some());
    run_frames(&mut app, &ctx, 2);
    app.confirm = None;

    app.show_new_branch = true;
    run_frames(&mut app, &ctx, 2);
    app.show_new_branch = false;

    app.error = Some("a long error message ".repeat(20));
    app.info = None;
    run_frames(&mut app, &ctx, 2);
    app.error = None;

    // Status bar with the Undo button present, which needs a real discard.
    app.select_file(Pane::Unstaged, "f.txt");
    if let Some(d) = app.diff.as_mut() {
        d.select_all();
    }
    app.apply_lines(Op::Discard);
    assert!(app.undo_label().is_some(), "undo entry expected");
    run_frames(&mut app, &ctx, 2);

    app.undo_last_discard();
    run_frames(&mut app, &ctx, 2);
}

#[test]
fn the_welcome_screen_renders_without_a_repo() {
    let ctx = egui::Context::default();
    let mut app = App::default();
    app.error = Some("not a git repository".into());
    run_frames(&mut app, &ctx, 3);
    assert!(app.repo.is_none());
}

fn staged_paths(app: &App) -> Vec<String> {
    let mut v: Vec<String> = app.staged().iter().map(|e| e.path.clone()).collect();
    v.sort();
    v
}

fn commit_count(t: &TempRepo) -> usize {
    t.repo
        .run(&["rev-list", "--count", "HEAD"])
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn amendable(tag: &str) -> TempRepo {
    let t = TempRepo::new(tag);
    t.write("f.txt", BASE);
    t.write("keep.txt", "untouched\n");
    t.commit_all("base");
    t.write("f.txt", "one\nCOMMITTED\nthree\nfour\nfive\n");
    t.commit_all("second");
    t
}

#[test]
fn amend_lists_the_files_the_commit_being_amended_touched() {
    let t = amendable("amend-list");
    let mut app = app_on(&t);
    assert!(
        staged_paths(&app).is_empty(),
        "nothing is staged on top of HEAD yet"
    );

    app.set_amend(true);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert!(app.amend);
    assert_eq!(
        staged_paths(&app),
        vec!["f.txt"],
        "the commit under amendment changed f.txt, so it belongs in the pane"
    );
}

#[test]
fn amend_shows_the_commit_and_newly_staged_work_together() {
    let t = amendable("amend-both");
    t.write("g.txt", "brand new\n");
    t.git(&["add", "g.txt"]);

    let mut app = app_on(&t);
    assert_eq!(
        staged_paths(&app),
        vec!["g.txt"],
        "against HEAD only the new file is staged"
    );

    app.set_amend(true);
    assert_eq!(
        staged_paths(&app),
        vec!["f.txt", "g.txt"],
        "amending must show the commit's own change alongside the staged one"
    );
}

#[test]
fn amend_diff_is_measured_against_the_commits_parent() {
    let t = amendable("amend-diff");
    let mut app = app_on(&t);
    app.set_amend(true);
    app.select_file(Pane::Staged, "f.txt");

    let d = app.diff.as_ref().expect("a diff is loaded");
    let has_committed_line = d
        .fd
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .any(|l| l.is_change() && l.text == "COMMITTED");
    assert!(
        has_committed_line,
        "the diff should carry the committed change, got {:#?}",
        d.fd
    );
}

#[test]
fn leaving_amend_narrows_the_pane_back_to_the_index() {
    let t = amendable("amend-off");
    let mut app = app_on(&t);

    app.set_amend(true);
    assert_eq!(staged_paths(&app), vec!["f.txt"]);
    app.select_file(Pane::Staged, "f.txt");
    assert!(app.diff.is_some());

    app.set_amend(false);
    assert!(!app.amend);
    assert!(
        staged_paths(&app).is_empty(),
        "with the index clean the pane is empty again"
    );
    assert!(
        app.sel_file.is_none(),
        "the selected row is gone, so its diff must go too"
    );
}

#[test]
fn unstaging_a_file_while_amending_drops_it_from_the_commit() {
    let t = amendable("amend-unstage");
    let mut app = app_on(&t);
    app.set_amend(true);

    app.unstage_file("f.txt");
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(
        t.indexed("f.txt"),
        BASE,
        "the index should fall back to the parent commit's content"
    );
    assert!(
        staged_paths(&app).is_empty(),
        "the amended commit no longer changes anything"
    );
    assert_eq!(
        app.status.unstaged().len(),
        1,
        "the change is not lost, it moves to the unstaged pane"
    );
    assert_eq!(t.worktree("f.txt"), "one\nCOMMITTED\nthree\nfour\nfive\n");
}

#[test]
fn unstaging_one_line_while_amending_rewrites_part_of_the_commit() {
    let t = TempRepo::new("amend-unstage-line");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");
    t.commit_all("second");

    let mut app = app_on(&t);
    app.set_amend(true);
    app.select_file(Pane::Staged, "f.txt");

    let row = row_of(&app, "AAA");
    app.diff.as_mut().unwrap().select_only(row);
    app.apply_lines(Op::Unstage);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(
        t.indexed("f.txt"),
        "one\ntwo\nBBB\nthree\nfour\nfive\n",
        "only the selected line should leave the commit"
    );
}

#[test]
fn amending_a_root_commit_lists_its_whole_tree() {
    let t = TempRepo::new("amend-root");
    t.write("a.txt", "a\n");
    t.write("b.txt", "b\n");
    t.commit_all("root");

    let mut app = app_on(&t);
    app.set_amend(true);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(
        staged_paths(&app),
        vec!["a.txt", "b.txt"],
        "a root commit has no parent, so everything in it is its content"
    );
}

#[test]
fn amend_is_refused_before_the_first_commit() {
    let t = TempRepo::new("amend-unborn");
    t.write("f.txt", BASE);

    let mut app = app_on(&t);
    app.set_amend(true);
    assert!(!app.amend, "there is nothing to amend on an unborn branch");
    assert!(app.error.is_some(), "the refusal should be explained");
    assert!(staged_paths(&app).is_empty());
}

#[test]
fn committing_an_amend_rewrites_rather_than_adds_a_commit() {
    let t = amendable("amend-commit");
    t.write("g.txt", "brand new\n");
    t.git(&["add", "g.txt"]);

    let before = commit_count(&t);
    let mut app = app_on(&t);
    app.set_amend(true);
    assert_eq!(
        app.commit_msg.trim(),
        "second",
        "the message should be prefilled from the commit being amended"
    );

    app.commit();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(commit_count(&t), before, "amending must not add a commit");
    assert_eq!(t.head("f.txt"), "one\nCOMMITTED\nthree\nfour\nfive\n");
    assert_eq!(t.head("g.txt"), "brand new\n");
    assert!(!app.amend, "amend mode ends with the commit");
    assert!(staged_paths(&app).is_empty());
}

#[test]
fn amending_a_rename_shows_both_names_and_can_be_undone() {
    let t = TempRepo::new("amend-rename");
    t.write("old.txt", BASE);
    t.commit_all("base");
    t.git(&["mv", "old.txt", "new.txt"]);
    t.commit_all("rename");

    let mut app = app_on(&t);
    app.set_amend(true);
    assert_eq!(staged_paths(&app), vec!["new.txt"]);

    let entry = app
        .staged()
        .into_iter()
        .find(|e| e.path == "new.txt")
        .expect("the renamed file is in the pane");
    assert_eq!(
        entry.orig_path.as_deref(),
        Some("old.txt"),
        "the pane should know where the content came from"
    );

    app.unstage_file("new.txt");
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(
        t.indexed("old.txt"),
        BASE,
        "unstaging a rename has to put the original name back"
    );
    assert!(
        t.repo.run(&["show", ":new.txt"]).is_err(),
        "and drop the new name from the index"
    );
}

#[test]
fn hiding_untracked_removes_only_the_new_files() {
    let t = TempRepo::new("hide-untracked");
    t.write("tracked.txt", BASE);
    t.commit_all("base");
    t.write("tracked.txt", "one
AAA
two
three
four
five
");
    t.write("brand-new.txt", "never committed
");

    let mut app = app_on(&t);
    let mut shown: Vec<String> = app.unstaged().iter().map(|e| e.path.clone()).collect();
    shown.sort();
    assert_eq!(shown, vec!["brand-new.txt", "tracked.txt"]);

    app.set_hide_untracked(true);
    let shown: Vec<String> = app.unstaged().iter().map(|e| e.path.clone()).collect();
    assert_eq!(
        shown,
        vec!["tracked.txt"],
        "only the edit to a tracked file should remain"
    );
    assert_eq!(
        app.status.unstaged().len(),
        2,
        "hiding is a view filter, not a change to what git reports"
    );
}

#[test]
fn hiding_untracked_keeps_deletions_and_other_tracked_states() {
    let t = TempRepo::new("hide-untracked-states");
    t.write("gone.txt", "bye
");
    t.write("edited.txt", BASE);
    t.commit_all("base");
    std::fs::remove_file(t.dir.join("gone.txt")).unwrap();
    t.write("edited.txt", "one
AAA
two
three
four
five
");
    t.write("fresh.txt", "new
");

    let mut app = app_on(&t);
    app.set_hide_untracked(true);

    let mut shown: Vec<String> = app.unstaged().iter().map(|e| e.path.clone()).collect();
    shown.sort();
    assert_eq!(
        shown,
        vec!["edited.txt", "gone.txt"],
        "a deleted tracked file is a change, so it stays"
    );
}

#[test]
fn hiding_untracked_drops_a_selected_untracked_file() {
    let t = TempRepo::new("hide-untracked-sel");
    t.write("tracked.txt", BASE);
    t.commit_all("base");
    t.write("brand-new.txt", "never committed
");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "brand-new.txt");
    assert!(app.diff.is_some(), "the untracked file's contents are shown");

    app.set_hide_untracked(true);
    assert!(
        app.sel_file.is_none(),
        "its row is gone, so it should not stay selected"
    );
    assert!(app.diff.is_none());
}

#[test]
fn hiding_untracked_leaves_a_selected_tracked_file_alone() {
    let t = TempRepo::new("hide-untracked-keep");
    t.write("tracked.txt", BASE);
    t.commit_all("base");
    t.write("tracked.txt", "one
AAA
two
three
four
five
");
    t.write("brand-new.txt", "never committed
");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "tracked.txt");

    app.set_hide_untracked(true);
    assert_eq!(
        app.sel_file.as_ref().map(|s| s.path.clone()),
        Some("tracked.txt".to_string()),
        "filtering the list must not disturb what is being viewed"
    );
    assert!(app.diff.is_some());
}

#[test]
fn hidden_untracked_files_are_still_staged_by_stage_changed() {
    let t = TempRepo::new("hide-untracked-inert");
    t.write("tracked.txt", BASE);
    t.commit_all("base");
    t.write("tracked.txt", "one
AAA
two
three
four
five
");
    t.write("brand-new.txt", "never committed
");

    let mut app = app_on(&t);
    app.set_hide_untracked(true);
    app.stage_changed();

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("brand-new.txt"), "never committed
");
    assert_eq!(
        app.unstaged().len(),
        0,
        "the tracked edit was staged and the untracked file stays hidden"
    );
    assert_eq!(
        app.status.unstaged().len(),
        1,
        "git still reports the untracked file"
    );
}

#[test]
fn the_open_target_is_the_selected_files_absolute_path() {
    let t = TempRepo::new("open-target");
    t.write("dir with spaces/f.txt", BASE);
    t.commit_all("base");
    t.write("dir with spaces/f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    assert!(app.open_target().is_none(), "nothing selected yet");

    app.select_file(Pane::Unstaged, "dir with spaces/f.txt");
    let target = app.open_target().expect("the file is on disk");
    assert_eq!(target, t.dir.join("dir with spaces").join("f.txt"));
    assert!(target.is_absolute(), "the opener needs a full path");
}

#[test]
fn an_untracked_file_can_be_opened_too() {
    let t = TempRepo::new("open-untracked");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("brand-new.txt", "never committed\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "brand-new.txt");
    assert_eq!(app.open_target(), Some(t.dir.join("brand-new.txt")));
}

#[test]
fn a_staged_file_opens_the_working_tree_copy() {
    let t = TempRepo::new("open-staged");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");
    t.repo.run(&["add", "f.txt"]).unwrap();

    let mut app = app_on(&t);
    app.select_file(Pane::Staged, "f.txt");
    assert_eq!(app.open_target(), Some(t.dir.join("f.txt")));
}

#[test]
fn opening_a_deleted_file_is_refused_with_an_explanation() {
    let t = TempRepo::new("open-deleted");
    t.write("gone.txt", "bye\n");
    t.commit_all("base");
    std::fs::remove_file(t.dir.join("gone.txt")).unwrap();

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "gone.txt");
    assert!(app.open_target().is_none());

    app.open_selected_file();
    let err = app.error.clone().expect("a refusal should be reported");
    assert!(
        err.contains("gone.txt") && err.contains("not in the working tree"),
        "unhelpful message: {err:?}"
    );
    assert!(app.info.is_none(), "it must not claim to have opened anything");
}

#[test]
fn a_historical_file_that_no_longer_exists_cannot_be_opened() {
    let t = TempRepo::new("open-history");
    t.write("temp.txt", "here for one commit\n");
    t.commit_all("add temp");
    std::fs::remove_file(t.dir.join("temp.txt")).unwrap();
    t.commit_all("remove temp");

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    let sha = app.commits.last().expect("commits loaded").sha.clone();
    app.select_commit(&sha);
    app.select_file(Pane::Commit, "temp.txt");

    assert!(
        app.open_target().is_none(),
        "it exists only inside the commit, not on disk"
    );
}

#[test]
fn the_path_handed_to_the_system_opener_has_native_separators() {
    let t = TempRepo::new("open-separators");
    t.write("src/deep/nested.txt", BASE);
    t.commit_all("base");
    t.write("src/deep/nested.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "src/deep/nested.txt");
    let target = app.open_target().expect("the file is on disk");

    let handed = gitgui::desktop::native(&target);
    let text = handed.to_string_lossy().into_owned();
    if cfg!(windows) {
        assert!(
            !text.contains('/'),
            "Explorer opens Documents instead of the file when given a mixed path: {text}"
        );
    }
    assert!(
        handed.exists(),
        "normalising must not break the path: {text}"
    );
    assert!(
        text.ends_with("nested.txt"),
        "wrong file entirely: {text}"
    );
}

#[test]
fn an_explicit_path_is_tried_before_the_working_directory() {
    let arg = std::path::PathBuf::from("/explicit/repo");
    let cwd = std::path::PathBuf::from("/some/where");
    let got = gitgui::app::startup_candidates(
        Some(arg.clone()),
        Some(cwd.clone()),
        Some(std::path::PathBuf::from("/opt/app")),
    );
    assert_eq!(got, vec![arg, cwd], "the command line argument comes first");
}

#[test]
fn the_working_directory_counts_when_it_is_not_the_program_folder() {
    let cwd = std::path::PathBuf::from("/home/me/project");
    let got = gitgui::app::startup_candidates(
        None,
        Some(cwd.clone()),
        Some(std::path::PathBuf::from("/opt/app")),
    );
    assert_eq!(got, vec![cwd], "running inside a checkout should open it");
}

#[test]
fn launching_from_the_program_folder_offers_no_repository() {
    let t = TempRepo::new("startup-pin");
    let exe_dir = t.dir.join("target").join("release");
    std::fs::create_dir_all(&exe_dir).unwrap();

    let got = gitgui::app::startup_candidates(None, Some(exe_dir.clone()), Some(exe_dir));
    assert!(
        got.is_empty(),
        "the binary's own folder is not a repository choice, got {got:?}"
    );
}

#[test]
fn the_program_folder_check_survives_separator_differences() {
    let t = TempRepo::new("startup-sep");
    let exe_dir = t.dir.join("target").join("release");
    std::fs::create_dir_all(&exe_dir).unwrap();

    let scruffy = std::path::PathBuf::from(exe_dir.to_string_lossy().replace('\\', "/"));
    let got = gitgui::app::startup_candidates(None, Some(scruffy), Some(exe_dir));
    assert!(
        got.is_empty(),
        "the same folder written differently is still the same folder: {got:?}"
    );
}

#[test]
fn an_explicit_path_still_wins_from_the_program_folder() {
    let t = TempRepo::new("startup-pin-arg");
    let exe_dir = t.dir.join("target").join("release");
    std::fs::create_dir_all(&exe_dir).unwrap();
    let arg = std::path::PathBuf::from("/chosen/repo");

    let got = gitgui::app::startup_candidates(Some(arg.clone()), Some(exe_dir.clone()), Some(exe_dir));
    assert_eq!(
        got,
        vec![arg],
        "a path on the command line must always be honoured"
    );
}

#[test]
fn dragging_over_the_text_selects_characters_not_lines() {
    let t = TempRepo::new("textsel-basic");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nHELLO WORLD\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "HELLO WORLD");
    let d = app.diff.as_mut().unwrap();

    d.begin_text_sel(TextPos { row, col: 6 });
    d.set_text_head(TextPos { row, col: 11 });

    assert_eq!(d.clipboard_text().as_deref(), Some("WORLD"));
    assert!(
        d.sel.is_empty(),
        "a text selection must not stage anything by itself"
    );
}

#[test]
fn a_text_selection_spanning_rows_yields_clean_code() {
    let t = TempRepo::new("textsel-rows");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let first = row_of(&app, "AAA");
    let last = row_of(&app, "BBB");
    let d = app.diff.as_mut().unwrap();

    d.begin_text_sel(TextPos { row: first, col: 0 });
    d.set_text_head(TextPos { row: last, col: 3 });

    let got = d.clipboard_text().expect("something is selected");
    assert_eq!(
        got, "AAA\nBBB",
        "no line numbers and no + markers, got {got:?}"
    );
}

#[test]
fn a_backwards_drag_selects_the_same_text() {
    let t = TempRepo::new("textsel-backwards");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let first = row_of(&app, "AAA");
    let last = row_of(&app, "BBB");
    let d = app.diff.as_mut().unwrap();

    d.begin_text_sel(TextPos { row: last, col: 3 });
    d.set_text_head(TextPos { row: first, col: 0 });
    assert_eq!(d.clipboard_text().as_deref(), Some("AAA\nBBB"));
}

#[test]
fn an_empty_text_selection_copies_nothing() {
    let t = TempRepo::new("textsel-empty");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    let d = app.diff.as_mut().unwrap();

    d.begin_text_sel(TextPos { row, col: 2 });
    assert!(
        d.clipboard_text().is_none(),
        "a click without a drag has nothing highlighted"
    );
}

#[test]
fn with_no_text_selection_copy_falls_back_to_the_selected_lines() {
    let t = TempRepo::new("textsel-fallback");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nBBB\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let a = row_of(&app, "AAA");
    let b = row_of(&app, "BBB");
    let d = app.diff.as_mut().unwrap();
    d.select_only(a);
    d.toggle(b);

    let got = d.clipboard_text().expect("two lines are selected");
    assert_eq!(
        got, "AAA\nBBB\n",
        "selected lines copy as bare code, got {got:?}"
    );
}

#[test]
fn a_text_selection_wins_over_the_line_selection() {
    let t = TempRepo::new("textsel-priority");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    let d = app.diff.as_mut().unwrap();
    d.select_only(row);
    d.begin_text_sel(TextPos { row, col: 0 });
    d.set_text_head(TextPos { row, col: 1 });

    assert_eq!(
        d.clipboard_text().as_deref(),
        Some("A"),
        "the highlight is what the user can see, so it wins"
    );
}

#[test]
fn deleted_lines_copy_without_their_marker() {
    let t = TempRepo::new("textsel-deletion");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "two");
    let d = app.diff.as_mut().unwrap();
    d.select_only(row);

    assert_eq!(
        d.clipboard_text().as_deref(),
        Some("two\n"),
        "a removed line copies as the code that was there"
    );
}

#[test]
fn the_highlight_span_is_clamped_to_each_row() {
    let t = TempRepo::new("textsel-span");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\nBBB\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let first = row_of(&app, "AAA");
    let last = row_of(&app, "BBB");
    let d = app.diff.as_mut().unwrap();

    d.begin_text_sel(TextPos { row: first, col: 1 });
    d.set_text_head(TextPos { row: last, col: 999 });

    assert_eq!(d.text_sel_span(first), Some((1, 3)), "runs to end of row");
    assert_eq!(d.text_sel_span(last), Some((0, 3)), "clamped to row length");
    assert_eq!(d.text_sel_span(first - 1), None, "rows outside are untouched");
}

#[test]
fn clearing_the_selection_clears_the_text_highlight_too() {
    let t = TempRepo::new("textsel-clear");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one\nAAA\ntwo\nthree\nfour\nfive\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "AAA");
    let d = app.diff.as_mut().unwrap();
    d.begin_text_sel(TextPos { row, col: 0 });
    d.set_text_head(TextPos { row, col: 3 });
    assert!(d.clipboard_text().is_some());

    d.clear_text_sel();
    assert!(d.clipboard_text().is_none());
    assert!(!d.text_dragging);
}

#[test]
fn the_cursor_line_is_the_working_tree_line_number() {
    let t = TempRepo::new("edit-line");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "one
two
three
INSERTED
four
five
");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "INSERTED");
    app.diff.as_mut().unwrap().cursor = row;

    assert_eq!(
        app.cursor_line(),
        4,
        "INSERTED is the fourth line of the new file"
    );
}

#[test]
fn a_deleted_line_falls_back_to_the_nearest_new_side_line() {
    let t = TempRepo::new("edit-line-del");
    t.write("f.txt", BASE);
    t.commit_all("base");
    // "three" is removed, so its row has no new-side number at all.
    t.write("f.txt", "one
two
four
five
");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    let row = row_of(&app, "three");
    app.diff.as_mut().unwrap().cursor = row;

    let line = app.cursor_line();
    assert!(
        (1..=4).contains(&line),
        "should land near the deletion, got {line}"
    );
}

#[test]
fn editing_a_file_that_is_gone_is_refused() {
    let t = TempRepo::new("edit-missing");
    t.write("gone.txt", "bye
");
    t.commit_all("base");
    std::fs::remove_file(t.dir.join("gone.txt")).unwrap();

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "gone.txt");
    app.edit_at_cursor();

    let err = app.error.clone().expect("a refusal should be reported");
    assert!(
        err.contains("gone.txt") && err.contains("nothing to edit"),
        "unhelpful message: {err:?}"
    );
    assert!(app.info.is_none(), "it must not claim to have opened anything");
}

#[test]
fn the_cursor_line_is_sane_with_no_diff_open() {
    let t = TempRepo::new("edit-noline");
    t.write("f.txt", BASE);
    t.commit_all("base");

    let app = app_on(&t);
    assert_eq!(app.cursor_line(), 1, "no diff means line one, never zero");
}

#[test]
fn a_conflicted_merge_is_reported_as_in_progress() {
    let t = TempRepo::new("merge-state");
    t.conflicted_merge();

    let app = app_on(&t);
    assert!(
        app.merging.is_some(),
        "MERGE_HEAD exists, so the app should say a merge is underway"
    );
    let name = app.merging.clone().unwrap();
    assert!(
        name.contains("side"),
        "the incoming branch should be named, got {name:?}"
    );
    assert_eq!(app.unresolved(), vec!["shared.txt"]);
}

#[test]
fn the_merge_message_is_prefilled() {
    let t = TempRepo::new("merge-msg");
    t.conflicted_merge();

    let app = app_on(&t);
    assert!(
        app.commit_msg.contains("side"),
        "git prepared a message; it should be offered, got {:?}",
        app.commit_msg
    );
    assert!(
        !app.commit_msg.contains("Conflicts:")
            || !app.commit_msg.lines().any(|l| l.trim_start().starts_with('#')),
        "comment lines must be stripped: {:?}",
        app.commit_msg
    );
}

#[test]
fn taking_ours_resolves_the_file_with_this_branchs_version() {
    let t = TempRepo::new("merge-ours");
    t.conflicted_merge();

    let mut app = app_on(&t);
    app.take_side("shared.txt", true);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(t.worktree("shared.txt"), "one\nOURS\nthree\n");
    assert_eq!(t.indexed("shared.txt"), "one\nOURS\nthree\n");
    assert!(
        app.unresolved().is_empty(),
        "the file should no longer be conflicted"
    );
    assert!(
        !app.staged().iter().any(|e| e.path == "shared.txt"),
        "ours matches HEAD, so git reports no staged change for this file"
    );
    assert!(
        app.staged().iter().any(|e| e.path == "only-theirs.txt"),
        "the side branch's non-conflicting addition is staged by the merge"
    );
    assert!(app.can_commit(), "the merge is ready to finish");
}

#[test]
fn taking_theirs_resolves_the_file_with_the_incoming_version() {
    let t = TempRepo::new("merge-theirs");
    t.conflicted_merge();

    let mut app = app_on(&t);
    app.take_side("shared.txt", false);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    assert_eq!(t.worktree("shared.txt"), "one\nTHEIRS\nthree\n");
    assert!(app.unresolved().is_empty());
}

#[test]
fn committing_is_blocked_until_every_conflict_is_resolved() {
    let t = TempRepo::new("merge-block");
    t.conflicted_merge();

    let mut app = app_on(&t);
    assert!(
        !app.can_commit(),
        "a conflicted merge must not look committable"
    );

    app.commit();
    let err = app.error.clone().expect("it should say why");
    assert!(
        err.contains("shared.txt") && err.contains("unresolved"),
        "unhelpful message: {err:?}"
    );
}

#[test]
fn resolving_then_committing_finishes_the_merge() {
    let t = TempRepo::new("merge-finish");
    t.conflicted_merge();

    let mut app = app_on(&t);
    app.take_side("shared.txt", true);
    assert!(app.can_commit(), "message prefilled and conflicts resolved");

    app.commit();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    let parents = t
        .repo
        .run(&["rev-list", "--parents", "-n1", "HEAD"])
        .unwrap()
        .split_whitespace()
        .count();
    assert_eq!(parents, 3, "a merge commit has two parents plus its own sha");
    assert!(app.merging.is_none(), "the merge is over");
    assert!(t.repo.merge_head().is_none(), "MERGE_HEAD should be gone");
    // The side branch's other file came along with the merge.
    assert_eq!(t.head("only-theirs.txt"), "added on side\n");
}

#[test]
fn aborting_a_merge_asks_first_and_then_restores_the_branch() {
    let t = TempRepo::new("merge-abort");
    t.conflicted_merge();
    let before = t.sha_of("HEAD");

    let mut app = app_on(&t);
    app.ask_abort_merge();
    assert!(
        matches!(app.confirm, Some(Confirm::AbortMerge)),
        "aborting throws work away, so it must ask"
    );

    app.confirm_yes();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert!(app.merging.is_none(), "no longer merging");
    assert_eq!(t.sha_of("HEAD"), before, "HEAD must not have moved");
    assert_eq!(t.worktree("shared.txt"), "one\nOURS\nthree\n");
    assert!(app.unresolved().is_empty());
    assert!(app.commit_msg.is_empty(), "the prepared message is dropped");
}

#[test]
fn cancelling_an_abort_leaves_the_merge_alone() {
    let t = TempRepo::new("merge-abort-cancel");
    t.conflicted_merge();

    let mut app = app_on(&t);
    app.ask_abort_merge();
    app.confirm = None;
    app.rescan();

    assert!(app.merging.is_some(), "still merging");
    assert_eq!(app.unresolved(), vec!["shared.txt"]);
}

#[test]
fn a_repository_with_no_merge_underway_says_so() {
    let t = TempRepo::new("merge-none");
    t.write("f.txt", BASE);
    t.commit_all("base");

    let mut app = app_on(&t);
    assert!(app.merging.is_none());
    assert!(app.unresolved().is_empty());
    app.ask_abort_merge();
    assert!(app.confirm.is_none(), "nothing to abort, nothing to ask");
}

#[test]
fn a_merge_resolved_entirely_to_ours_can_still_be_committed() {
    // Only the conflicted file differs, so resolving it to ours leaves the
    // index identical to HEAD and nothing at all shows as staged.
    let t = TempRepo::new("merge-ours-only");
    t.write("shared.txt", "one\ntwo\nthree\n");
    t.commit_all("base");
    t.git(&["checkout", "-q", "-b", "side"]);
    t.write("shared.txt", "one\nTHEIRS\nthree\n");
    t.commit_all("side edit");
    t.git(&["checkout", "-q", "main"]);
    t.write("shared.txt", "one\nOURS\nthree\n");
    t.commit_all("main edit");
    let out = std::process::Command::new("git")
        .current_dir(&t.dir)
        .args(["merge", "--no-edit", "side"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "expected a conflict");

    let mut app = app_on(&t);
    app.take_side("shared.txt", true);
    assert!(
        app.staged().is_empty(),
        "nothing differs from HEAD, so the staged pane is empty"
    );
    assert!(
        app.can_commit(),
        "a merge still has to be committable with an empty index delta"
    );

    app.commit();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    let parents = t
        .repo
        .run(&["rev-list", "--parents", "-n1", "HEAD"])
        .unwrap()
        .split_whitespace()
        .count();
    assert_eq!(parents, 3, "the second parent has to be recorded");
    assert!(app.merging.is_none());
}
