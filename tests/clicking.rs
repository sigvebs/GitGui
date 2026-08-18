//! Real pointer input against the real widget tree.
//!
//! These exist because of a bug that state-level tests could not see: list rows
//! were laid out with `selectable_label`, so the click target was only as wide
//! as the text. Clicking a row anywhere except directly on its label did
//! nothing. Everything below drives actual clicks.

mod common;

use common::TempRepo;

use gitgui::app::{App, Mode, Pane, Row};

const W: f32 = 1280.0;
const H: f32 = 840.0;

struct Harness {
    ctx: egui::Context,
    time: f64,
}

impl Harness {
    fn new() -> Harness {
        Harness {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, app: &mut App, events: Vec<egui::Event>) {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(W, H),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run(input, |ctx| gitgui::ui::draw(ctx, app));
    }

    /// Runs one frame and reports what egui asked the host to do, which is how
    /// a clipboard write shows up.
    fn frame_output(&mut self, app: &mut App, events: Vec<egui::Event>) -> Vec<String> {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(W, H),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let out = self.ctx.run(input, |ctx| gitgui::ui::draw(ctx, app));
        out.platform_output
            .commands
            .iter()
            .map(|c| format!("{c:?}"))
            .collect()
    }

    fn frames(&mut self, app: &mut App, n: usize) {
        for _ in 0..n {
            self.frame(app, Vec::new());
        }
    }

    /// Two clicks inside egui's double-click window, unlike `click`, which
    /// deliberately steps past it.
    fn double_click(&mut self, app: &mut App, pos: egui::Pos2) {
        self.time += 1.0;
        self.frame(app, vec![egui::Event::PointerMoved(pos)]);
        for _ in 0..2 {
            for pressed in [true, false] {
                self.frame(
                    app,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    }],
                );
            }
        }
        self.frame(app, Vec::new());
    }

    /// Press, move in steps so egui registers a drag, then release.
    fn drag(&mut self, app: &mut App, from: egui::Pos2, to: egui::Pos2) {
        self.time += 1.0;
        self.frame(app, vec![egui::Event::PointerMoved(from)]);
        self.frame(
            app,
            vec![egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        for step in 1..=6 {
            let t = step as f32 / 6.0;
            let at = egui::pos2(
                from.x + (to.x - from.x) * t,
                from.y + (to.y - from.y) * t,
            );
            self.frame(app, vec![egui::Event::PointerMoved(at)]);
        }
        self.frame(
            app,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        self.frame(app, Vec::new());
    }

    fn click(&mut self, app: &mut App, pos: egui::Pos2) {
        // Skip past egui's double-click window, or successive probe clicks fuse
        // into double-clicks — which in the file panes means "stage/unstage".
        self.time += 1.0;
        self.frame(app, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(
                app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }],
            );
        }
        self.frame(app, Vec::new());
    }
}

fn app_on(t: &TempRepo) -> App {
    let mut app = App::default();
    app.open(t.dir.clone());
    assert!(app.repo.is_some(), "repo should open: {:?}", app.error);
    app
}

/// Finds a y within `ys` where clicking at `x` selects a file, resetting between
/// attempts. Returns the y that worked.
fn find_row_y(
    h: &mut Harness,
    app: &mut App,
    x: f32,
    ys: &[f32],
    reset: &mut dyn FnMut(&mut App),
) -> Option<f32> {
    for &y in ys {
        reset(app);
        h.frames(app, 1);
        h.click(app, egui::pos2(x, y));
        if app.sel_file.is_some() {
            return Some(y);
        }
    }
    None
}

/// Row y positions to probe. The panes start just under their headers.
const UNSTAGED_YS: [f32; 6] = [60.0, 66.0, 72.0, 78.0, 84.0, 90.0];
const STAGED_YS: [f32; 8] = [441.0, 447.0, 453.0, 459.0, 465.0, 471.0, 477.0, 483.0];
const COMMIT_FILE_YS: [f32; 8] = [
    520.0, 526.0, 532.0, 538.0, 544.0, 550.0, 556.0, 562.0,
];

