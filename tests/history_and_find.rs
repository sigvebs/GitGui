//! History browsing and diff search, driven through `App` against real
//! repositories.

mod common;

use common::{TempRepo, BASE};

use gitgui::app::{App, Mode, Op, Pane, Row};
use gitgui::git::status::Change;

fn app_on(t: &TempRepo) -> App {
    let mut app = App::default();
    app.open(t.dir.clone());
    assert!(app.repo.is_some(), "repo should open: {:?}", app.error);
    app
}

fn row_of(app: &App, needle: &str) -> usize {
    let d = app.diff.as_ref().expect("a diff is loaded");
    for (i, r) in d.rows.iter().enumerate() {
        if let Row::Line { hunk, line } = r {
            if d.fd.hunks[*hunk].lines[*line].text == needle {
                return i;
            }
        }
    }
    panic!("no row {needle:?}");
}

// ---- history ---------------------------------------------------------------

#[test]
fn history_mode_lists_commits_newest_first() {
    let t = TempRepo::new("hist-list");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);

    assert_eq!(app.mode, Mode::History);
    assert_eq!(app.commits.len(), 5, "got: {:?}", app.commits);
    assert_eq!(app.commits[0].subject, "merge side");
    assert_eq!(
        app.commits.last().unwrap().subject,
        "root commit",
        "oldest last"
    );
    // HEAD decoration should be present on the tip.
    assert!(app.commits[0].refs.contains("main"), "refs: {:?}", app.commits[0].refs);
}

#[test]
fn selecting_a_commit_lists_its_files_and_metadata() {
    let t = TempRepo::new("hist-select");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);

    let modify = app
        .commits
        .iter()
        .find(|c| c.subject == "modify a")
        .unwrap()
        .sha
        .clone();
    app.select_commit(&modify);

    assert_eq!(app.commit_files.len(), 1);
    assert_eq!(app.commit_files[0].path, "a.txt");
    assert_eq!(app.commit_files[0].status, Change::Modified);

    let meta = app.commit_meta.as_ref().expect("metadata loaded");
    assert_eq!(meta.author, "Test");
    assert!(!meta.is_merge());
    assert!(meta.body.starts_with("modify a"));
}

#[test]
fn commit_file_diff_loads_and_is_read_only() {
    let t = TempRepo::new("hist-diff");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    let sha = app
        .commits
        .iter()
        .find(|c| c.subject == "modify a")
        .unwrap()
        .sha
        .clone();
    app.select_commit(&sha);
    app.select_file(Pane::Commit, "a.txt");

    let d = app.diff.as_ref().expect("diff loaded");
    assert_eq!(d.fd.total_changes(), 2, "one line replaced");
    assert!(app.read_only());
    assert_eq!(app.primary_op(), None);

    // Staging out of history must be inert, not an error or a mutation.
    let before = t.head("renamed.txt");
    app.diff.as_mut().unwrap().select_all();
    app.apply_lines(Op::Stage);
    app.apply_hunk(Op::Stage, 0);
    app.apply_lines(Op::Discard);
    assert!(app.error.is_none(), "should be silently inert: {:?}", app.error);
    assert_eq!(app.status.staged().len(), 0, "index untouched");
    assert_eq!(t.head("renamed.txt"), before, "HEAD untouched");
}

#[test]
fn a_renamed_file_is_reported_as_a_rename_not_an_add() {
    let t = TempRepo::new("hist-rename");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    let sha = app
        .commits
        .iter()
        .find(|c| c.subject == "rename and edit")
        .unwrap()
        .sha
        .clone();
    app.select_commit(&sha);

    let f = &app.commit_files[0];
    assert_eq!(f.status, Change::Renamed);
    assert_eq!(f.orig_path.as_deref(), Some("a.txt"));
    assert_eq!(f.path, "renamed.txt");

    // The old path has to be passed through or git reports a fresh add.
    app.select_file(Pane::Commit, "renamed.txt");
    let d = app.diff.as_ref().unwrap();
    assert!(d.fd.rename, "header should carry rename info");
    assert!(!d.fd.is_new, "must not look like a new file");
    assert_eq!(d.fd.total_changes(), 1, "just the appended line");
}

#[test]
fn a_merge_commit_shows_its_first_parent_diff() {
    let t = TempRepo::new("hist-merge");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    let sha = app.commits[0].sha.clone();
    app.select_commit(&sha);

    let meta = app.commit_meta.as_ref().unwrap();
    assert!(meta.is_merge());
    assert_eq!(meta.parents.len(), 2);

    // Plain `git show` prints nothing for a merge; the file list must not be
    // empty here.
    assert_eq!(app.commit_files.len(), 1, "got: {:?}", app.commit_files);
    assert_eq!(app.commit_files[0].path, "s.txt");

    app.select_file(Pane::Commit, "s.txt");
    let d = app.diff.as_ref().expect("merge diff loaded");
    assert!(d.fd.total_changes() > 0);
}

