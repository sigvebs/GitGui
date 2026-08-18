pub mod diff_view;

use egui::{Align2, Color32, Ui};

use crate::app::{App, Confirm, Mode, Pane};
use crate::config;
use crate::git::status::Change;
use crate::theme::Palette;

pub fn draw(ctx: &egui::Context, app: &mut App) {
    top_bar(ctx, app);

    if app.repo.is_none() {
        welcome(ctx, app);
        modals(ctx, app);
        return;
    }

    status_bar(ctx, app);

    match app.mode {
        Mode::WorkingTree => {
            files_panel(ctx, app);
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::TopBottomPanel::bottom("commit")
                    .resizable(true)
                    .default_height(190.0)
                    .min_height(96.0)
                    .show_inside(ui, |ui| commit_box(ui, app));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| diff_view::show(ui, app));
            });
        }
        Mode::History => {
            history_panel(ctx, app);
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::TopBottomPanel::bottom("commit-detail")
                    .resizable(true)
                    .default_height(170.0)
                    .min_height(72.0)
                    .show_inside(ui, |ui| commit_detail(ui, app));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| diff_view::show(ui, app));
            });
        }
        Mode::Stashes => {
            stash_panel(ctx, app);
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::TopBottomPanel::bottom("stash-detail")
                    .resizable(true)
                    .default_height(140.0)
                    .min_height(72.0)
                    .show_inside(ui, |ui| stash_detail(ui, app));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| diff_view::show(ui, app));
            });
        }
    }

    modals(ctx, app);
}

fn top_bar(ctx: &egui::Context, app: &mut App) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("Repository", |ui| {
                if ui.button("Open…").clicked() {
                    app.open_dialog();
                    ui.close();
                }
                if !app.recent.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("Recent").small());
                    let recent = app.recent.clone();
                    for r in recent {
                        let label = r
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| r.display().to_string());
                        if ui.button(label).on_hover_text(r.display().to_string()).clicked() {
                            app.open(r);
                            ui.close();
                        }
                    }
                }
                ui.separator();
                if ui.button("Rescan").clicked() {
                    app.rescan();
                    ui.close();
                }
            });

            if app.repo.is_some() {
                ui.menu_button("Branch", |ui| {
                    if ui.button("New Branch…").clicked() {
                        app.show_new_branch = true;
                        ui.close();
                    }
                    ui.separator();
                    let current = app.status.branch.clone().unwrap_or_default();
                    let branches = app.branches.clone();
                    if branches.is_empty() {
                        ui.label(egui::RichText::new("no branches yet").small());
                    }
                    for b in branches {
                        let mark = if b == current { "● " } else { "   " };
                        if ui.button(format!("{mark}{b}")).clicked() {
                            if b != current {
                                app.checkout_branch(&b);
                            }
                            ui.close();
                        }
                    }
                });

                ui.separator();
                if ui
                    .selectable_label(app.mode == Mode::WorkingTree, "Working Tree")
                    .clicked()
                {
                    app.set_mode(Mode::WorkingTree);
                }
                if ui
                    .selectable_label(app.mode == Mode::History, "History")
                    .on_hover_text("Browse commits")
                    .clicked()
                {
                    app.set_mode(Mode::History);
                }
                let stash_label = if app.stashes.is_empty() {
                    "Stashes".to_string()
                } else {
                    format!("Stashes ({})", app.stashes.len())
                };
                if ui
                    .selectable_label(app.mode == Mode::Stashes, stash_label)
                    .on_hover_text("Browse stashed changes")
                    .clicked()
                {
                    app.set_mode(Mode::Stashes);
                }

                ui.separator();
                if ui.button("Rescan").on_hover_text("Reload from disk  (⌘R)").clicked() {
                    app.rescan();
                }
                if app.mode == Mode::WorkingTree {
                    if ui
                        .button("Stage Changed")
                        .on_hover_text("Stage all tracked modifications (git add -u)")
                        .clicked()
                    {
                        app.stage_changed();
                    }
                    if ui
                        .button("Unstage All")
                        .on_hover_text("Move everything back out of the index")
                        .clicked()
                    {
                        app.unstage_all();
                    }
                    if ui
                        .button("Stash…")
                        .on_hover_text("Put the working tree aside for later")
                        .clicked()
                    {
                        app.open_stash_dialog();
                    }
                    let mut hide = app.hide_untracked;
                    if ui
                        .checkbox(&mut hide, "Hide Untracked")
                        .on_hover_text(
                            "Leave new, never-committed files out of the unstaged \
                             list so only changes to tracked files show",
                        )
                        .changed()
                    {
                        app.set_hide_untracked(hide);
                    }
                }
                if ui
                    .button("Find")
                    .on_hover_text("Search the open diff  (⌘F)")
                    .clicked()
                {
                    app.find_open();
                }

                ui.separator();
                let busy = app.busy();
                ui.add_enabled_ui(!busy, |ui| {
                    let ahead = app.status.ahead;
                    let label = if ahead > 0 {
                        format!("Push ↑{ahead}")
                    } else {
                        "Push…".to_string()
                    };
                    if ui
                        .button(label)
                        .on_hover_text("Push the current branch to a remote")
                        .clicked()
                    {
                        app.open_push_dialog();
                    }
                });
                if busy {
                    ui.spinner();
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("A+").on_hover_text("Larger diff text").clicked() {
                    app.font_size = (app.font_size + 1.0).min(28.0);
                    config::save_font_size(app.font_size);
                }
                if ui.small_button("A−").on_hover_text("Smaller diff text").clicked() {
                    app.font_size = (app.font_size - 1.0).max(8.0);
                    config::save_font_size(app.font_size);
                }
                if let Some(repo) = app.repo.as_ref() {
                    ui.label(
                        egui::RichText::new(repo.name())
                            .strong()
                            .color(ui.visuals().hyperlink_color),
                    )
                    .on_hover_text(repo.root.display().to_string());
                }
            });
        });
    });
}

