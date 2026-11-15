//! What the viewer remembers between runs, whatever the model: the files
//! opened last, the theme, the workspace shown and how each one is laid out. A JSON file
//! in the user's configuration folder (`%APPDATA%\llevant\viewer.json`,
//! `~/.config/llevant/viewer.json`); unreadable or missing, the defaults.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::frame::Frame;

/// Files listed at most.
const RECENT: usize = 12;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    /// `dark`, `light` or `system`.
    pub theme: Option<String>,
    /// The workspace shown and how each one is laid out (dropped if it
    /// does not read).
    #[serde(deserialize_with = "lenient")]
    pub frame: Option<Frame>,
    /// Reload the model as soon as its input changes on disk (only what
    /// changed is built again), instead of offering to.
    pub auto_reload: bool,
}

/// `None` for a value that does not read, so that one stale setting does not
/// lose the others.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()))
}

impl Settings {
    /// Where they are kept, if there is a configuration folder.
    pub fn path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("LLEVANT_VIEWER_SETTINGS") {
            return Some(p.into());
        }
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("llevant").join("viewer.json"))
    }

    pub fn load() -> Settings {
        Settings::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Write them; quietly gives up (they are a convenience).
    pub fn save(&self) {
        let Some(p) = Settings::path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, t);
        }
    }

    /// `path` opened: first in the list.
    pub fn opened(&mut self, path: &std::path::Path) {
        let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|q| *q != p);
        self.recent.insert(0, p);
        self.recent.truncate(RECENT);
    }

    pub fn theme(&self) -> egui::ThemePreference {
        match self.theme.as_deref() {
            Some("light") => egui::ThemePreference::Light,
            Some("system") => egui::ThemePreference::System,
            _ => egui::ThemePreference::Dark,
        }
    }

    pub fn set_theme(&mut self, t: egui::ThemePreference) {
        self.theme = Some(
            match t {
                egui::ThemePreference::Dark => "dark",
                egui::ThemePreference::Light => "light",
                egui::ThemePreference::System => "system",
            }
            .into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_most_recent_first_without_repeats() {
        let mut s = Settings::default();
        for p in ["a.i", "b.i", "a.i"] {
            s.opened(std::path::Path::new(p));
        }
        let names: Vec<String> = s
            .recent
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into())
            .collect();
        assert_eq!(names, ["a.i", "b.i"]);
        s.set_theme(egui::ThemePreference::Light);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.theme(), egui::ThemePreference::Light);
        assert_eq!(back.recent, s.recent);
    }
}