#[test]
fn root_commit_shows_its_files() {
    let t = TempRepo::new("hist-root");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    let sha = app.commits.last().unwrap().sha.clone();
    app.select_commit(&sha);

    assert_eq!(app.commit_files.len(), 1);
    assert_eq!(app.commit_files[0].status, Change::Added);
    app.select_file(Pane::Commit, "a.txt");
    assert_eq!(app.diff.as_ref().unwrap().fd.total_changes(), 3);
}

#[test]
fn commit_filter_narrows_the_list() {
    let t = TempRepo::new("hist-filter");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    assert_eq!(app.filtered_commits().len(), 5);

    app.commit_filter = "rename".into();
    assert_eq!(app.filtered_commits().len(), 1);

    app.commit_filter = "TEST".into(); // author, case-insensitive
    assert_eq!(app.filtered_commits().len(), 5);

    app.commit_filter = "nothing matches this".into();
    assert!(app.filtered_commits().is_empty());
}

#[test]
fn history_on_an_unborn_branch_is_empty_not_an_error() {
    let t = TempRepo::new("hist-unborn");
    t.write("untracked.txt", "x\n");

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    assert!(app.commits.is_empty());
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);
}

#[test]
fn switching_modes_clears_the_open_diff() {
    let t = TempRepo::new("hist-switch");
    t.build_history();
    t.write("renamed.txt", "a1\nCHANGED here\na3\nmore\nlocal edit\n");

    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "renamed.txt");
    assert!(app.diff.is_some());

    app.set_mode(Mode::History);
    assert!(app.diff.is_none(), "stale working-tree diff must go");
    assert!(app.sel_file.is_none());

    app.set_mode(Mode::WorkingTree);
    assert!(app.diff.is_none());
}

// ---- diff search -----------------------------------------------------------

fn search_app(t: &TempRepo) -> App {
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write(
        "f.txt",
        "one\nalpha Beta gamma\ntwo\nbeta again beta\nthree\nfour\nfive\n",
    );
    let mut app = app_on(t);
    app.select_file(Pane::Unstaged, "f.txt");
    app
}

#[test]
fn search_finds_matches_case_insensitively_by_default() {
    let t = TempRepo::new("find-basic");
    let mut app = search_app(&t);

    app.find.query = "beta".into();
    app.find_recompute();

    // "Beta" on one added line, "beta" twice on another.
    assert_eq!(app.find.matches.len(), 3, "got: {:?}", app.find.matches);
    assert_eq!(app.find_status(), "1 of 3");
}

#[test]
fn match_case_toggle_restricts_results() {
    let t = TempRepo::new("find-case");
    let mut app = search_app(&t);

    app.find.query = "Beta".into();
    app.find.case_sensitive = true;
    app.find_recompute();

    assert_eq!(app.find.matches.len(), 1, "only the capitalised one");
}

#[test]
fn match_columns_line_up_with_the_rendered_row() {
    let t = TempRepo::new("find-cols");
    let mut app = search_app(&t);

    app.find.query = "Beta".into();
    app.find.case_sensitive = true;
    app.find_recompute();

    let m = app.find.matches[0];
    let expected_row = row_of(&app, "alpha Beta gamma");
    assert_eq!(m.row, expected_row);
    // "alpha Beta" -> B is at char 6 of the text, +1 for the +/- marker column.
    assert_eq!(m.start, 7);
    assert_eq!(m.end, 11);
}

#[test]
fn repeated_matches_on_one_line_do_not_overlap() {
    let t = TempRepo::new("find-repeat");
    let mut app = search_app(&t);

    app.find.query = "aa".into();
    app.find_recompute();
    let before = app.find.matches.len();

    // "aaaa" holds two non-overlapping "aa".
    t.write("g.txt", "aaaa\n");
    app.rescan();
    app.select_file(Pane::Unstaged, "g.txt");
    app.find.query = "aa".into();
    app.find_recompute();
    assert_eq!(app.find.matches.len(), 2, "non-overlapping, got {before} before");
}

#[test]
fn next_and_prev_wrap_around() {
    let t = TempRepo::new("find-nav");
    let mut app = search_app(&t);
    app.find.query = "beta".into();
    app.find_recompute();
    assert_eq!(app.find.current, 0);

    app.find_next();
    assert_eq!(app.find.current, 1);
    app.find_next();
    assert_eq!(app.find.current, 2);
    app.find_next();
    assert_eq!(app.find.current, 0, "wraps forward");
    app.find_prev();
    assert_eq!(app.find.current, 2, "wraps backward");
}

