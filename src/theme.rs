use egui::Color32;

#[derive(Clone, Copy)]
pub struct Palette {
    pub add_bg: Color32,
    pub del_bg: Color32,
    pub add_fg: Color32,
    pub del_fg: Color32,
    pub ctx_fg: Color32,
    pub hunk_bg: Color32,
    pub hunk_fg: Color32,
    pub gutter_fg: Color32,
    pub gutter_bg: Color32,
    pub sel_bg: Color32,
    pub sel_bar: Color32,
    pub cursor_bar: Color32,
    pub note_fg: Color32,
    pub find_bg: Color32,
    pub find_current_bg: Color32,
    pub ref_badge: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Palette {
        if dark {
            Palette {
                add_bg: Color32::from_rgb(24, 54, 33),
                del_bg: Color32::from_rgb(63, 30, 33),
                add_fg: Color32::from_rgb(178, 235, 190),
                del_fg: Color32::from_rgb(245, 185, 190),
                ctx_fg: Color32::from_rgb(178, 182, 190),
                hunk_bg: Color32::from_rgb(34, 42, 58),
                hunk_fg: Color32::from_rgb(140, 170, 225),
                gutter_fg: Color32::from_rgb(110, 116, 128),
                gutter_bg: Color32::from_rgb(26, 28, 33),
                sel_bg: Color32::from_rgba_unmultiplied(90, 150, 255, 60),
                sel_bar: Color32::from_rgb(110, 165, 255),
                cursor_bar: Color32::from_rgb(200, 200, 210),
                note_fg: Color32::from_rgb(150, 155, 165),
                find_bg: Color32::from_rgba_unmultiplied(235, 200, 60, 70),
                find_current_bg: Color32::from_rgba_unmultiplied(255, 165, 40, 165),
                ref_badge: Color32::from_rgb(120, 175, 140),
            }
        } else {
            Palette {
                add_bg: Color32::from_rgb(224, 248, 228),
                del_bg: Color32::from_rgb(255, 232, 233),
                add_fg: Color32::from_rgb(18, 74, 32),
                del_fg: Color32::from_rgb(122, 22, 28),
                ctx_fg: Color32::from_rgb(48, 52, 60),
                hunk_bg: Color32::from_rgb(228, 236, 250),
                hunk_fg: Color32::from_rgb(38, 88, 165),
                gutter_fg: Color32::from_rgb(140, 146, 158),
                gutter_bg: Color32::from_rgb(246, 247, 249),
                sel_bg: Color32::from_rgba_unmultiplied(60, 130, 246, 48),
                sel_bar: Color32::from_rgb(40, 110, 235),
                cursor_bar: Color32::from_rgb(70, 74, 82),
                note_fg: Color32::from_rgb(110, 116, 128),
                find_bg: Color32::from_rgba_unmultiplied(250, 215, 90, 130),
                find_current_bg: Color32::from_rgba_unmultiplied(255, 160, 30, 190),
                ref_badge: Color32::from_rgb(40, 120, 75),
            }
        }
    }
}
