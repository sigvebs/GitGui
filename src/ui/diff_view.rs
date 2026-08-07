use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::app::{App, Loaded, Op, Pane, Row};
use crate::git::diff::LineKind;
use crate::theme::Palette;

enum Pending {
    Lines(Op),
    Hunk(Op, usize),
    SelectHunk(usize),
}

#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    page_up: bool,
    page_down: bool,
    extend: bool,
    select_all: bool,
    clear: bool,
    primary: bool,
    discard: bool,
    hunk: bool,
}

pub fn show(ui: &mut Ui, app: &mut App) {
    header(ui, app);
    find_bar(ui, app);

    if app.diff.is_none() {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.label(
                egui::RichText::new("Select a file to see its changes")
                    .color(Palette::new(ui.visuals().dark_mode).note_fg),
            );
        });
        return;
    }

    let pal = Palette::new(ui.visuals().dark_mode);
    let font = FontId::monospace(app.font_size);
    let row_h = ui.fonts(|f| f.row_height(&font)).ceil().max(8.0);
    let char_w = ui.fonts(|f| f.glyph_width(&font, '0')).max(1.0);
    let Some(pane) = app.sel_file.as_ref().map(|s| s.pane) else {
        return;
    };

    let keys = read_keys(ui, pane);
    let mut pending: Option<Pending> = None;

    // Copy out the hit list so `loaded` can be borrowed mutably below.
    let hits: Vec<(usize, usize, usize, bool)> = app
        .find
        .matches
        .iter()
        .enumerate()
        .map(|(i, m)| (m.row, m.start, m.end, i == app.find.current))
        .collect();

    {
        let loaded = app.diff.as_mut().expect("checked above");
        apply_keys(loaded, &keys, pane, &mut pending);
        paint_rows(
            ui, loaded, &pal, &font, row_h, char_w, pane, &hits, &mut pending,
        );
    }

    match pending {
        Some(Pending::Lines(op)) => app.apply_lines(op),
        Some(Pending::Hunk(op, h)) => app.apply_hunk(op, h),
        Some(Pending::SelectHunk(h)) => {
            if let Some(d) = app.diff.as_mut() {
                d.select_hunk(h);
            }
        }
        None => {}
    }
}

fn header(ui: &mut Ui, app: &mut App) {
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        let (title, sub) = match app.sel_file.as_ref() {
            Some(s) => (
                s.path.clone(),
                match s.pane {
                    Pane::Unstaged => "unstaged changes",
                    Pane::Staged => "staged changes",
                    Pane::Commit => "in this commit",
                    Pane::Stash => "in this stash",
                },
            ),
            None => ("No file selected".to_string(), ""),
        };
        ui.label(egui::RichText::new(title).strong());
        if !sub.is_empty() {
            ui.label(egui::RichText::new(sub).color(pal.note_fg).small());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(pane) = app.sel_file.as_ref().map(|s| s.pane) {
                let has_hunks = app
                    .diff
                    .as_ref()
                    .map(|d| !d.fd.hunks.is_empty())
                    .unwrap_or(false);
                ui.add_enabled_ui(has_hunks, |ui| match pane {
                    Pane::Unstaged => {
                        if ui
                            .button("Discard")
                            .on_hover_text("Revert the selected lines in the working tree")
                            .clicked()
                        {
                            app.apply_lines(Op::Discard);
                        }
                        if ui
                            .button("Stage Lines")
                            .on_hover_text("Stage the selected lines  (Space)")
                            .clicked()
                        {
                            app.apply_lines(Op::Stage);
                        }
                    }
                    Pane::Staged => {
                        if ui
                            .button("Unstage Lines")
                            .on_hover_text("Unstage the selected lines  (Space)")
                            .clicked()
                        {
                            app.apply_lines(Op::Unstage);
                        }
                    }
                    // Nothing to stage out of an immutable snapshot.
                    Pane::Commit | Pane::Stash => {}
                });
            }
            if let Some(s) = app.selection_summary() {
                ui.label(egui::RichText::new(s).color(pal.note_fg).small());
            }
        });
    });
    ui.separator();
}

