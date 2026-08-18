// Keep a console window from appearing alongside the GUI on Windows release
// builds, while leaving stdout available in debug builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gitgui::app::App;
use gitgui::ui;

struct GitGui {
    app: App,
}

impl eframe::App for GitGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let busy = self.app.poll_task();
        ui::global_keys(ctx, &mut self.app);
        ui::draw(ctx, &mut self.app);
        // egui only redraws on input, so a background push would otherwise
        // finish without the window noticing.
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }
}

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 840.0])
        .with_min_inner_size([760.0, 480.0])
        .with_title("Git GUI");

    match eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
    {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(err) => eprintln!("could not decode the window icon: {err}"),
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Git GUI",
        options,
        Box::new(|cc| {
            cc.egui_ctx.style_mut(|s| {
                s.interaction.selectable_labels = false;
            });
            Ok(Box::new(GitGui { app: App::new() }))
        }),
    )
}