fn welcome(ctx: &egui::Context, app: &mut App) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let pal = Palette::new(ui.visuals().dark_mode);
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.heading("Git GUI");
            ui.label(
                egui::RichText::new("Stage and unstage individual lines and hunks")
                    .color(pal.note_fg),
            );
            ui.add_space(20.0);
            if ui.button("Open Repository…").clicked() {
                app.open_dialog();
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("or path:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut app.open_input)
                        .desired_width(320.0)
                        .hint_text("/path/to/repo"),
                );
                let go = ui.button("Open").clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go && !app.open_input.trim().is_empty() {
                    let p = std::path::PathBuf::from(app.open_input.trim());
                    app.open(p);
                }
            });
        });

        ui.add_space(28.0);
        let recent = app.recent.clone();
        let mut open: Option<std::path::PathBuf> = None;
        // Keep the list narrow and centred rather than stretched across the window.
        let width = 520.0_f32.min(ui.available_width() - 32.0);
        ui.vertical_centered(|ui| {
            ui.allocate_ui(egui::vec2(width, ui.available_height()), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Recent Repositories").strong());
                    if !recent.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("{}", recent.len()))
                                .color(pal.note_fg)
                                .small(),
                        );
                    }
                });
                ui.separator();

                if recent.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "None yet. Repositories you open will be listed here.",
                        )
                        .color(pal.note_fg),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .id_salt("welcome-recent")
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for r in &recent {
                            let name = r
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| r.display().to_string());
                            let parent = r
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            let resp = list_row(
                                ui,
                                false,
                                &[
                                    Cell::new(&name, ui.visuals().text_color()).strong(),
                                    Cell::new(&parent, pal.note_fg),
                                ],
                                None,
                            );
                            if resp.clicked() {
                                open = Some(r.clone());
                            }
                            resp.on_hover_text(r.display().to_string());
                        }
                    });
            });
        });
        if let Some(p) = open {
            app.open(p);
        }

        if let Some(e) = app.error.clone() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.colored_label(Color32::from_rgb(210, 80, 80), e);
            });
        }
    });
}

fn files_panel(ctx: &egui::Context, app: &mut App) {
    egui::SidePanel::left("files")
        .resizable(true)
        .default_width(320.0)
        .min_width(200.0)
        .show(ctx, |ui| {
            let half = (ui.available_height() * 0.5).max(120.0);
            egui::TopBottomPanel::top("unstaged-pane")
                .resizable(true)
                .default_height(half)
                .min_height(80.0)
                .show_inside(ui, |ui| {
                    file_list(ui, app, Pane::Unstaged);
                });
            // Must be a CentralPanel, not bare `ui`: widgets drawn straight onto
            // the leftover region below a top panel render correctly but never
            // receive pointer input.
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    file_list(ui, app, Pane::Staged);
                });
        });
}

