//! Drives `App` the way the UI does, plus a headless run of the real widget
//! tree. Together these cover everything a mouse click reaches except the
//! pixel hit-test itself.

mod common;

use common::{TempRepo, BASE};

use gitgui::app::{App, Confirm, Op, Pane, Row};

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