fn find_bar(ui: &mut Ui, app: &mut App) {
    if !app.find.open {
        return;
    }
    let pal = Palette::new(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Find").small());
        let resp = ui.add(
            egui::TextEdit::singleline(&mut app.find.query)
                .desired_width(240.0)
                .hint_text("search this diff"),
        );
        if std::mem::take(&mut app.find.request_focus) {
            resp.request_focus();
        }
        if resp.changed() {
            app.find_recompute();
        }

        // Enter walks matches without leaving the field.
        if resp.has_focus() {
            let (enter, shift) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.modifiers.shift,
                )
            });
            if enter {
                if shift {
                    app.find_prev();
                } else {
                    app.find_next();
                }
            }
        }

        let empty = app.find.matches.is_empty();
        ui.add_enabled_ui(!empty, |ui| {
            if ui.small_button("◀").on_hover_text("Previous  (⇧⏎ / ⌘⇧G)").clicked() {
                app.find_prev();
            }
            if ui.small_button("▶").on_hover_text("Next  (⏎ / ⌘G)").clicked() {
                app.find_next();
            }
        });

        let mut cs = app.find.case_sensitive;
        if ui
            .checkbox(&mut cs, "Aa")
            .on_hover_text("Match case")
            .changed()
        {
            app.find.case_sensitive = cs;
            app.find_recompute();
        }

        let status = app.find_status();
        if !status.is_empty() {
            let color = if empty {
                egui::Color32::from_rgb(200, 110, 110)
            } else {
                pal.note_fg
            };
            ui.label(egui::RichText::new(status).color(color).small());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Close  (Esc)").clicked() {
                app.find_close();
            }
        });
    });
    ui.separator();
}

fn read_keys(ui: &Ui, pane: Pane) -> Keys {
    // Never steal keys while the commit box or a text field has focus.
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return Keys::default();
    }
    ui.input_mut(|i| {
        let extend = i.modifiers.shift;
        let cmd = i.modifiers.command;
        Keys {
            up: i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::K),
            down: i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::J),
            page_up: i.key_pressed(egui::Key::PageUp),
            page_down: i.key_pressed(egui::Key::PageDown),
            extend,
            select_all: cmd && i.key_pressed(egui::Key::A),
            clear: i.key_pressed(egui::Key::Escape),
            primary: i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter),
            discard: pane == Pane::Unstaged
                && (i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete)),
            hunk: i.key_pressed(egui::Key::H),
        }
    })
}

fn apply_keys(loaded: &mut Loaded, k: &Keys, pane: Pane, pending: &mut Option<Pending>) {
    let step = 12;
    if k.up {
        loaded.move_cursor(-1, k.extend);
        loaded.scroll_to_cursor = true;
    }
    if k.down {
        loaded.move_cursor(1, k.extend);
        loaded.scroll_to_cursor = true;
    }
    if k.page_up {
        loaded.move_cursor(-step, k.extend);
        loaded.scroll_to_cursor = true;
    }
    if k.page_down {
        loaded.move_cursor(step, k.extend);
        loaded.scroll_to_cursor = true;
    }
    if k.select_all {
        loaded.select_all();
    }
    if k.clear {
        loaded.sel.clear();
    }
    let Some(primary) = pane_op(pane) else {
        // Read-only pane: navigation and selection only.
        return;
    };
    if k.hunk {
        if let Some(h) = loaded.hunk_of_row(loaded.cursor) {
            *pending = Some(Pending::Hunk(primary, h));
        }
    }
    if k.primary {
        *pending = Some(Pending::Lines(primary));
    }
    if k.discard {
        *pending = Some(Pending::Lines(Op::Discard));
    }
}