/// Working-tree file pane. `Pane::Commit` has its own list; passing it here
/// would be a programming error, so it is turned away up front and the rest of
/// the function reasons about a single boolean.
fn file_list(ui: &mut Ui, app: &mut App, pane: Pane) {
    if pane.read_only() {
        return;
    }
    let staged = pane == Pane::Staged;
    let pal = Palette::new(ui.visuals().dark_mode);
    let (title, entries) = if staged {
        (
            if app.amend {
                "Staged Changes (Will Amend)"
            } else {
                "Staged Changes (Will Commit)"
            },
            app.staged()
                .iter()
                .map(|e| (e.path.clone(), e.display_path(), e.index, false))
                .collect::<Vec<_>>(),
        )
    } else {
        (
            if app.hide_untracked {
                "Unstaged Changes (Tracked Only)"
            } else {
                "Unstaged Changes"
            },
            app.unstaged()
                .iter()
                .map(|e| {
                    (
                        e.path.clone(),
                        e.display_path(),
                        if e.untracked {
                            Change::Untracked
                        } else {
                            e.worktree
                        },
                        e.unmerged,
                    )
                })
                .collect::<Vec<_>>(),
        )
    };

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong().small());
        ui.label(
            egui::RichText::new(format!("{}", entries.len()))
                .color(pal.note_fg)
                .small(),
        );
    });

    let mut clicked: Option<String> = None;
    let mut toggled: Option<String> = None;
    let mut discard: Option<String> = None;
    let mut take_ours: Option<String> = None;
    let mut take_theirs: Option<String> = None;

    egui::ScrollArea::vertical()
        .id_salt(if staged { "staged-list" } else { "unstaged-list" })
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if entries.is_empty() {
                ui.label(egui::RichText::new("nothing here").color(pal.note_fg).small());
            }
            for (path, display, change, unmerged) in &entries {
                let is_sel = app
                    .sel_file
                    .as_ref()
                    .map(|s| s.pane == pane && &s.path == path)
                    .unwrap_or(false);

                let letter = if *unmerged { "U" } else { change.letter() };
                let color = change_color(*change, *unmerged);

                let resp = list_row(
                    ui,
                    is_sel,
                    &[
                        Cell::new(letter, color).mono().strong(),
                        Cell::new(display, ui.visuals().text_color()),
                    ],
                    None,
                );

                if resp.clicked() {
                    clicked = Some(path.clone());
                }
                if resp.double_clicked() {
                    toggled = Some(path.clone());
                }
                resp.on_hover_text(format!("{} — {}", path, if *unmerged { "conflicted" } else { change.label() }))
                    .context_menu(|ui| {
                        if staged {
                            if ui.button("Unstage File").clicked() {
                                toggled = Some(path.clone());
                                ui.close();
                            }
                        } else {
                            if *unmerged {
                                if ui
                                    .button("Use Ours (this branch)")
                                    .on_hover_text("Keep this branch's version and mark it resolved")
                                    .clicked()
                                {
                                    take_ours = Some(path.clone());
                                    ui.close();
                                }
                                if ui
                                    .button("Use Theirs (incoming)")
                                    .on_hover_text("Take the merged branch's version and mark it resolved")
                                    .clicked()
                                {
                                    take_theirs = Some(path.clone());
                                    ui.close();
                                }
                                ui.separator();
                            }
                            let stage_label = if *unmerged {
                                "Mark Resolved (stage as-is)"
                            } else {
                                "Stage File"
                            };
                            if ui.button(stage_label).clicked() {
                                toggled = Some(path.clone());
                                ui.close();
                            }
                            if ui.button("Discard Changes…").clicked() {
                                discard = Some(path.clone());
                                ui.close();
                            }
                        }
                    });
            }
        });

    if let Some(p) = clicked {
        app.select_file(pane, &p);
    }
    if let Some(p) = toggled {
        if staged {
            app.unstage_file(&p);
        } else {
            app.stage_file(&p);
        }
    }
    if let Some(p) = discard {
        app.ask_discard_file(&p);
    }
    if let Some(p) = take_ours {
        app.take_side(&p, true);
    }
    if let Some(p) = take_theirs {
        app.take_side(&p, false);
    }
}

fn history_panel(ctx: &egui::Context, app: &mut App) {
    egui::SidePanel::left("history")
        .resizable(true)
        .default_width(400.0)
        .min_width(240.0)
        .show(ctx, |ui| {
            let half = (ui.available_height() * 0.6).max(140.0);
            egui::TopBottomPanel::top("commit-list")
                .resizable(true)
                .default_height(half)
                .min_height(100.0)
                .show_inside(ui, |ui| commit_list(ui, app));
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| commit_file_list(ui, app));
        });
}

