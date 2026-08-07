//! Push, driven against a local bare repository so the real `git push` runs
//! without needing a network or credentials.

mod common;

use common::{TempRepo, BASE};

use gitgui::app::App;

fn app_on(t: &TempRepo) -> App {
    let mut app = App::default();
    app.open(t.dir.clone());
    assert!(app.repo.is_some(), "repo should open: {:?}", app.error);
    app
}

/// Pumps the background task to completion, with a ceiling so a hang fails the
/// test instead of stalling the suite.
fn wait_for_task(app: &mut App) {
    for _ in 0..1000 {
        if !app.poll_task() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("background task did not finish within 10s");
}

#[test]
fn pushes_the_current_branch_and_sets_upstream() {
    let mut t = TempRepo::new("push-ok");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let bare = t.add_bare_remote("origin");
    assert_eq!(TempRepo::remote_sha(&bare, "refs/heads/main"), None);

    let mut app = app_on(&t);
    assert_eq!(app.remotes, vec!["origin"]);

    app.open_push_dialog();
    assert!(app.show_push);
    assert_eq!(app.push_remote, "origin");
    assert!(
        app.push_set_upstream,
        "should offer to set upstream when there is none"
    );
    assert!(!app.push_force_lease, "force must never default on");

    app.start_push();
    assert!(!app.show_push, "dialog closes when the push starts");
    wait_for_task(&mut app);

    let outcome = app.outcome.as_ref().expect("an outcome is recorded");
    assert!(outcome.ok, "push failed: {}", outcome.output);
    assert!(app.error.is_none(), "unexpected error: {:?}", app.error);

    let local = t.sha_of("HEAD");
    assert_eq!(
        TempRepo::remote_sha(&bare, "refs/heads/main").as_deref(),
        Some(local.as_str()),
        "the remote should now hold our commit"
    );

    // -u wired up tracking, and the rescan picked it up.
    assert_eq!(app.status.upstream.as_deref(), Some("origin/main"));
    assert_eq!(app.status.ahead, 0, "nothing left to push");
    assert!(!app.busy());
}

#[test]
fn a_second_push_with_nothing_new_still_succeeds() {
    let mut t = TempRepo::new("push-again");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.add_bare_remote("origin");

    let mut app = app_on(&t);
    app.open_push_dialog();
    app.start_push();
    wait_for_task(&mut app);
    assert!(app.outcome.as_ref().unwrap().ok);

    app.open_push_dialog();
    app.start_push();
    wait_for_task(&mut app);
    let o = app.outcome.as_ref().unwrap();
    assert!(o.ok, "up-to-date push should not be an error: {}", o.output);
}

#[test]
fn push_dialog_defaults_to_the_remote_the_branch_tracks() {
    let mut t = TempRepo::new("push-default");
    t.write("f.txt", BASE);
    t.commit_all("base");
    // "origin" exists but the branch tracks "backup".
    t.add_bare_remote("origin");
    t.add_bare_remote("backup");

    let mut app = app_on(&t);
    app.push_remote = "backup".into();
    app.push_set_upstream = true;
    app.start_push();
    wait_for_task(&mut app);
    assert!(app.outcome.as_ref().unwrap().ok);
    assert_eq!(app.status.upstream.as_deref(), Some("backup/main"));

    // Reopening should now prefer the tracked remote over plain "origin".
    app.open_push_dialog();
    assert_eq!(app.push_remote, "backup");
    assert!(
        !app.push_set_upstream,
        "upstream already set, so do not offer it again"
    );
}

#[test]
fn a_non_fast_forward_push_fails_with_advice() {
    let mut t = TempRepo::new("push-reject");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let bare = t.add_bare_remote("origin");

    let mut app = app_on(&t);
    app.push_remote = "origin".into();
    app.push_set_upstream = true;
    app.start_push();
    wait_for_task(&mut app);
    assert!(app.outcome.as_ref().unwrap().ok);
    let pushed = t.sha_of("HEAD");

    // Rewrite history locally so the remote is no longer an ancestor.
    t.write("f.txt", "totally different\n");
    t.repo.run(&["commit", "-a", "--amend", "-m", "rewritten"]).unwrap();
    app.rescan();

    app.push_remote = "origin".into();
    app.push_set_upstream = false;
    app.push_force_lease = false;
    app.start_push();
    wait_for_task(&mut app);

    let o = app.outcome.as_ref().expect("outcome recorded");
    assert!(!o.ok, "rewritten history must be rejected");
    assert!(app.error.is_some(), "the failure should surface");
    assert!(
        o.output.to_lowercase().contains("fetch")
            || o.output.to_lowercase().contains("non-fast-forward")
            || o.output.to_lowercase().contains("rejected"),
        "expected rejection detail, got: {}",
        o.output
    );
    assert_eq!(
        TempRepo::remote_sha(&bare, "refs/heads/main").as_deref(),
        Some(pushed.as_str()),
        "the remote must be untouched by a rejected push"
    );
}

#[test]
fn force_with_lease_replaces_the_remote_branch() {
    let mut t = TempRepo::new("push-force");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let bare = t.add_bare_remote("origin");

    let mut app = app_on(&t);
    app.push_remote = "origin".into();
    app.push_set_upstream = true;
    app.start_push();
    wait_for_task(&mut app);
    assert!(app.outcome.as_ref().unwrap().ok);

    t.write("f.txt", "rewritten\n");
    t.repo.run(&["commit", "-a", "--amend", "-m", "rewritten"]).unwrap();
    app.rescan();

    app.push_remote = "origin".into();
    app.push_set_upstream = false;
    app.push_force_lease = true;
    app.start_push();
    wait_for_task(&mut app);

    let o = app.outcome.as_ref().unwrap();
    assert!(o.ok, "force-with-lease should succeed here: {}", o.output);
    assert_eq!(
        TempRepo::remote_sha(&bare, "refs/heads/main").as_deref(),
        Some(t.sha_of("HEAD").as_str()),
        "remote should now match the rewritten local branch"
    );
}

#[test]
fn push_is_refused_without_a_remote() {
    let t = TempRepo::new("push-noremote");
    t.write("f.txt", BASE);
    t.commit_all("base");

    let mut app = app_on(&t);
    assert!(app.remotes.is_empty());
    app.open_push_dialog();

    assert!(!app.show_push, "no dialog without somewhere to push");
    let e = app.error.clone().unwrap_or_default();
    assert!(e.contains("remote"), "got: {e}");
}

#[test]
fn push_is_refused_on_a_detached_head() {
    let mut t = TempRepo::new("push-detached");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.write("f.txt", "second\n");
    t.commit_all("second");
    t.add_bare_remote("origin");
    t.git(&["checkout", "-q", "--detach", "HEAD~1"]);

    let mut app = app_on(&t);
    assert!(app.status.detached);
    app.open_push_dialog();

    assert!(!app.show_push);
    let e = app.error.clone().unwrap_or_default();
    assert!(e.to_lowercase().contains("detached"), "got: {e}");
}

#[test]
fn push_is_refused_before_the_first_commit() {
    let mut t = TempRepo::new("push-unborn");
    t.write("f.txt", "not committed\n");
    t.add_bare_remote("origin");

    let mut app = app_on(&t);
    assert!(app.status.unborn || app.status.branch.is_none());
    app.open_push_dialog();

    assert!(!app.show_push);
    assert!(app.error.is_some());
}

#[test]
fn polling_with_nothing_running_is_a_no_op() {
    let t = TempRepo::new("push-idle");
    t.write("f.txt", BASE);
    t.commit_all("base");
    let mut app = app_on(&t);

    assert!(!app.busy());
    assert!(!app.poll_task());
    assert!(app.outcome.is_none());
    assert!(app.error.is_none());
}

#[test]
fn a_push_in_flight_blocks_starting_another() {
    let mut t = TempRepo::new("push-reentrant");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.add_bare_remote("origin");

    let mut app = app_on(&t);
    app.push_remote = "origin".into();
    app.start_push();

    // Whether it is still running is a race, but re-entry must never be allowed
    // while it is.
    if app.busy() {
        app.open_push_dialog();
        assert!(!app.show_push, "must not open a second push dialog");
        let before = app.running.is_some();
        app.start_push();
        assert_eq!(app.running.is_some(), before, "must not stack pushes");
    }
    wait_for_task(&mut app);
    assert!(app.outcome.as_ref().unwrap().ok);
}

// ---- welcome screen with recents -------------------------------------------

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
fn opening_a_repo_records_it_in_the_recent_list() {
    let t = TempRepo::new("recent");
    t.write("f.txt", BASE);
    t.commit_all("base");

    let mut app = App::default();
    assert!(app.repo.is_none());
    app.open(t.dir.clone());

    // Compare against the discovered root, not the path passed in: git resolves
    // symlinks (on macOS /var -> /private/var), and storing the canonical path
    // is what keeps one repo from appearing twice in the list.
    let root = app.repo.as_ref().unwrap().root.clone();
    assert_eq!(
        app.recent.first(),
        Some(&root),
        "the just-opened repo should head the list"
    );

    // Re-opening moves it to the front rather than duplicating it.
    app.open(t.dir.clone());
    assert_eq!(app.recent.iter().filter(|p| **p == root).count(), 1);
}

#[test]
fn welcome_screen_renders_with_and_without_recents() {
    let t = TempRepo::new("recent-render");
    t.write("f.txt", BASE);
    t.commit_all("base");

    let ctx = egui::Context::default();

    // Empty list: should show the explanatory line, not a bare heading.
    let mut app = App::default();
    app.recent.clear();
    run_frames(&mut app, &ctx, 2);
    assert!(app.repo.is_none());

    // Populated, including a stale entry that no longer exists on disk.
    app.recent = vec![
        t.dir.clone(),
        std::path::PathBuf::from("/nonexistent/old/repo"),
    ];
    run_frames(&mut app, &ctx, 2);
    assert!(app.repo.is_none(), "rendering must not open anything");
}

#[test]
fn push_dialog_and_outcome_windows_render() {
    let mut t = TempRepo::new("push-render");
    t.write("f.txt", BASE);
    t.commit_all("base");
    t.add_bare_remote("origin");

    let ctx = egui::Context::default();
    let mut app = app_on(&t);

    app.open_push_dialog();
    assert!(app.show_push);
    run_frames(&mut app, &ctx, 2);

    // The force-with-lease branch draws an extra warning line.
    app.push_force_lease = true;
    run_frames(&mut app, &ctx, 2);
    app.show_push = false;

    // Busy state: spinner in the toolbar and status bar.
    app.push_remote = "origin".into();
    app.start_push();
    run_frames(&mut app, &ctx, 2);
    wait_for_task(&mut app);

    // Success output window.
    run_frames(&mut app, &ctx, 2);
    assert!(app.outcome.is_some());
    app.outcome = None;

    // Failure output window, with a long message.
    app.outcome = Some(gitgui::app::TaskOutcome {
        ok: false,
        title: "Push failed".into(),
        output: "error: failed to push some refs\n".repeat(40),
    });
    run_frames(&mut app, &ctx, 2);
}