/// The stage/unstage action a pane offers, or `None` when it is read-only.
fn pane_op(pane: Pane) -> Option<Op> {
    match pane {
        Pane::Unstaged => Some(Op::Stage),
        Pane::Staged => Some(Op::Unstage),
        Pane::Commit | Pane::Stash => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_rows(
    ui: &mut Ui,
    loaded: &mut Loaded,
    pal: &Palette,
    font: &FontId,
    row_h: f32,
    char_w: f32,
    pane: Pane,
    hits: &[(usize, usize, usize, bool)],
    pending: &mut Option<Pending>,
) {
    let digits = line_no_digits(loaded);
    let gutter_w = (digits as f32 * 2.0 + 3.0) * char_w;
    let content_w = gutter_w + (loaded.max_chars as f32 + 2.0) * char_w;

    // Row pitch must equal row_h exactly for the offset maths below.
    ui.spacing_mut().item_spacing.y = 0.0;

    let mut area = egui::ScrollArea::both()
        .id_salt("diff-rows")
        .auto_shrink([false; 2]);

    if std::mem::take(&mut loaded.scroll_to_cursor) {
        let top = loaded.cursor as f32 * row_h;
        let bottom = top + row_h;
        let view = loaded.viewport_h.max(row_h);
        let y = if top < loaded.scroll_y {
            top
        } else if bottom > loaded.scroll_y + view {
            bottom - view
        } else {
            loaded.scroll_y
        };
        area = area.vertical_scroll_offset(y);
    }

    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    if !primary_down {
        loaded.dragging = false;
    }

    let n = loaded.rows.len();
    let out = area.show_rows(ui, row_h, n, |ui, range| {
        let width = content_w.max(ui.available_width());
        for i in range {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click_and_drag());

            // Interaction first, so the paint below reflects this frame.
            handle_row(
                ui,
                loaded,
                i,
                &rect,
                &resp,
                primary_pressed,
                pane,
                pending,
            );

            if !ui.is_rect_visible(rect) {
                continue;
            }
            paint_row(
                ui, loaded, i, rect, pal, font, row_h, char_w, gutter_w, digits, hits,
            );
        }
    });

    loaded.scroll_y = out.state.offset.y;
    loaded.viewport_h = out.inner_rect.height();
}

#[allow(clippy::too_many_arguments)]
fn handle_row(
    ui: &Ui,
    loaded: &mut Loaded,
    i: usize,
    rect: &Rect,
    resp: &egui::Response,
    primary_pressed: bool,
    pane: Pane,
    pending: &mut Option<Pending>,
) {
    let is_hunk_header = matches!(loaded.rows.get(i), Some(Row::Hunk(_)));
    let hunk = loaded.hunk_of_row(i);

    if primary_pressed && resp.contains_pointer() {
        let (shift, cmd) = ui.input(|inp| (inp.modifiers.shift, inp.modifiers.command));
        if is_hunk_header {
            if let Some(h) = hunk {
                *pending = Some(Pending::SelectHunk(h));
            }
        } else if shift {
            loaded.extend_to(i);
        } else if cmd {
            loaded.toggle(i);
        } else {
            loaded.select_only(i);
            loaded.dragging = true;
        }
    }

    // Extend while the button is held, using the pointer position rather than
    // hover state, which egui suppresses during a drag.
    if loaded.dragging && !is_hunk_header {
        if let Some(p) = ui.input(|inp| inp.pointer.interact_pos()) {
            if p.y >= rect.top() && p.y < rect.bottom() && i != loaded.cursor {
                loaded.extend_to(i);
            }
        }
    }

    if resp.double_clicked() {
        if let (Some(h), Some(op)) = (hunk, pane_op(pane)) {
            *pending = Some(Pending::Hunk(op, h));
        }
    }

    let has_sel = loaded.selected_change_count() > 0;
    resp.context_menu(|ui| {
        match pane {
            Pane::Unstaged => {
                if ui
                    .add_enabled(has_sel, egui::Button::new("Stage Lines"))
                    .clicked()
                {
                    *pending = Some(Pending::Lines(Op::Stage));
                    ui.close();
                }
                if let Some(h) = hunk {
                    if ui.button("Stage Hunk").clicked() {
                        *pending = Some(Pending::Hunk(Op::Stage, h));
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(has_sel, egui::Button::new("Discard Lines"))
                    .clicked()
                {
                    *pending = Some(Pending::Lines(Op::Discard));
                    ui.close();
                }
                if let Some(h) = hunk {
                    if ui.button("Discard Hunk").clicked() {
                        *pending = Some(Pending::Hunk(Op::Discard, h));
                        ui.close();
                    }
                }
            }
            Pane::Staged => {
                if ui
                    .add_enabled(has_sel, egui::Button::new("Unstage Lines"))
                    .clicked()
                {
                    *pending = Some(Pending::Lines(Op::Unstage));
                    ui.close();
                }
                if let Some(h) = hunk {
                    if ui.button("Unstage Hunk").clicked() {
                        *pending = Some(Pending::Hunk(Op::Unstage, h));
                        ui.close();
                    }
                }
            }
            // Commits and stashes are immutable; nothing to offer.
            Pane::Commit | Pane::Stash => {}
        }
        ui.separator();
        if ui.button("Select All Lines").clicked() {
            loaded.select_all();
            ui.close();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &Ui,
    loaded: &Loaded,
    i: usize,
    rect: Rect,
    pal: &Palette,
    font: &FontId,
    row_h: f32,
    char_w: f32,
    gutter_w: f32,
    digits: usize,
    hits: &[(usize, usize, usize, bool)],
) {
    let p = ui.painter();
    let selected = loaded.sel.contains(&i);
    let is_cursor = loaded.cursor == i;

    match loaded.rows.get(i) {
        Some(Row::Note(text)) => {
            p.rect_filled(rect, 0.0, pal.hunk_bg);
            p.text(
                egui::pos2(rect.left() + char_w, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font.clone(),
                pal.note_fg,
            );
        }
        Some(Row::Hunk(h)) => {
            let hunk = &loaded.fd.hunks[*h];
            p.rect_filled(rect, 0.0, pal.hunk_bg);
            p.text(
                egui::pos2(rect.left() + char_w, rect.center().y),
                Align2::LEFT_CENTER,
                hunk.header(),
                font.clone(),
                pal.hunk_fg,
            );
        }
        Some(Row::Line { hunk, line }) => {
            let l = &loaded.fd.hunks[*hunk].lines[*line];
            let (bg, fg) = match l.kind {
                LineKind::Addition => (Some(pal.add_bg), pal.add_fg),
                LineKind::Deletion => (Some(pal.del_bg), pal.del_fg),
                LineKind::Context => (None, pal.ctx_fg),
                LineKind::NoNewline => (None, pal.note_fg),
            };
            // Gutter sits behind everything so numbers stay legible.
            let gutter = Rect::from_min_size(rect.min, Vec2::new(gutter_w, row_h));
            p.rect_filled(gutter, 0.0, pal.gutter_bg);
            if let Some(c) = bg {
                let body = Rect::from_min_max(
                    egui::pos2(rect.left() + gutter_w, rect.top()),
                    rect.max,
                );
                p.rect_filled(body, 0.0, c);
            }
            if selected {
                p.rect_filled(rect, 0.0, pal.sel_bg);
                p.rect_filled(
                    Rect::from_min_size(rect.min, Vec2::new(3.0, row_h)),
                    0.0,
                    pal.sel_bar,
                );
            }

            // Search hits sit above the row tint but below the glyphs. Columns
            // map exactly to pixels because the font is monospaced.
            for (row, start, end, is_current) in hits.iter().copied() {
                if row != i {
                    continue;
                }
                let x0 = rect.left() + gutter_w + start as f32 * char_w;
                let x1 = rect.left() + gutter_w + end as f32 * char_w;
                p.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(x0, rect.top()),
                        egui::pos2(x1, rect.bottom()),
                    ),
                    0.0,
                    if is_current {
                        pal.find_current_bg
                    } else {
                        pal.find_bg
                    },
                );
            }

            let num = |n: Option<u32>| match n {
                Some(v) => format!("{v:>digits$}", digits = digits),
                None => " ".repeat(digits),
            };
            p.text(
                egui::pos2(rect.left() + char_w * 0.5, rect.center().y),
                Align2::LEFT_CENTER,
                format!("{} {}", num(l.old_no), num(l.new_no)),
                font.clone(),
                pal.gutter_fg,
            );

            let marker = match l.kind {
                LineKind::Addition => '+',
                LineKind::Deletion => '-',
                LineKind::Context => ' ',
                LineKind::NoNewline => ' ',
            };
            let body = if l.kind == LineKind::NoNewline {
                l.text.clone()
            } else {
                format!("{marker}{}", l.text)
            };
            p.text(
                egui::pos2(rect.left() + gutter_w, rect.center().y),
                Align2::LEFT_CENTER,
                body,
                font.clone(),
                fg,
            );
        }
        None => {}
    }

    if is_cursor {
        p.rect_stroke(
            rect.shrink(0.5),
            0.0,
            Stroke::new(1.0_f32, pal.cursor_bar.gamma_multiply(0.7)),
            StrokeKind::Inside,
        );
    }
}

fn line_no_digits(loaded: &Loaded) -> usize {
    let max = loaded
        .fd
        .hunks
        .iter()
        .map(|h| (h.old_start + h.old_count).max(h.new_start + h.new_count))
        .max()
        .unwrap_or(0);
    let mut d = 1;
    let mut v = max;
    while v >= 10 {
        v /= 10;
        d += 1;
    }
    d.max(3)
}

/// Legend shown under the diff so the interaction model is discoverable.
pub fn hint_text(pane: Option<Pane>) -> &'static str {
    match pane {
        Some(Pane::Unstaged) => {
            "click a line · shift-click or drag for a range · ⌘-click to toggle · Space stages · H stages the hunk · ⌫ discards"
        }
        Some(Pane::Staged) => {
            "click a line · shift-click or drag for a range · ⌘-click to toggle · Space unstages · H unstages the hunk"
        }
        Some(Pane::Commit) | Some(Pane::Stash) => "read-only · ⌘F to search this diff",
        None => "",
    }
}