fn commit_list(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Commits").strong().small());
        let total = app.commits.len();
        let shown = app.filtered_commits().len();
        let count = if shown == total {
            format!("{total}")
        } else {
            format!("{shown}/{total}")
        };
        ui.label(egui::RichText::new(count).color(pal.note_fg).small());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut all = app.log_all_refs;
            if ui
                .checkbox(&mut all, "All branches")
                .on_hover_text("Include commits not reachable from HEAD")
                .changed()
            {
                app.set_log_all_refs(all);
            }
        });
    });

    ui.add(
        egui::TextEdit::singleline(&mut app.commit_filter)
            .desired_width(f32::INFINITY)
            .hint_text("filter by message, author, hash or ref"),
    );
    ui.add_space(2.0);

    // Clone the rows so the list can be walked while `app` is mutated.
    let rows: Vec<(String, String, String, String, String, String)> = app
        .filtered_commits()
        .iter()
        .map(|c| {
            (
                c.sha.clone(),
                c.short.clone(),
                c.subject.clone(),
                c.author.clone(),
                c.date.clone(),
                c.refs.clone(),
            )
        })
        .collect();

    let mut clicked: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("commit-list-scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if rows.is_empty() {
                let msg = if app.commits.is_empty() {
                    "no commits yet"
                } else {
                    "nothing matches the filter"
                };
                ui.label(egui::RichText::new(msg).color(pal.note_fg).small());
            }
            for (sha, short, subject, author, date, refs) in &rows {
                let selected = app.sel_commit.as_deref() == Some(sha.as_str());
                let badge = if refs.is_empty() {
                    String::new()
                } else {
                    format!("⎇{refs}")
                };
                let mut cells = vec![Cell::new(short, pal.hunk_fg).mono()];
                if !badge.is_empty() {
                    cells.push(Cell::new(&badge, pal.ref_badge));
                }
                cells.push(Cell::new(subject, ui.visuals().text_color()));

                let resp = list_row(
                    ui,
                    selected,
                    &cells,
                    Some(Cell::new(date, pal.note_fg)),
                );
                if resp.clicked() {
                    clicked = Some(sha.clone());
                }
                resp.on_hover_text(format!("{sha}\n{author} · {date}"));
            }

            if app.commits.len() >= app.log_limit {
                ui.add_space(4.0);
                if ui
                    .button("Load more…")
                    .on_hover_text(format!("Currently showing up to {}", app.log_limit))
                    .clicked()
                {
                    app.load_more_commits();
                }
            }
        });

    if let Some(sha) = clicked {
        app.select_commit(&sha);
    }
}

fn commit_file_list(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Files in Commit").strong().small());
        ui.label(
            egui::RichText::new(format!("{}", app.commit_files.len()))
                .color(pal.note_fg)
                .small(),
        );
    });

    let rows: Vec<(String, String, Change)> = app
        .commit_files
        .iter()
        .map(|f| {
            let display = match &f.orig_path {
                Some(o) => format!("{o} → {}", f.path),
                None => f.path.clone(),
            };
            (f.path.clone(), display, f.status)
        })
        .collect();

    let mut clicked: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("commit-files-scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if rows.is_empty() {
                let msg = if app.sel_commit.is_none() {
                    "select a commit"
                } else {
                    "no file changes"
                };
                ui.label(egui::RichText::new(msg).color(pal.note_fg).small());
            }
            for (path, display, status) in &rows {
                let selected = app
                    .sel_file
                    .as_ref()
                    .map(|s| s.pane == Pane::Commit && &s.path == path)
                    .unwrap_or(false);
                let resp = list_row(
                    ui,
                    selected,
                    &[
                        Cell::new(status.letter(), change_color(*status, false))
                            .mono()
                            .strong(),
                        Cell::new(display, ui.visuals().text_color()),
                    ],
                    None,
                );
                if resp.clicked() {
                    clicked = Some(path.clone());
                }
                resp.on_hover_text(format!("{path} — {}", status.label()));
            }
        });

    if let Some(p) = clicked {
        app.select_file(Pane::Commit, &p);
    }
}

fn commit_detail(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.add_space(4.0);

    let Some(meta) = app.commit_meta.as_ref() else {
        ui.label(
            egui::RichText::new("Select a commit to see its details")
                .color(pal.note_fg)
                .small(),
        );
        return;
    };
    let sha = app.sel_commit.clone().unwrap_or_default();

    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(&sha).monospace().small().color(pal.hunk_fg));
        if meta.is_merge() {
            ui.label(
                egui::RichText::new(format!("merge of {} parents", meta.parents.len()))
                    .small()
                    .color(pal.note_fg),
            )
            .on_hover_text(
                "Shown as the diff against the first parent, like `git show -m --first-parent`",
            );
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("{} <{}>", meta.author, meta.email)).small());
        ui.label(egui::RichText::new(&meta.date).color(pal.note_fg).small());
    });
    if !meta.refs.is_empty() {
        ui.label(
            egui::RichText::new(&meta.refs)
                .small()
                .color(pal.ref_badge),
        );
    }

    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("commit-body")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.label(egui::RichText::new(&meta.body).monospace());
        });
}

fn stash_panel(ctx: &egui::Context, app: &mut App) {
    egui::SidePanel::left("stashes")
        .resizable(true)
        .default_width(400.0)
        .min_width(240.0)
        .show(ctx, |ui| {
            let half = (ui.available_height() * 0.55).max(140.0);
            egui::TopBottomPanel::top("stash-list")
                .resizable(true)
                .default_height(half)
                .min_height(100.0)
                .show_inside(ui, |ui| stash_list(ui, app));
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| stash_file_list(ui, app));
        });
}

