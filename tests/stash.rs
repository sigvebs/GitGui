//! Stash browsing and operations, against real repositories.

mod common;

use common::TempRepo;

use gitgui::app::{App, Confirm, Mode, Op, Pane};
use gitgui::git::status::Change;

fn app_on(t: &TempRepo) -> App {
    let mut app = App::default();
    app.open(t.dir.clone());
    assert!(app.repo.is_some(), "repo should open: {:?}", app.error);
    app
}

/// Builds three stashes covering the shapes that matter: tracked-only, one with
/// untracked files (which get a third parent), and one with a staged change.
fn seed_stashes(t: &TempRepo) {
    t.write("a.txt", "a1\na2\na3\n");
    t.write("b.txt", "keep\n");
    t.commit_all("base");

    t.write("a.txt", "a1\nCHANGED\na3\n");
    t.git(&["stash", "push", "-q", "-m", "tracked only"]);

    t.write("a.txt", "a1\na2\na3\nAPPENDED\n");
    t.write("fresh.txt", "brand new\n");
    t.git(&["stash", "push", "-q", "-u", "-m", "with untracked"]);

    t.write("a.txt", "a1\nSTAGED\na3\n");
    t.git(&["add", "a.txt"]);
    t.git(&["stash", "push", "-q", "-m", "index and worktree"]);
}

#[test]
fn lists_stashes_newest_first_with_usable_refs() {
    let t = TempRepo::new("stash-list");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);

    assert_eq!(app.stashes.len(), 3);
    // Refs must be positional, not date-derived: %gd with --date=short would
    // have produced "stash@{2026-08-07}".
    assert_eq!(app.stashes[0].refname(), "stash@{0}");
    assert_eq!(app.stashes[1].refname(), "stash@{1}");
    assert_eq!(app.stashes[2].refname(), "stash@{2}");

    assert_eq!(app.stashes[0].message(), "index and worktree");
    assert_eq!(app.stashes[1].message(), "with untracked");
    assert_eq!(app.stashes[2].message(), "tracked only");
    assert_eq!(app.stashes[0].branch(), Some("main"));

    // Only the -u stash carries a third parent.
    assert!(!app.stashes[0].has_untracked());
    assert!(app.stashes[1].has_untracked());
    assert!(!app.stashes[2].has_untracked());
}

#[test]
fn selecting_a_stash_lists_its_files_and_loads_a_diff() {
    let t = TempRepo::new("stash-select");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.select_stash(2); // "tracked only"

    assert_eq!(app.stash_files.len(), 1);
    assert_eq!(app.stash_files[0].path, "a.txt");
    assert_eq!(app.stash_files[0].status, Change::Modified);
    assert!(!app.stash_files[0].untracked);

    app.select_file(Pane::Stash, "a.txt");
    let d = app.diff.as_ref().expect("diff loaded");
    assert_eq!(d.fd.total_changes(), 2, "one line replaced");
    assert!(
        d.fd.hunks[0]
            .lines
            .iter()
            .any(|l| l.text.contains("CHANGED")),
        "should show the stashed content"
    );
}

#[test]
fn a_stash_with_untracked_files_shows_them_too() {
    let t = TempRepo::new("stash-untracked");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.select_stash(1); // "with untracked"

    let paths: Vec<&str> = app.stash_files.iter().map(|f| f.path.as_str()).collect();
    assert!(
        paths.contains(&"fresh.txt"),
        "untracked file must be listed, got {paths:?}"
    );
    assert!(paths.contains(&"a.txt"), "tracked change too, got {paths:?}");

    let fresh = app
        .stash_files
        .iter()
        .find(|f| f.path == "fresh.txt")
        .unwrap();
    assert!(fresh.untracked, "should be flagged as from the third parent");

    // Its content lives in the third parent, so a plain diff cannot reach it.
    app.select_file(Pane::Stash, "fresh.txt");
    let d = app.diff.as_ref().expect("untracked diff loaded");
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(d.fd.total_changes(), 1);
    assert!(d.fd.is_new);
    assert_eq!(d.fd.hunks[0].lines[0].text, "brand new");

    // And the tracked file still works in the same stash.
    app.select_file(Pane::Stash, "a.txt");
    assert!(app.diff.as_ref().unwrap().fd.total_changes() > 0);
}

