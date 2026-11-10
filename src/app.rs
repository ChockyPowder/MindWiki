//! App state and the non-drawing logic: opening/saving pages, history,
//! the file watcher, shortcuts and autosave.

use crate::config::Config;
use crate::index::Index;
use crate::page::key;
use crate::remote::{self, Remote};
use crate::sync::{self, Status, SyncHandle};
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

#[derive(Default)]
pub struct Rects {
    pub sidebar: Option<egui::Rect>,
    pub modes: Option<egui::Rect>,
    pub search: Option<egui::Rect>,
    pub central: Option<egui::Rect>,
    pub backlinks: Option<egui::Rect>,
}

pub enum Dialog {
    None,
    NewPage(String),
    Rename(String),
    Delete,
    Settings { root: String, autosave: f32 },
    /// Log in to a server.
    Connect { url: String, user: String, pass: String, fingerprint: String, error: Option<String> },
    /// Details of the current server connection.
    Remote,
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
    /// When the tour step last changed (for its fade).
    pub tour_changed_at: f64,
    /// When the page or view mode last changed (for the fade-in).
    pub view_changed_at: f64,
    pub sidebar_open: bool,
    /// Where parts of the UI were drawn last frame; the tour highlights them.
    pub rects: Rects,

    /// Background sync with the server, when connected.
    sync: Option<SyncHandle>,
    pub sync_status: Option<Status>,
    /// A login attempt running in the background.
    connect_rx: Option<Receiver<Result<Remote, String>>>,

    _watcher: Option<RecommendedWatcher>,
    rx: Receiver<PathBuf>,
}

impl WikiApp {
    pub fn new(ctx: &egui::Context, config: Config) -> Self {
        crate::ui::theme::apply(ctx, config.dark_mode);
        let root = config.active_root();
        let (watcher, rx) = start_watcher(&root, ctx.clone());
        let mut app = WikiApp {
            wiki: Wiki { root, pages: BTreeMap::new() },
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
            tour_changed_at: 0.0,
            view_changed_at: 0.0,
            sidebar_open: true,
            rects: Rects::default(),
            md_cache: CommonMarkCache::default(),
            dialog: Dialog::None,
            status: None,
            sync: None,
            sync_status: None,
            connect_rx: None,
            _watcher: watcher,
            rx,
        };
        app.load_wiki(ctx);
        app.start_sync(ctx);
        if !app.config.intro_done {
            app.start_tour();
        }
        app
    }