fn stash_list(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Stashes").strong().small());
        let total = app.stashes.len();
        let shown = app.filtered_stashes().len();
        let count = if shown == total {
            format!("{total}")
        } else {
            format!("{shown}/{total}")
        };
        ui.label(egui::RichText::new(count).color(pal.note_fg).small());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Stash…")
                .on_hover_text("Stash the current working tree")
                .clicked()
            {
                app.open_stash_dialog();
            }
        });
    });

    if !app.stashes.is_empty() {
        ui.add(
            egui::TextEdit::singleline(&mut app.stash_filter)
                .desired_width(f32::INFINITY)
                .hint_text("filter stashes"),
        );
        ui.add_space(2.0);
    }

    // (index, refname, message, branch, date, untracked)
    let rows: Vec<(usize, String, String, String, String, bool)> = app
        .filtered_stashes()
        .iter()
        .map(|s| {
            (
                s.index,
                s.refname(),
                s.message().to_string(),
                s.branch().unwrap_or("").to_string(),
                s.date.clone(),
                s.has_untracked(),
            )
        })
        .collect();

    let mut clicked: Option<usize> = None;
    let mut apply: Option<(usize, bool)> = None;
    let mut drop_it: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("stash-list-scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if rows.is_empty() {
                let msg = if app.stashes.is_empty() {
                    "No stashes. Use Stash… to put the working tree aside."
                } else {
                    "nothing matches the filter"
                };
                ui.label(egui::RichText::new(msg).color(pal.note_fg).small());
            }
            for (index, refname, message, branch, date, untracked) in &rows {
                let selected = app.sel_stash == Some(*index);
                let tag = format!("{{{}}}", index);
                let mut cells = vec![Cell::new(&tag, pal.hunk_fg).mono()];
                if !branch.is_empty() {
                    cells.push(Cell::new(branch, pal.ref_badge));
                }
                cells.push(Cell::new(message, ui.visuals().text_color()));
                let marker = if *untracked { "+untracked" } else { "" };
                if !marker.is_empty() {
                    cells.push(Cell::new(marker, pal.note_fg));
                }

                let resp = list_row(ui, selected, &cells, Some(Cell::new(date, pal.note_fg)));
                if resp.clicked() {
                    clicked = Some(*index);
                }
                resp.on_hover_text(format!(
                    "{refname}\n{}\n{date}",
                    if *untracked {
                        "includes untracked files"
                    } else {
                        "tracked changes only"
                    }
                ))
                .context_menu(|ui| {
                    if ui
                        .button("Apply (keep the stash)")
                        .clicked()
                    {
                        apply = Some((*index, false));
                        ui.close();
                    }
                    if ui.button("Pop (apply and remove)").clicked() {
                        apply = Some((*index, true));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Drop…").clicked() {
                        drop_it = Some(*index);
                        ui.close();
                    }
                });
            }
        });

    if let Some(i) = clicked {
        app.select_stash(i);
    }
    if let Some((i, pop)) = apply {
        app.apply_stash(i, pop);
    }
    if let Some(i) = drop_it {
        app.ask_drop_stash(i);
    }
}

fn stash_file_list(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Files in Stash").strong().small());
        ui.label(
            egui::RichText::new(format!("{}", app.stash_files.len()))
                .color(pal.note_fg)
                .small(),
        );
    });

    let rows: Vec<(String, String, Change, bool)> = app
        .stash_files
        .iter()
        .map(|f| {
            let display = match &f.orig_path {
                Some(o) => format!("{o} → {}", f.path),
                None => f.path.clone(),
            };
            (f.path.clone(), display, f.status, f.untracked)
        })
        .collect();

    let mut clicked: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("stash-files-scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if rows.is_empty() {
                let msg = if app.sel_stash.is_none() {
                    "select a stash"
                } else {
                    "no file changes"
                };
                ui.label(egui::RichText::new(msg).color(pal.note_fg).small());
            }
            for (path, display, status, untracked) in &rows {
                let selected = app
                    .sel_file
                    .as_ref()
                    .map(|s| s.pane == Pane::Stash && &s.path == path)
                    .unwrap_or(false);
                let mut cells = vec![
                    Cell::new(status.letter(), change_color(*status, false))
                        .mono()
                        .strong(),
                    Cell::new(display, ui.visuals().text_color()),
                ];
                if *untracked {
                    cells.push(Cell::new("untracked", pal.note_fg));
                }
                let resp = list_row(ui, selected, &cells, None);
                if resp.clicked() {
                    clicked = Some(path.clone());
                }
                resp.on_hover_text(format!("{path} — {}", status.label()));
            }
        });

    if let Some(p) = clicked {
        app.select_file(Pane::Stash, &p);
    }
}