#[test]
fn stash_contents_are_read_only() {
    let t = TempRepo::new("stash-readonly");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.select_stash(2);
    app.select_file(Pane::Stash, "a.txt");
    assert!(app.read_only());
    assert_eq!(app.primary_op(), None);

    let before = t.worktree("a.txt");
    app.diff.as_mut().unwrap().select_all();
    app.apply_lines(Op::Stage);
    app.apply_hunk(Op::Stage, 0);
    app.apply_lines(Op::Discard);

    assert!(app.error.is_none(), "should be inert: {:?}", app.error);
    assert_eq!(app.status.staged().len(), 0, "index untouched");
    assert_eq!(t.worktree("a.txt"), before, "working tree untouched");
    assert_eq!(app.stashes.len(), 3, "stashes untouched");
}

#[test]
fn creating_a_stash_clears_the_working_tree() {
    let t = TempRepo::new("stash-create");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "one\ntwo\n");
    t.write("new.txt", "untracked\n");

    let mut app = app_on(&t);
    assert!(!app.status.unstaged().is_empty());

    app.open_stash_dialog();
    assert!(app.show_stash_dialog);
    assert!(
        app.stash_untracked,
        "should default to including untracked when some exist"
    );
    app.stash_message = "wip on the thing".into();
    app.create_stash();

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert!(!app.show_stash_dialog);
    assert_eq!(t.worktree("a.txt"), "one\n", "reverted to HEAD");
    assert!(
        !t.dir.join("new.txt").exists(),
        "untracked file went into the stash"
    );
    assert_eq!(app.stashes.len(), 1);
    assert_eq!(app.stashes[0].message(), "wip on the thing");
    assert!(app.stashes[0].has_untracked());
}

#[test]
fn stashing_is_refused_when_there_is_nothing_to_stash() {
    let t = TempRepo::new("stash-clean");
    t.write("a.txt", "one\n");
    t.commit_all("base");

    let mut app = app_on(&t);
    app.open_stash_dialog();

    assert!(!app.show_stash_dialog);
    let e = app.error.clone().unwrap_or_default();
    assert!(e.to_lowercase().contains("clean"), "got: {e}");
}

#[test]
fn applying_a_stash_restores_the_changes_and_keeps_the_entry() {
    let t = TempRepo::new("stash-apply");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "one\ntwo\n");
    t.git(&["stash", "push", "-q", "-m", "later"]);
    assert_eq!(t.worktree("a.txt"), "one\n");

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    assert_eq!(app.stashes.len(), 1);

    app.apply_stash(0, false);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("a.txt"), "one\ntwo\n", "changes are back");
    assert_eq!(app.stashes.len(), 1, "apply keeps the entry");
}

#[test]
fn popping_a_stash_restores_the_changes_and_removes_the_entry() {
    let t = TempRepo::new("stash-pop");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "one\ntwo\n");
    t.git(&["stash", "push", "-q", "-m", "later"]);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.select_stash(0);
    app.apply_stash(0, true);

    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(t.worktree("a.txt"), "one\ntwo\n");
    assert!(app.stashes.is_empty(), "pop removes the entry");
    assert!(app.sel_stash.is_none(), "selection cleared");
    assert!(app.diff.is_none());
}

#[test]
fn a_conflicting_apply_reports_the_failure_without_losing_the_stash() {
    let t = TempRepo::new("stash-conflict");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "stashed version\n");
    t.git(&["stash", "push", "-q", "-m", "conflicting"]);

    // Change the same file so the stash cannot apply cleanly.
    t.write("a.txt", "local version\n");

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.apply_stash(0, true);

    let o = app.outcome.as_ref().expect("an outcome is recorded");
    assert!(!o.ok, "should report failure");
    assert!(app.error.is_some());
    assert_eq!(
        app.stashes.len(),
        1,
        "a failed pop must not drop the stash"
    );
}

#[test]
fn dropping_a_stash_asks_first_and_can_be_undone() {
    let t = TempRepo::new("stash-drop");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    assert_eq!(app.stashes.len(), 3);

    app.ask_drop_stash(1);
    assert!(matches!(app.confirm, Some(Confirm::DropStash { .. })));
    assert_eq!(app.stashes.len(), 3, "nothing happens before confirming");

    app.confirm_yes();
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
    assert_eq!(app.stashes.len(), 2, "entry removed");
    let left: Vec<&str> = app.stashes.iter().map(|s| s.message()).collect();
    assert_eq!(left, vec!["index and worktree", "tracked only"]);

    // The commit outlives the entry, so the drop is recoverable.
    assert!(app.undo_label().is_some(), "undo should be offered");
    app.undo_last_discard();
    assert!(app.error.is_none(), "undo failed: {:?}", app.error);
    app.load_stashes();
    assert_eq!(app.stashes.len(), 3, "stash is back");
    assert!(
        app.stashes.iter().any(|s| s.message() == "with untracked"),
        "the restored stash should be the one dropped, got {:?}",
        app.stashes.iter().map(|s| s.message()).collect::<Vec<_>>()
    );
}