    /// (Re)opens the active wiki folder and opens a start page.
    fn load_wiki(&mut self, ctx: &egui::Context) {
        let root = self.config.active_root();
        match Wiki::open(&root) {
            Ok(mut w) => {
                // A server's wiki gets its pages from the server instead.
                if self.config.remote.is_none()
                    && let Err(e) = w.seed_if_empty()
                {
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
        self.buffer.clear();
        self.dirty = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("mindwiki".into()));
        self.open_start_page(ctx);
    }

    fn open_start_page(&mut self, ctx: &egui::Context) {
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

    /// Switches to the folder `config.active_root()` now points at.
    fn switch_root(&mut self, ctx: &egui::Context) {
        self.sync = None;
        self.sync_status = None;
        self.config.last_page = None;
        let (w, rx) = start_watcher(&self.config.active_root(), ctx.clone());
        self._watcher = w;
        self.rx = rx;
        self.load_wiki(ctx);
        self.start_sync(ctx);
    }

    // ---------- server connection ----------

    pub fn connecting(&self) -> bool {
        self.connect_rx.is_some()
    }

    fn start_sync(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.config.remote else { return };
        match remote::Client::new(r) {
            Ok(client) => {
                let c = ctx.clone();
                self.sync = Some(sync::spawn(self.config.active_root(), client, move || c.request_repaint()));
                self.sync_status = Some(Status::Syncing);
            }
            Err(e) => self.sync_status = Some(Status::Offline(e)),
        }
    }

    pub fn sync_now(&mut self) {
        if let Some(s) = &self.sync {
            s.sync_now();
        }
    }

    /// Logs in on a background thread so the window doesn't freeze.
    pub fn start_connect(&mut self, url: String, user: String, pass: String, fingerprint: String, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(remote::connect(&url, &user, &pass, &fingerprint));
            ctx.request_repaint();
        });
        self.connect_rx = Some(rx);
    }

    pub fn cancel_connect(&mut self) {
        self.connect_rx = None;
    }

    pub fn poll_remote(&mut self, ctx: &egui::Context) {
        // Result of a login attempt.
        if let Some(rx) = &self.connect_rx
            && let Ok(result) = rx.try_recv()
        {
            self.connect_rx = None;
            match result {
                Ok(r) => {
                    self.flush();
                    self.set_status(format!("Connected as {}", r.username));
                    self.config.remote = Some(r);
                    self.dialog = Dialog::None;
                    self.switch_root(ctx);
                    self.config.save();
                }
                Err(e) => {
                    if let Dialog::Connect { error, pass, .. } = &mut self.dialog {
                        *error = Some(e);
                        pass.clear();
                    }
                }
            }
        }
        // News from the sync thread.
        let mut latest = None;
        if let Some(s) = &self.sync {
            while let Ok(st) = s.status.try_recv() {
                latest = Some(st);
            }
        }
        if let Some(st) = latest {
            match &st {
                Status::Synced { conflicts, .. } if !conflicts.is_empty() => {
                    self.set_status(format!("Edited in two places; your version was kept as “{}”", conflicts[0]));
                }
                Status::LoggedOut => {
                    self.sync = None;
                    if let Some(r) = &self.config.remote {
                        self.dialog = Dialog::Connect {
                            url: r.url.clone(),
                            user: r.username.clone(),
                            pass: String::new(),
                            fingerprint: String::new(),
                            error: Some("Your login has expired. Please log in again.".into()),
                        };
                    }
                }
                _ => {}
            }
            self.sync_status = Some(st);
        }
    }

    /// Goes back to the local wiki. The downloaded copy stays on disk.
    pub fn disconnect(&mut self, ctx: &egui::Context) {
        self.flush();
        self.sync = None;
        if let Some(r) = self.config.remote.take() {
            // Tell the server to forget the token; don't wait for it.
            std::thread::spawn(move || {
                if let Ok(c) = remote::Client::new(&r) {
                    c.logout();
                }
            });
        }
        self.switch_root(ctx);
        self.config.save();
        self.set_status("Disconnected; showing your local wiki");
    }

    /// Call after any change to the pages: rebuilds backlinks and tags, and
    /// asks the server sync (if connected) to run.
    pub fn reindex(&mut self) {
        self.index = Index::build(&self.wiki);
        self.sync_now();
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
        self.view_changed_at = ctx.input(|i| i.time);
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
            // While connected to a server the local folder isn't on screen.
            if self.config.remote.is_none() {
                self.switch_root(ctx);
            }
        }
        self.config.save();
    }

    /// Applies changes made to the folder by other programs.
    pub fn poll_watcher(&mut self, ctx: &egui::Context) {
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
        if self.current.is_none() {
            // First pages arriving from a server.
            self.open_start_page(ctx);
            return;
        }
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
        let (new, save, search, toggle, back, fwd, side) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::K) | i.consume_key(Modifiers::COMMAND, Key::P),
                i.consume_key(Modifiers::COMMAND, Key::E),
                i.consume_key(Modifiers::ALT, Key::ArrowLeft),
                i.consume_key(Modifiers::ALT, Key::ArrowRight),
                i.consume_key(Modifiers::COMMAND, Key::B),
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
        if side {
            self.sidebar_open = !self.sidebar_open;
        }
        if toggle {
            self.view_changed_at = ctx.input(|i| i.time);
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
