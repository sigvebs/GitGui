//! Real pointer input against the real widget tree.
//!
//! These exist because of a bug that state-level tests could not see: list rows
//! were laid out with `selectable_label`, so the click target was only as wide
//! as the text. Clicking a row anywhere except directly on its label did
//! nothing. Everything below drives actual clicks.

mod common;

use common::TempRepo;

use gitgui::app::{App, Mode, Pane};

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

    fn frames(&mut self, app: &mut App, n: usize) {
        for _ in 0..n {
            self.frame(app, Vec::new());
        }
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