#[test]
fn cancelling_a_drop_changes_nothing() {
    let t = TempRepo::new("stash-nodrop");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.ask_drop_stash(0);
    app.confirm = None;

    app.load_stashes();
    assert_eq!(app.stashes.len(), 3);
}

#[test]
fn stash_filter_narrows_the_list() {
    let t = TempRepo::new("stash-filter");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    assert_eq!(app.filtered_stashes().len(), 3);

    app.stash_filter = "untracked".into();
    assert_eq!(app.filtered_stashes().len(), 1);

    app.stash_filter = "nothing here".into();
    assert!(app.filtered_stashes().is_empty());
}

#[test]
fn searching_works_inside_a_stash_diff() {
    let t = TempRepo::new("stash-find");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.select_stash(2);
    app.select_file(Pane::Stash, "a.txt");

    app.find.query = "changed".into();
    app.find_recompute();
    assert_eq!(app.find.matches.len(), 1);
}

#[test]
fn no_stashes_is_an_empty_list_not_an_error() {
    let t = TempRepo::new("stash-none");
    t.write("a.txt", "one\n");
    t.commit_all("base");

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    assert!(app.stashes.is_empty());
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
}

#[test]
fn stashes_on_an_unborn_branch_are_empty_not_an_error() {
    let t = TempRepo::new("stash-unborn");
    t.write("a.txt", "one\n");

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    assert!(app.stashes.is_empty());
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
}

#[test]
fn switching_away_from_stashes_clears_the_diff() {
    let t = TempRepo::new("stash-switch");
    seed_stashes(&t);

    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    app.select_stash(2);
    app.select_file(Pane::Stash, "a.txt");
    assert!(app.diff.is_some());

    app.set_mode(Mode::WorkingTree);
    assert!(app.diff.is_none());
    assert!(app.sel_file.is_none());
}

// ---- headless render --------------------------------------------------------

fn run_frames(app: &mut App, ctx: &egui::Context, n: usize) {
    for _ in 0..n {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 840.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| gitgui::ui::draw(ctx, app));
    }
}

#[test]
fn stash_mode_renders_every_state_without_panicking() {
    let t = TempRepo::new("stash-render");
    seed_stashes(&t);

    let ctx = egui::Context::default();
    let mut app = app_on(&t);

    // Empty selection, then every stash and every file in each.
    app.set_mode(Mode::Stashes);
    run_frames(&mut app, &ctx, 2);

    for i in 0..app.stashes.len() {
        app.select_stash(i);
        run_frames(&mut app, &ctx, 2);
        let paths: Vec<String> = app.stash_files.iter().map(|f| f.path.clone()).collect();
        for p in paths {
            app.select_file(Pane::Stash, &p);
            run_frames(&mut app, &ctx, 2);
            if let Some(d) = app.diff.as_mut() {
                d.select_all();
            }
            run_frames(&mut app, &ctx, 1);
        }
    }

    // Filter, including no results.
    app.stash_filter = "untracked".into();
    run_frames(&mut app, &ctx, 2);
    app.stash_filter = "zzz".into();
    run_frames(&mut app, &ctx, 2);
    app.stash_filter.clear();

    // Drop confirmation.
    app.ask_drop_stash(0);
    run_frames(&mut app, &ctx, 2);
    app.confirm = None;

    // Stash dialog, with and without untracked files present.
    app.set_mode(Mode::WorkingTree);
    t.write("a.txt", "dirty\n");
    t.write("brand-new.txt", "x\n");
    app.rescan();
    app.open_stash_dialog();
    assert!(app.show_stash_dialog);
    run_frames(&mut app, &ctx, 2);
    app.show_stash_dialog = false;

    // Empty stash list.
    let t2 = TempRepo::new("stash-render-empty");
    t2.write("a.txt", "one\n");
    t2.commit_all("base");
    let mut app2 = app_on(&t2);
    app2.set_mode(Mode::Stashes);
    run_frames(&mut app2, &ctx, 2);
}
