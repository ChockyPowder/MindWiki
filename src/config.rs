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
    /// True once the first-run tour has been finished or skipped.
    #[serde(default)]
    pub intro_done: bool,
    /// Some(true) = dark, Some(false) = light, None = follow the system.
    #[serde(default)]
    pub dark_mode: Option<bool>,
    /// The server this app is connected to, if any. Kept last so the TOML
    /// file stays valid (tables come after plain values).
    #[serde(default)]
    pub remote: Option<crate::remote::Remote>,
}

fn default_autosave() -> f32 {
    2.0
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Config { wiki_root: home.join("wiki"), autosave_secs: default_autosave(), last_page: None, intro_done: false, dark_mode: None, remote: None }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("mindwiki").join("config.toml"))
}

impl Config {
    /// The folder the app works in: the local wiki, or the local copy of the
    /// connected server's wiki.
    pub fn active_root(&self) -> PathBuf {
        match &self.remote {
            Some(r) => {
                let slug: String = format!("{}_{}", r.url.trim_start_matches("https://"), r.username)
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
                    .collect();
                dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("mindwiki").join("remotes").join(slug)
            }
            None => self.wiki_root.clone(),
        }
    }

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
            // The file can hold a login token, so only the owner may read it.
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                opts.mode(0o600);
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
            if let Ok(mut f) = opts.open(&p) {
                use std::io::Write;
                let _ = f.write_all(s.as_bytes());
            }
        }
    }
}