#[test]
fn clicking_an_unstaged_file_row_loads_its_diff() {
    let t = TempRepo::new("click-unstaged");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "one\ntwo\n");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    h.frames(&mut app, 2);

    let y = find_row_y(&mut h, &mut app, 200.0, &UNSTAGED_YS, &mut |a| {
        a.sel_file = None;
        a.diff = None;
    })
    .expect("no unstaged row responded to a click");

    assert_eq!(app.sel_file.as_ref().map(|s| s.pane), Some(Pane::Unstaged));
    assert!(app.diff.is_some(), "diff should load, err={:?}", app.error);
    eprintln!("unstaged row at y={y}");
}

#[test]
fn clicking_a_staged_file_row_loads_its_diff() {
    let t = TempRepo::new("click-staged");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "one\ntwo\n");
    t.repo.run(&["add", "a.txt"]).unwrap();

    let mut h = Harness::new();
    let mut app = app_on(&t);
    h.frames(&mut app, 2);
    assert_eq!(app.status.staged().len(), 1);

    find_row_y(&mut h, &mut app, 200.0, &STAGED_YS, &mut |a| {
        a.sel_file = None;
        a.diff = None;
    })
    .expect("no staged row responded to a click");

    assert_eq!(app.sel_file.as_ref().map(|s| s.pane), Some(Pane::Staged));
    assert!(app.diff.is_some(), "diff should load, err={:?}", app.error);
}

#[test]
fn clicking_a_commit_then_a_file_shows_the_diff() {
    let t = TempRepo::new("click-history");
    t.build_history();

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.set_mode(Mode::History);
    h.frames(&mut app, 2);

    // Click a commit row. Rows begin under the header and filter box.
    let mut picked = None;
    for y in [70.0_f32, 91.0, 112.0, 133.0, 154.0] {
        app.sel_commit = None;
        app.commit_files.clear();
        h.frames(&mut app, 1);
        h.click(&mut app, egui::pos2(200.0, y));
        if app.sel_commit.is_some() {
            picked = Some(y);
            break;
        }
    }
    picked.expect("no commit row responded to a click");
    assert!(
        !app.commit_files.is_empty(),
        "clicking a commit should list its files"
    );
    let sha = app.sel_commit.clone().unwrap();
    let want = app.commit_files[0].path.clone();

    // Now click the file row underneath.
    find_row_y(&mut h, &mut app, 200.0, &COMMIT_FILE_YS, &mut |a| {
        a.select_commit(&sha);
        a.sel_file = None;
        a.diff = None;
    })
    .expect("no commit file row responded to a click");

    assert_eq!(app.sel_file.as_ref().map(|s| s.pane), Some(Pane::Commit));
    assert_eq!(app.sel_file.as_ref().unwrap().path, want);
    assert!(
        app.diff.is_some(),
        "file selected but no diff loaded; err={:?}",
        app.error
    );

    // And it survives further drawing.
    h.frames(&mut app, 3);
    assert!(app.diff.is_some(), "diff vanished after drawing");
}

/// The actual regression. A short filename leaves most of the row empty; that
/// empty space must still be clickable.
#[test]
fn clicking_the_empty_space_beside_a_short_filename_still_selects_the_row() {
    let t = TempRepo::new("click-farright");
    t.write("z.md", "one\n"); // deliberately short
    t.commit_all("base");
    t.write("z.md", "one\ntwo\n");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    h.frames(&mut app, 2);

    // Establish which y holds the row using a modest x.
    let y = find_row_y(&mut h, &mut app, 60.0, &UNSTAGED_YS, &mut |a| {
        a.sel_file = None;
        a.diff = None;
    })
    .expect("could not locate the row at all");

    // Now click far to the right of the text, still inside the pane.
    for x in [180.0_f32, 240.0, 280.0] {
        app.sel_file = None;
        app.diff = None;
        h.frames(&mut app, 1);
        h.click(&mut app, egui::pos2(x, y));
        assert_eq!(
            app.sel_file.as_ref().map(|s| s.pane),
            Some(Pane::Unstaged),
            "clicking at x={x} on the row (text ends near x=40) must select it; \
             the whole row is the target, not just the label"
        );
        assert!(app.diff.is_some());
    }
}