#[test]
fn search_moves_the_cursor_without_disturbing_the_selection() {
    let t = TempRepo::new("find-sel");
    let mut app = search_app(&t);

    // Stage-relevant selection the user set up before searching.
    let target = row_of(&app, "alpha Beta gamma");
    app.diff.as_mut().unwrap().select_only(target);
    let sel_before = app.diff.as_ref().unwrap().sel.clone();

    app.find.query = "beta again".into();
    app.find_recompute();

    let d = app.diff.as_ref().unwrap();
    assert_eq!(d.sel, sel_before, "searching must not restage anything");
    assert_ne!(d.cursor, target, "but the cursor should have moved");
    assert!(d.scroll_to_cursor, "and the view should follow");
}

#[test]
fn empty_and_missing_queries_report_cleanly() {
    let t = TempRepo::new("find-empty");
    let mut app = search_app(&t);

    app.find.query = String::new();
    app.find_recompute();
    assert!(app.find.matches.is_empty());
    assert_eq!(app.find_status(), "");

    app.find.query = "zzzznotthere".into();
    app.find_recompute();
    assert!(app.find.matches.is_empty());
    assert_eq!(app.find_status(), "no matches");
    // Navigation on an empty set must not panic or wrap into nothing.
    app.find_next();
    app.find_prev();
}

#[test]
fn search_covers_context_lines_too() {
    let t = TempRepo::new("find-ctx");
    let mut app = search_app(&t);

    // "three" is unchanged context in this diff.
    app.find.query = "three".into();
    app.find_recompute();
    assert_eq!(app.find.matches.len(), 1, "context should be searchable");
}

#[test]
fn search_works_inside_a_historical_commit() {
    let t = TempRepo::new("find-hist");
    t.build_history();

    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    let sha = app
        .commits
        .iter()
        .find(|c| c.subject == "modify a")
        .unwrap()
        .sha
        .clone();
    app.select_commit(&sha);
    app.select_file(Pane::Commit, "a.txt");

    app.find.query = "changed".into();
    app.find_recompute();
    assert_eq!(app.find.matches.len(), 1);
}

#[test]
fn switching_files_drops_stale_matches() {
    let t = TempRepo::new("find-stale");
    let mut app = search_app(&t);
    t.write("g.txt", "beta elsewhere\n");
    app.rescan();

    app.find.query = "beta".into();
    app.find_recompute();
    assert_eq!(app.find.matches.len(), 3);

    // select_file must recompute against the new row list.
    app.select_file(Pane::Unstaged, "g.txt");
    assert_eq!(app.find.matches.len(), 1, "recomputed for the new diff");
    for m in &app.find.matches {
        let n = app.diff.as_ref().unwrap().rows.len();
        assert!(m.row < n, "match row {} out of range {n}", m.row);
    }
}

#[test]
fn opening_and_closing_the_find_bar() {
    let t = TempRepo::new("find-open");
    let mut app = search_app(&t);

    app.find.query = "beta".into();
    app.find_open();
    assert!(app.find.open);
    assert!(app.find.request_focus);
    assert_eq!(app.find.matches.len(), 3);

    app.find_close();
    assert!(!app.find.open);
    assert!(app.find.matches.is_empty(), "no stale highlights");
}

// ---- headless render of the new surfaces -----------------------------------

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
fn history_mode_and_find_bar_render_without_panicking() {
    let t = TempRepo::new("ui-hist");
    t.build_history();
    t.write("renamed.txt", "a1\nCHANGED here\na3\nmore\nlocal\n");

    let ctx = egui::Context::default();
    let mut app = app_on(&t);

    app.set_mode(Mode::History);
    run_frames(&mut app, &ctx, 2);

    // Every commit shape: merge, rename, plain edit, root.
    let shas: Vec<String> = app.commits.iter().map(|c| c.sha.clone()).collect();
    for sha in shas {
        app.select_commit(&sha);
        run_frames(&mut app, &ctx, 2);
        let paths: Vec<String> = app.commit_files.iter().map(|f| f.path.clone()).collect();
        for p in paths {
            app.select_file(Pane::Commit, &p);
            run_frames(&mut app, &ctx, 2);
        }
    }

    // Filter box, including a no-results state.
    app.commit_filter = "rename".into();
    run_frames(&mut app, &ctx, 2);
    app.commit_filter = "zzz-no-match".into();
    run_frames(&mut app, &ctx, 2);
    app.commit_filter.clear();

    // Find bar with hits, with no hits, and with match-case on.
    app.find_open();
    app.find.query = "CHANGED".into();
    app.find_recompute();
    run_frames(&mut app, &ctx, 2);
    app.find.case_sensitive = true;
    app.find_recompute();
    run_frames(&mut app, &ctx, 2);
    app.find.query = "zzzz".into();
    app.find_recompute();
    run_frames(&mut app, &ctx, 2);
    app.find_close();

    // Find bar over the working tree as well.
    app.set_mode(Mode::WorkingTree);
    app.select_file(Pane::Unstaged, "renamed.txt");
    app.find_open();
    app.find.query = "local".into();
    app.find_recompute();
    run_frames(&mut app, &ctx, 2);
}
