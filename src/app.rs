//! App state and the non-drawing logic: opening/saving pages, history,
//! the file watcher, shortcuts and autosave.

use crate::config::Config;
use crate::index::Index;
use crate::page::key;
use crate::store::Wiki;
use egui_commonmark::CommonMarkCache;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Split,
    Preview,
}

pub enum Dialog {
    None,
    NewPage(String),
    Rename(String),
    Delete,
    Settings { root: String, autosave: f32 },
}

pub struct WikiApp {
    pub config: Config,
    pub wiki: Wiki,
    pub index: Index,

    pub current: Option<String>,
    pub buffer: String,
    pub dirty: bool,
    pub last_edit: f64,
    pub mode: Mode,

    pub history: Vec<String>,
    pub hist_pos: usize,

    pub filter: String,
    pub tag_filter: Option<String>,

    pub search_open: bool,
    pub search_focus: bool,
    pub query: String,
    pub search_sel: usize,

    pub md_cache: CommonMarkCache,
    pub dialog: Dialog,
    pub status: Option<(String, f64)>,
    /// Current step of the intro tour, if it is showing.
    pub tour: Option<usize>,

    _watcher: Option<RecommendedWatcher>,
    rx: Receiver<PathBuf>,
}

impl WikiApp {
    pub fn new(ctx: &egui::Context, config: Config) -> Self {
        let (watcher, rx) = start_watcher(&config.wiki_root, ctx.clone());
        let mut app = WikiApp {
            wiki: Wiki { root: config.wiki_root.clone(), pages: BTreeMap::new() },
            config,
            index: Index::default(),
            current: None,
            buffer: String::new(),
            dirty: false,
            last_edit: 0.0,
            mode: Mode::Preview,
            history: Vec::new(),
            hist_pos: 0,
            filter: String::new(),
            tag_filter: None,
            search_open: false,
            search_focus: false,
            query: String::new(),
            search_sel: 0,
            tour: None,
            md_cache: CommonMarkCache::default(),
            dialog: Dialog::None,
            status: None,
            _watcher: watcher,
            rx,
        };
        app.load_wiki(ctx);
        if !app.config.intro_done {
            app.start_tour();
        }
        app
    }

    /// (Re)opens the wiki folder from `config.wiki_root` and opens a start page.
    fn load_wiki(&mut self, ctx: &egui::Context) {
        let root = self.config.wiki_root.clone();
        match Wiki::open(&root) {
            Ok(mut w) => {
                if let Err(e) = w.seed_if_empty() {
                    self.set_status(format!("Could not write Welcome page: {e}"));
                }
                self.wiki = w;
            }
            Err(e) => {
                self.set_status(format!("Could not open {}: {e}", root.display()));
                self.wiki = Wiki { root, pages: BTreeMap::new() };
            }
        }
        self.reindex();
        self.history.clear();
        self.hist_pos = 0;
        self.current = None;
        let start = self
            .config
            .last_page
            .clone()
            .filter(|t| self.wiki.exists(t))
            .or_else(|| self.wiki.get("Welcome").map(|p| p.title.clone()))
            .or_else(|| self.wiki.pages.values().next().map(|p| p.title.clone()));
        if let Some(t) = start {
            self.navigate(&t, ctx);
        }
    }

