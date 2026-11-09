//! Settings saved to `~/.config/mindwiki/config.toml`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub wiki_root: PathBuf,
    #[serde(default = "default_autosave")]
    pub autosave_secs: f32,
    #[serde(default)]
    pub last_page: Option<String>,
}

fn default_autosave() -> f32 {
    2.0
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Config { wiki_root: home.join("wiki"), autosave_secs: default_autosave(), last_page: None }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("mindwiki").join("config.toml"))
}

impl Config {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }
}