#[test]
fn clicking_a_stash_then_a_file_shows_the_diff() {
    let t = TempRepo::new("click-stash");
    t.write("a.txt", "one\n");
    t.commit_all("base");
    t.write("a.txt", "one\nstashed line\n");
    t.git(&["stash", "push", "-q", "-m", "for clicking"]);

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.set_mode(Mode::Stashes);
    h.frames(&mut app, 2);
    assert_eq!(app.stashes.len(), 1);

    // Click the stash row.
    let mut picked = None;
    for y in [70.0_f32, 78.0, 86.0, 94.0, 102.0] {
        app.sel_stash = None;
        app.stash_files.clear();
        h.frames(&mut app, 1);
        h.click(&mut app, egui::pos2(200.0, y));
        if app.sel_stash.is_some() {
            picked = Some(y);
            break;
        }
    }
    picked.expect("no stash row responded to a click");
    assert!(!app.stash_files.is_empty(), "should list the stash's files");

    // Click the file row below.
    let ys: Vec<f32> = (0..14).map(|i| 470.0 + i as f32 * 6.0).collect();
    let mut hit = false;
    for y in ys {
        app.select_stash(0);
        app.sel_file = None;
        app.diff = None;
        h.frames(&mut app, 1);
        h.click(&mut app, egui::pos2(200.0, y));
        if app.sel_file.is_some() {
            hit = true;
            break;
        }
    }
    assert!(hit, "no stash file row responded to a click");
    assert_eq!(app.sel_file.as_ref().map(|s| s.pane), Some(Pane::Stash));
    assert!(
        app.diff.is_some(),
        "file selected but no diff loaded; err={:?}",
        app.error
    );
}

#[test]
fn clicking_a_diff_line_selects_it_for_staging() {
    let t = TempRepo::new("click-diffline");
    t.write("a.txt", "one\ntwo\nthree\n");
    t.commit_all("base");
    t.write("a.txt", "one\nINSERTED\ntwo\nthree\n");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "a.txt");
    h.frames(&mut app, 2);

    // The diff pane is right of the side panel; sweep down its rows.
    let mut hit = None;
    for y in (60..240).step_by(6) {
        if let Some(d) = app.diff.as_mut() {
            d.sel.clear();
        }
        h.frames(&mut app, 1);
        h.click(&mut app, egui::pos2(600.0, y as f32));
        if app
            .diff
            .as_ref()
            .map(|d| d.selected_change_count())
            .unwrap_or(0)
            > 0
        {
            hit = Some(y);
            break;
        }
    }
    let y = hit.expect("clicking a diff line did not select it");
    eprintln!("diff line selected at y={y}");

    // Selecting a changed line and staging it must move exactly that line.
    let n = app.diff.as_ref().unwrap().selected_change_count();
    assert_eq!(n, 1, "a single click selects one line");
    app.apply_lines(gitgui::app::Op::Stage);
    assert!(app.error.is_none(), "staging failed: {:?}", app.error);
    assert_eq!(
        t.indexed("a.txt"),
        "one\nINSERTED\ntwo\nthree\n",
        "the clicked line should be the one staged"
    );
}