fn stash_detail(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.add_space(4.0);

    let Some(entry) = app.selected_stash().cloned() else {
        ui.label(
            egui::RichText::new("Select a stash to see what it holds")
                .color(pal.note_fg)
                .small(),
        );
        return;
    };
    let index = entry.index;

    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(entry.refname()).monospace().strong());
        ui.label(
            egui::RichText::new(&entry.sha[..entry.sha.len().min(10)])
                .monospace()
                .small()
                .color(pal.hunk_fg),
        );
        ui.label(egui::RichText::new(&entry.date).color(pal.note_fg).small());
        if entry.has_untracked() {
            ui.label(
                egui::RichText::new("includes untracked files")
                    .small()
                    .color(pal.ref_badge),
            );
        }
    });
    ui.label(egui::RichText::new(entry.message()).monospace());
    if let Some(b) = entry.branch() {
        ui.label(
            egui::RichText::new(format!("stashed from {b}"))
                .color(pal.note_fg)
                .small(),
        );
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui
            .button("Apply")
            .on_hover_text("Restore these changes and keep the stash")
            .clicked()
        {
            app.apply_stash(index, false);
        }
        if ui
            .button("Pop")
            .on_hover_text("Restore these changes and remove the stash")
            .clicked()
        {
            app.apply_stash(index, true);
        }
        if ui
            .add(egui::Button::new("Drop…"))
            .on_hover_text("Delete this stash entry")
            .clicked()
        {
            app.ask_drop_stash(index);
        }
    });
}

/// One cell of text in a list row.
pub struct Cell<'a> {
    pub text: &'a str,
    pub color: Color32,
    pub mono: bool,
    pub strong: bool,
}

impl<'a> Cell<'a> {
    pub fn new(text: &'a str, color: Color32) -> Cell<'a> {
        Cell {
            text,
            color,
            mono: false,
            strong: false,
        }
    }
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }
}

/// Draws a list row whose **entire width** is the click target.
///
/// Laying these out with `selectable_label` made the hit area only as wide as
/// the text, so clicking the empty space beside a short filename did nothing —
/// which is exactly where people click in a list.
fn list_row(ui: &mut Ui, selected: bool, cells: &[Cell], right: Option<Cell>) -> egui::Response {
    let row_h = ui.spacing().interact_size.y.max(18.0);
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );

    let visuals = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, 4.0, visuals.selection.bg_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }

    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let selected_fg = ui.visuals().strong_text_color();

    // Right-aligned cell first, so the left side knows where to stop.
    let mut right_edge = rect.right() - 6.0;
    if let Some(r) = right {
        let font = if r.mono { mono.clone() } else { body.clone() };
        let color = if selected { selected_fg } else { r.color };
        let galley = ui.painter().layout_no_wrap(r.text.to_string(), font, color);
        let pos = egui::pos2(right_edge - galley.size().x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(pos, galley, color);
        right_edge = pos.x - 10.0;
    }

    // Clip the left cells so a long path cannot run over the right-hand cell.
    let mut p = ui.painter().clone();
    p.set_clip_rect(
        egui::Rect::from_min_max(rect.min, egui::pos2(right_edge, rect.max.y))
            .intersect(ui.clip_rect()),
    );

    let mut x = rect.left() + 6.0;
    for c in cells {
        let font = if c.mono { mono.clone() } else { body.clone() };
        let color = if selected && !c.mono { selected_fg } else { c.color };
        let drawn = p.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            c.text,
            font,
            color,
        );
        x = drawn.right() + 8.0;
        if c.strong {
            // Fake a bolder weight by overdrawing one pixel across; egui's
            // default fonts have no separate bold face.
            p.text(
                egui::pos2(drawn.left() + 0.4, rect.center().y),
                egui::Align2::LEFT_CENTER,
                c.text,
                if c.mono { mono.clone() } else { body.clone() },
                color,
            );
        }
    }

    resp
}

fn change_color(c: Change, unmerged: bool) -> Color32 {
    if unmerged {
        return Color32::from_rgb(225, 120, 60);
    }
    match c {
        Change::Added => Color32::from_rgb(90, 180, 110),
        Change::Modified | Change::TypeChanged => Color32::from_rgb(215, 170, 70),
        Change::Deleted => Color32::from_rgb(215, 95, 95),
        Change::Renamed | Change::Copied => Color32::from_rgb(110, 160, 235),
        Change::Untracked => Color32::from_rgb(150, 150, 160),
        _ => Color32::GRAY,
    }
}

