//! Tiny plain-text settings store. Avoids a config crate so nothing extra has
//! to survive cross-compilation.

use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 12;

fn config_dir() -> Option<PathBuf> {
    // Lets tests (and portable installs) redirect settings away from the real
    // user profile.
    if let Some(p) = std::env::var_os("GITGUI_CONFIG_DIR") {
        return Some(PathBuf::from(p));
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("gitgui"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("gitgui"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(x).join("gitgui"));
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config").join("gitgui"))
    }
}

fn file(name: &str) -> Option<PathBuf> {
    Some(config_dir()?.join(name))
}

fn write(name: &str, contents: &str) {
    let Some(path) = file(name) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, contents);
}

pub fn load_recent() -> Vec<PathBuf> {
    let Some(path) = file("recent.txt") else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.join(".git").exists() || p.join(".git").is_file())
        .take(MAX_RECENT)
        .collect()
}

pub fn push_recent(list: &mut Vec<PathBuf>, root: &Path) {
    list.retain(|p| p != root);
    list.insert(0, root.to_path_buf());
    list.truncate(MAX_RECENT);
    let text: String = list
        .iter()
        .map(|p| format!("{}\n", p.display()))
        .collect();
    write("recent.txt", &text);
}

pub fn load_font_size() -> Option<f32> {
    let path = file("font_size.txt")?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: f32 = text.trim().parse().ok()?;
    if (8.0..=32.0).contains(&v) {
        Some(v)
    } else {
        None
    }
}

pub fn save_font_size(v: f32) {
    write("font_size.txt", &format!("{v}"));
}