#[test]
fn hiding_untracked_takes_those_rows_out_of_the_list() {
    let t = TempRepo::new("click-hide-untracked");
    t.write("zzz-tracked.txt", "one
");
    t.commit_all("base");
    t.write("zzz-tracked.txt", "one
two
");
    // Sorts first, so it owns the top row until it is filtered out.
    t.write("aaa-untracked.txt", "never committed
");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    h.frames(&mut app, 2);

    find_row_y(&mut h, &mut app, 200.0, &UNSTAGED_YS, &mut |a| {
        a.sel_file = None;
        a.diff = None;
    })
    .expect("no unstaged row responded to a click");
    assert_eq!(
        app.sel_file.as_ref().map(|s| s.path.clone()),
        Some("aaa-untracked.txt".to_string()),
        "the untracked file sorts first, so it should be the top row"
    );

    app.hide_untracked = true;
    app.sel_file = None;
    app.diff = None;
    h.frames(&mut app, 2);

    find_row_y(&mut h, &mut app, 200.0, &UNSTAGED_YS, &mut |a| {
        a.sel_file = None;
        a.diff = None;
    })
    .expect("no row responded once untracked files were hidden");
    assert_eq!(
        app.sel_file.as_ref().map(|s| s.path.clone()),
        Some("zzz-tracked.txt".to_string()),
        "with untracked rows filtered out the tracked edit moves to the top"
    );
    assert!(app.diff.is_some(), "diff should load, err={:?}", app.error);
}

#[test]
fn double_clicking_the_diff_header_reaches_the_open_action() {
    let t = TempRepo::new("click-open-header");
    t.write("gone.txt", "bye\n");
    t.commit_all("base");
    // Deleted on purpose: the open action refuses, which proves the double
    // click was wired through without launching a real program in a test run.
    std::fs::remove_file(t.dir.join("gone.txt")).unwrap();

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "gone.txt");
    h.frames(&mut app, 2);
    assert!(app.error.is_none(), "clean to start: {:?}", app.error);
    assert!(app.open_target().is_none(), "the file really is gone");

    let mut hit: Option<f32> = None;
    for y in [30.0, 34.0, 38.0, 42.0, 46.0, 50.0, 54.0, 58.0, 62.0, 66.0] {
        app.error = None;
        h.frames(&mut app, 1);
        h.double_click(&mut app, egui::pos2(350.0, y));
        if app.error.is_some() {
            hit = Some(y);
            break;
        }
    }

    let y = hit.expect("double-clicking the header never reached the open action");
    let err = app.error.clone().unwrap();
    assert!(
        err.contains("not in the working tree"),
        "expected the refusal, got {err:?} (header at y={y})"
    );
}

#[test]
fn a_single_click_on_the_header_does_not_open_anything() {
    let t = TempRepo::new("click-open-header-single");
    t.write("gone.txt", "bye\n");
    t.commit_all("base");
    std::fs::remove_file(t.dir.join("gone.txt")).unwrap();

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "gone.txt");
    h.frames(&mut app, 2);

    for y in [30.0, 34.0, 38.0, 42.0, 46.0, 50.0, 54.0, 58.0, 62.0, 66.0] {
        h.click(&mut app, egui::pos2(350.0, y));
    }
    assert!(
        app.error.is_none(),
        "single clicks must not trigger the opener: {:?}",
        app.error
    );
}

/// Row index whose own text matches, ignoring markers and line numbers.
fn row_with_text(app: &App, needle: &str) -> usize {
    let d = app.diff.as_ref().expect("a diff is loaded");
    for i in 0..d.rows.len() {
        if d.row_text(i).as_deref() == Some(needle) {
            return i;
        }
    }
    panic!("no row whose text is {needle:?}");
}

/// The y at which clicking in the text area lands on exactly `want`. Hunk
/// headers are ruled out by insisting a single change ends up selected, so the
/// caller must aim at a hunk holding more than one change.
fn y_for_row(h: &mut Harness, app: &mut App, x: f32, want: usize) -> f32 {
    for y in (56..520).step_by(2) {
        let y = y as f32;
        if let Some(d) = app.diff.as_mut() {
            d.sel.clear();
            d.clear_text_sel();
        }
        h.frames(app, 1);
        h.click(app, egui::pos2(x, y));
        let d = app.diff.as_ref().unwrap();
        if d.cursor == want
            && d.selected_change_count() == 1
            && matches!(d.rows.get(want), Some(Row::Line { .. }))
        {
            return y;
        }
    }
    panic!("no y maps to row {want}");
}

#[test]
fn dragging_the_text_selects_characters_and_leaves_staging_alone() {
    let t = TempRepo::new("drag-text");
    t.write("f.txt", "x
");
    t.commit_all("base");
    let long: String = std::iter::repeat("abcdefghij").take(12).collect();
    t.write("f.txt", &format!("x
AAA
{long}
BBB
"));

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, &long);
    let y = y_for_row(&mut h, &mut app, 700.0, want);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }
    h.frames(&mut app, 1);

    h.drag(&mut app, egui::pos2(700.0, y), egui::pos2(820.0, y));

    let d = app.diff.as_ref().unwrap();
    let got = d
        .clipboard_text()
        .expect("dragging across the text should select some of it");
    assert!(
        !got.is_empty() && long.contains(&got),
        "expected a slice of the line, got {got:?}"
    );
    assert!(
        !got.contains('\n'),
        "a drag within one row must not span rows: {got:?}"
    );
    assert_eq!(
        d.selected_change_count(),
        0,
        "selecting text must not pick lines for staging"
    );
}

#[test]
fn dragging_the_gutter_still_selects_lines_for_staging() {
    let t = TempRepo::new("drag-gutter");
    t.write("f.txt", "x
");
    t.commit_all("base");
    t.write("f.txt", "x
AAA
BBB
CCC
DDD
");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, "AAA");
    let y = y_for_row(&mut h, &mut app, 700.0, want);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }
    h.frames(&mut app, 1);

    // x=340 is inside the diff pane but over the line numbers.
    h.drag(&mut app, egui::pos2(340.0, y), egui::pos2(340.0, y + 40.0));

    let d = app.diff.as_ref().unwrap();
    assert!(
        d.selected_change_count() >= 2,
        "a gutter drag should sweep several lines, got {}",
        d.selected_change_count()
    );
    assert!(
        d.text_sel.is_none(),
        "a gutter drag must not leave a text highlight"
    );
}

#[test]
fn a_plain_click_on_the_text_still_selects_the_line() {
    let t = TempRepo::new("click-text-line");
    t.write("f.txt", "x
");
    t.commit_all("base");
    t.write("f.txt", "x
AAA
BBB
CCC
");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, "BBB");
    y_for_row(&mut h, &mut app, 700.0, want);

    let d = app.diff.as_ref().unwrap();
    assert_eq!(
        d.selected_change_count(),
        1,
        "clicking the text keeps its old meaning"
    );
    assert!(d.text_sel.is_none(), "a click leaves no text highlight");
}

/// Drags right-to-left from beyond the end of the line back past its start, so
/// the whole row is covered without depending on exact pixel offsets.
fn select_whole_row(h: &mut Harness, app: &mut App, y: f32) {
    h.drag(app, egui::pos2(1270.0, y), egui::pos2(330.0, y));
}

#[test]
fn a_selection_reaches_the_very_end_of_the_line() {
    let t = TempRepo::new("sel-end");
    t.write("f.txt", "x\n");
    t.commit_all("base");
    let line: String = std::iter::repeat("abcdefghij").take(4).collect();
    t.write("f.txt", &format!("x\nAAA\n{line}\nBBB\n"));

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, &line);
    let y = y_for_row(&mut h, &mut app, 700.0, want);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }
    h.frames(&mut app, 1);

    select_whole_row(&mut h, &mut app, y);

    let got = app
        .diff
        .as_ref()
        .unwrap()
        .clipboard_text()
        .expect("the row should be selected");
    assert_eq!(
        got, line,
        "the selection must reach both ends of the line, got {} of {} chars",
        got.chars().count(),
        line.chars().count()
    );
}

#[test]
fn a_tab_indented_line_selects_exactly() {
    let t = TempRepo::new("sel-tabs");
    t.write("f.txt", "x\n");
    t.commit_all("base");
    // Tabs advance four cells, not one, so column maths cannot be a
    // multiplication of one character width.
    let line = "\t\tlet value = compute(a, b);";
    t.write("f.txt", &format!("x\nAAA\n{line}\nBBB\n"));

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, line);
    let y = y_for_row(&mut h, &mut app, 700.0, want);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }
    h.frames(&mut app, 1);

    select_whole_row(&mut h, &mut app, y);

    let got = app
        .diff
        .as_ref()
        .unwrap()
        .clipboard_text()
        .expect("the row should be selected");
    assert_eq!(got, line, "tabs must not throw the columns off");
}

#[test]
fn a_drag_anchors_where_the_press_landed() {
    let t = TempRepo::new("sel-anchor");
    t.write("f.txt", "x\n");
    t.commit_all("base");
    let line: String = std::iter::repeat("abcdefghij").take(4).collect();
    t.write("f.txt", &format!("x\nAAA\n{line}\nBBB\n"));

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, &line);
    let y = y_for_row(&mut h, &mut app, 700.0, want);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }
    h.frames(&mut app, 1);

    // Press past the end of the line, then drag only a little way back. egui
    // reports a drag once the pointer has moved a few pixels; the anchor has to
    // be where the press was, not where the drag was noticed.
    h.drag(&mut app, egui::pos2(700.0, y), egui::pos2(660.0, y));

    let got = app
        .diff
        .as_ref()
        .unwrap()
        .clipboard_text()
        .expect("a short drag still selects something");
    assert!(
        line.contains(&got) && got.chars().count() >= 3,
        "a short drag should pick up a handful of characters, got {got:?}"
    );
    assert!(
        got.chars().count() <= 12,
        "a 40px drag must not run away with the line: {got:?}"
    );
}