fn commit_box(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut amend = app.amend;
        if ui.radio_value(&mut amend, false, "New Commit").clicked() && !amend {
            app.set_amend(false);
        }
        if ui.radio_value(&mut amend, true, "Amend Last Commit").clicked() && amend {
            app.set_amend(true);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can = app.can_commit();
            if ui
                .add_enabled(can, egui::Button::new("Commit"))
                .on_hover_text("Commit the staged changes")
                .clicked()
            {
                app.commit();
            }
            if ui.button("Sign Off").clicked() {
                app.sign_off();
            }
            let staged = app.staged_len();
            let label = if app.amend {
                format!("{staged} file(s) in commit")
            } else {
                format!("{staged} file(s) staged")
            };
            ui.label(egui::RichText::new(label).color(pal.note_fg).small());
        });
    });

    let hint = diff_view::hint_text(app.sel_file.as_ref().map(|s| s.pane));
    if !hint.is_empty() {
        ui.label(egui::RichText::new(hint).color(pal.note_fg).small());
    }

    ui.add_space(2.0);
    egui::ScrollArea::vertical()
        .id_salt("commit-msg")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(&mut app.commit_msg)
                    .hint_text("Commit message")
                    .desired_width(f32::INFINITY),
            );
        });
}

fn status_bar(ctx: &egui::Context, app: &mut App) {
    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        let pal = Palette::new(ui.visuals().dark_mode);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("⎇").monospace());
            ui.label(egui::RichText::new(app.branch_label()).strong());
            if let Some(up) = app.status.upstream.clone() {
                ui.label(egui::RichText::new(format!("→ {up}")).color(pal.note_fg).small());
            }

            if let Some(name) = app.merging.clone() {
                ui.separator();
                let unresolved = app.unresolved().len();
                let text = if unresolved == 0 {
                    format!("merging {name} — all resolved, commit to finish")
                } else {
                    format!("merging {name} — {unresolved} conflict(s) left")
                };
                ui.colored_label(Color32::from_rgb(225, 150, 60), text);
                if ui
                    .small_button("Abort Merge")
                    .on_hover_text("git merge --abort: throw the merge away and go back")
                    .clicked()
                {
                    app.ask_abort_merge();
                }
            }

            ui.separator();

            if let Some(task) = app.running.as_ref() {
                ui.spinner();
                ui.label(egui::RichText::new(task.label.clone()).small());
            } else if let Some(e) = app.error.clone() {
                ui.colored_label(Color32::from_rgb(215, 90, 90), truncate(&e, 160));
                if ui.small_button("✕").clicked() {
                    app.error = None;
                }
            } else if let Some(m) = app.info.clone() {
                ui.colored_label(Color32::from_rgb(110, 175, 120), truncate(&m, 160));
                if ui.small_button("✕").clicked() {
                    app.info = None;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&app.git_version)
                        .color(pal.note_fg)
                        .small(),
                );
                if let Some(label) = app.undo_label() {
                    ui.separator();
                    if ui
                        .button("↺ Undo discard")
                        .on_hover_text(format!("Restore {label}  (⌘Z)"))
                        .clicked()
                    {
                        app.undo_last_discard();
                    }
                }
            });
        });
    });
}

fn truncate(s: &str, n: usize) -> String {
    let one_line: String = s.replace('\n', " ");
    if one_line.chars().count() <= n {
        one_line
    } else {
        let cut: String = one_line.chars().take(n).collect();
        format!("{cut}…")
    }
}