    pub fn reindex(&mut self) {
        self.index = Index::build(&self.wiki);
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), f64::NAN));
    }

    /// Writes the editor buffer to disk if it has unsaved changes.
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(title) = self.current.clone() else { return };
        match self.wiki.save(&title, &self.buffer) {
            Ok(()) => {
                self.dirty = false;
                self.reindex();
            }
            Err(e) => self.set_status(format!("Save failed: {e}")),
        }
    }

    /// Opens a page (creating it if it doesn't exist) and records it in history.
    pub fn navigate(&mut self, title: &str, ctx: &egui::Context) {
        if self.open(title, ctx) {
            let t = self.current.clone().unwrap();
            if self.history.get(self.hist_pos).is_some_and(|h| key(h) == key(&t)) {
                return;
            }
            if !self.history.is_empty() {
                self.history.truncate(self.hist_pos + 1);
            }
            self.history.push(t);
            self.hist_pos = self.history.len() - 1;
        }
    }

    pub fn back(&mut self, ctx: &egui::Context) {
        if self.hist_pos > 0 {
            self.hist_pos -= 1;
            let t = self.history[self.hist_pos].clone();
            self.open(&t, ctx);
        }
    }

    pub fn forward(&mut self, ctx: &egui::Context) {
        if self.hist_pos + 1 < self.history.len() {
            self.hist_pos += 1;
            let t = self.history[self.hist_pos].clone();
            self.open(&t, ctx);
        }
    }

    fn open(&mut self, title: &str, ctx: &egui::Context) -> bool {
        self.flush();
        if !self.wiki.exists(title) {
            let heading = title.rsplit('/').next().unwrap_or(title).trim();
            if let Err(e) = self.wiki.save(title, &format!("# {heading}\n\n")) {
                self.set_status(format!("Could not create “{title}”: {e}"));
                return false;
            }
            self.reindex();
            self.mode = Mode::Edit;
        }
        let page = self.wiki.get(title).unwrap();
        self.current = Some(page.title.clone());
        self.buffer = page.body.clone();
        self.dirty = false;
        self.config.last_page = self.current.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} — mindwiki", page.title)));
        true
    }

    pub fn create_page(&mut self, title: &str, ctx: &egui::Context) {
        match crate::store::clean_title(title) {
            Some(t) => self.navigate(&t, ctx),
            None => self.set_status("Invalid title (no . prefix, .., [ ] or |)"),
        }
    }

    pub fn rename_current(&mut self, new: &str, ctx: &egui::Context) {
        let Some(old) = self.current.clone() else { return };
        self.flush();
        match self.wiki.rename(&old, new) {
            Ok(n) => {
                self.reindex();
                let new_title = self.wiki.get(new).map(|p| p.title.clone()).unwrap_or_default();
                for h in &mut self.history {
                    if key(h) == key(&old) {
                        *h = new_title.clone();
                    }
                }
                self.open(&new_title, ctx);
                self.set_status(format!("Renamed; updated links in {n} page(s)"));
            }
            Err(e) => self.set_status(format!("Rename failed: {e}")),
        }
    }

    pub fn delete_current(&mut self, ctx: &egui::Context) {
        let Some(title) = self.current.take() else { return };
        self.dirty = false;
        if let Err(e) = self.wiki.delete(&title) {
            self.set_status(format!("Delete failed: {e}"));
        }
        self.reindex();
        self.history.retain(|h| key(h) != key(&title));
        self.hist_pos = self.history.len().saturating_sub(1);
        self.buffer.clear();
        match self.history.last().cloned().or_else(|| self.wiki.pages.values().next().map(|p| p.title.clone())) {
            Some(t) => {
                self.open(&t, ctx);
            }
            None => ctx.send_viewport_cmd(egui::ViewportCommand::Title("mindwiki".into())),
        }
    }

    pub fn apply_settings(&mut self, root: &str, autosave: f32, ctx: &egui::Context) {
        self.flush();
        self.config.autosave_secs = autosave.max(0.0);
        let root = PathBuf::from(shellexpand_home(root.trim()));
        if root != self.config.wiki_root {
            self.config.wiki_root = root;
            self.config.last_page = None;
            let (w, rx) = start_watcher(&self.config.wiki_root, ctx.clone());
            self._watcher = w;
            self.rx = rx;
            self.load_wiki(ctx);
        }
        self.config.save();
    }

    /// Applies changes made to the folder by other programs.
    pub fn poll_watcher(&mut self) {
        let mut changed_current = false;
        let mut any = false;
        while let Ok(path) = self.rx.try_recv() {
            if self.wiki.reload_path(&path) {
                any = true;
                if let (Some(cur), Some(t)) = (&self.current, self.wiki.title_for(&path)) {
                    changed_current |= key(cur) == key(&t);
                }
            }
        }
        if !any {
            return;
        }
        self.reindex();
        if changed_current {
            let cur = self.current.clone().unwrap();
            match self.wiki.get(&cur) {
                Some(p) if !self.dirty => self.buffer = p.body.clone(),
                Some(_) => self.set_status("Page changed on disk; keeping your unsaved edits"),
                None => {
                    // Deleted elsewhere: keep the text on screen; editing or Ctrl+S recreates it.
                    self.set_status("Page was deleted on disk; edit or Ctrl+S to keep it");
                }
            }
        }
    }

    /// Global keyboard shortcuts. Runs before widgets so they can't eat the keys.
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let (new, save, search, toggle, back, fwd) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::K) | i.consume_key(Modifiers::COMMAND, Key::P),
                i.consume_key(Modifiers::COMMAND, Key::E),
                i.consume_key(Modifiers::ALT, Key::ArrowLeft),
                i.consume_key(Modifiers::ALT, Key::ArrowRight),
            )
        });
        if new {
            self.dialog = Dialog::NewPage(String::new());
        }
        if save {
            self.dirty |= self.current.as_ref().is_some_and(|c| !self.wiki.exists(c));
            self.flush();
            if !self.dirty {
                self.set_status("Saved");
            }
        }
        if search {
            self.search_open = true;
            self.search_focus = true;
        }
        if toggle {
            self.mode = if self.mode == Mode::Edit { Mode::Preview } else { Mode::Edit };
        }
        if back {
            self.back(ctx);
        }
        if fwd {
            self.forward(ctx);
        }
    }

    pub fn autosave(&mut self, ctx: &egui::Context) {
        if !self.dirty || self.config.autosave_secs <= 0.0 {
            return;
        }
        let wait = self.config.autosave_secs as f64;
        let idle = ctx.input(|i| i.time) - self.last_edit;
        if idle >= wait {
            self.flush();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait - idle));
        }
    }

    pub fn on_close(&mut self) {
        self.flush();
        self.config.save();
    }
}

fn shellexpand_home(p: &str) -> String {
    match (p.strip_prefix("~"), dirs::home_dir()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", home.display())
        }
        _ => p.to_string(),
    }
}

fn start_watcher(root: &Path, ctx: egui::Context) -> (Option<RecommendedWatcher>, Receiver<PathBuf>) {
    let (tx, rx) = channel();
    let _ = std::fs::create_dir_all(root);
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let _ = tx.send(p);
            }
            ctx.request_repaint();
        }
    });
    match watcher {
        Ok(mut w) => match w.watch(root, RecursiveMode::Recursive) {
            Ok(()) => (Some(w), rx),
            Err(_) => (None, rx),
        },
        Err(_) => (None, rx),
    }
}