#[test]
fn the_copy_shortcut_puts_the_text_selection_on_the_clipboard() {
    let t = TempRepo::new("copy-text");
    t.write("f.txt", "x\n");
    t.commit_all("base");
    let line: String = std::iter::repeat("abcdefghij").take(3).collect();
    t.write("f.txt", &format!("x\nAAA\n{line}\nBBB\n"));

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, &line);
    let y = y_for_row(&mut h, &mut app, 700.0, want);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }
    h.frames(&mut app, 1);
    select_whole_row(&mut h, &mut app, y);

    // What the host actually sends for the copy shortcut; it never delivers the
    // key itself.
    let cmds = h.frame_output(&mut app, vec![egui::Event::Copy]);
    assert!(
        cmds.iter().any(|c| c.contains("CopyText") && c.contains(&line)),
        "expected the line on the clipboard, got {cmds:?}"
    );
}

#[test]
fn the_copy_shortcut_falls_back_to_the_selected_lines() {
    let t = TempRepo::new("copy-lines");
    t.write("f.txt", "x\n");
    t.commit_all("base");
    t.write("f.txt", "x\nAAA\nBBB\n");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);

    let want = row_with_text(&app, "AAA");
    y_for_row(&mut h, &mut app, 700.0, want);
    assert!(app.diff.as_ref().unwrap().text_sel.is_none());

    let cmds = h.frame_output(&mut app, vec![egui::Event::Copy]);
    assert!(
        cmds.iter().any(|c| c.contains("CopyText") && c.contains("AAA")),
        "with no highlight the selected line should be copied, got {cmds:?}"
    );
}

#[test]
fn copying_with_nothing_selected_writes_nothing() {
    let t = TempRepo::new("copy-nothing");
    t.write("f.txt", "x\n");
    t.commit_all("base");
    t.write("f.txt", "x\nAAA\n");

    let mut h = Harness::new();
    let mut app = app_on(&t);
    app.select_file(Pane::Unstaged, "f.txt");
    h.frames(&mut app, 2);
    if let Some(d) = app.diff.as_mut() {
        d.sel.clear();
        d.clear_text_sel();
    }

    let cmds = h.frame_output(&mut app, vec![egui::Event::Copy]);
    assert!(
        !cmds.iter().any(|c| c.contains("CopyText")),
        "nothing selected should mean nothing copied, got {cmds:?}"
    );
}