fn modals(ctx: &egui::Context, app: &mut App) {
    if let Some(c) = app.confirm.as_ref() {
        let (title, body, action) = match c {
            Confirm::DropStash { label, .. } => (
                "Drop stash?",
                format!("Delete {label}.\n\nThis can be undone with ⌘Z straight afterwards."),
                "Drop",
            ),
            Confirm::DiscardFile { path, untracked } => (
                "Discard file?",
                if *untracked {
                    format!("Delete the untracked file {path}.\nThis cannot be undone.")
                } else {
                    format!("Revert all changes in {path}.\nThis cannot be undone.")
                },
                "Discard",
            ),
            Confirm::AbortMerge => (
                "Abort merge?",
                "Throw the merge away and put the working tree back where it was before it started. Any conflict resolutions made so far are lost.".to_string(),
                "Abort Merge",
            ),
        };
        let mut yes = false;
        let mut no = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(body);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        no = true;
                    }
                    if ui
                        .add(egui::Button::new(action).fill(Color32::from_rgb(160, 60, 60)))
                        .clicked()
                    {
                        yes = true;
                    }
                });
            });
        if yes {
            app.confirm_yes();
        } else if no {
            app.confirm = None;
        }
    }

    if app.show_stash_dialog {
        let mut go = false;
        let mut cancel = false;
        let dirty = app.status.unstaged().len() + app.status.staged().len();
        let has_untracked = app.status.entries.iter().any(|e| e.untracked);
        egui::Window::new("Stash Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let pal = Palette::new(ui.visuals().dark_mode);
                ui.label(
                    egui::RichText::new(format!(
                        "Set aside changes in {dirty} file(s) and return the \
                         working tree to HEAD."
                    ))
                    .small(),
                );
                ui.add_space(6.0);
                ui.label("Message (optional)");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.stash_message)
                        .desired_width(320.0)
                        .hint_text("what you were in the middle of"),
                );
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }

                ui.add_space(6.0);
                ui.add_enabled_ui(has_untracked, |ui| {
                    ui.checkbox(&mut app.stash_untracked, "Include untracked files (-u)");
                });
                if !has_untracked {
                    ui.label(
                        egui::RichText::new("no untracked files to include")
                            .color(pal.note_fg)
                            .small(),
                    );
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Stash").clicked() {
                        go = true;
                    }
                });
            });
        if go {
            app.create_stash();
        } else if cancel {
            app.show_stash_dialog = false;
            app.stash_message.clear();
        }
    }

    if app.show_push {
        let branch = app.status.branch.clone().unwrap_or_default();
        let remotes = app.remotes.clone();
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Push")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let pal = Palette::new(ui.visuals().dark_mode);
                egui::Grid::new("push-grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Branch");
                        ui.label(egui::RichText::new(&branch).strong());
                        ui.end_row();

                        ui.label("Remote");
                        egui::ComboBox::from_id_salt("push-remote")
                            .selected_text(&app.push_remote)
                            .show_ui(ui, |ui| {
                                for r in &remotes {
                                    ui.selectable_value(&mut app.push_remote, r.clone(), r);
                                }
                            });
                        ui.end_row();

                        ui.label("Tracking");
                        match app.status.upstream.clone() {
                            Some(up) => {
                                ui.label(egui::RichText::new(up).color(pal.note_fg).small());
                            }
                            None => {
                                ui.label(
                                    egui::RichText::new("none yet")
                                        .color(pal.note_fg)
                                        .small(),
                                );
                            }
                        }
                        ui.end_row();
                    });

                ui.add_space(6.0);
                ui.checkbox(
                    &mut app.push_set_upstream,
                    "Set as upstream for this branch (-u)",
                );
                ui.checkbox(&mut app.push_tags, "Also push tags");
                ui.checkbox(
                    &mut app.push_force_lease,
                    "Force with lease (overwrite the remote branch)",
                );
                if app.push_force_lease {
                    ui.label(
                        egui::RichText::new(
                            "Refuses if the remote moved in a way you have not fetched, \
                             so it cannot clobber someone else's work unseen.",
                        )
                        .color(pal.note_fg)
                        .small(),
                    );
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(
                            !app.push_remote.is_empty(),
                            egui::Button::new(format!("Push to {}", app.push_remote)),
                        )
                        .clicked()
                    {
                        go = true;
                    }
                });
            });
        if go {
            app.start_push();
        } else if cancel {
            app.show_push = false;
        }
    }

    if let Some(outcome) = app.outcome.as_ref() {
        let ok = outcome.ok;
        let title = outcome.title.clone();
        let body = outcome.output.clone();
        let mut close = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if !ok {
                    ui.colored_label(
                        Color32::from_rgb(215, 90, 90),
                        "git push reported an error:",
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt("push-output")
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&body).monospace())
                                .wrap(),
                        );
                    });
                ui.add_space(8.0);
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if close {
            app.outcome = None;
        }
    }

    if app.show_new_branch {
        let mut create = false;
        let mut cancel = false;
        egui::Window::new("New Branch")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Branch name");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.new_branch)
                        .desired_width(260.0)
                        .hint_text("feature/thing"),
                );
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    create = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(
                            !app.new_branch.trim().is_empty(),
                            egui::Button::new("Create"),
                        )
                        .clicked()
                    {
                        create = true;
                    }
                });
            });
        if create {
            app.create_branch();
        } else if cancel {
            app.show_new_branch = false;
            app.new_branch.clear();
        }
    }
}

/// Global shortcuts that work regardless of which pane has focus.
pub fn global_keys(ctx: &egui::Context, app: &mut App) {
    let typing = ctx.memory(|m| m.focused().is_some());
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::F) {
            app.find_open();
        }
        // These must work while the find field has focus, hence no `typing`
        // guard, and Escape is consumed here so it closes the bar rather than
        // clearing the diff selection.
        if app.find.open {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                app.find_close();
            }
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::G) {
                app.find_next();
            }
            if i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::G,
            ) {
                app.find_prev();
            }
        }
        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::R) {
            app.rescan();
        }
        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::O) {
            app.open_dialog();
        }
        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
            app.commit();
        }
        if !typing {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::F5) {
                app.rescan();
            }
            // Guarded on focus so it does not shadow the commit box's own undo.
            if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
                app.undo_last_discard();
            }
        }
    });
}
