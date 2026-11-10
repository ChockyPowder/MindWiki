//! Drawing: top bar, editor/preview, backlinks panel and dialogs.

pub mod preview;
mod search;
mod sidebar;
pub mod theme;
mod tour;

use crate::app::{Dialog, Mode, WikiApp};
use egui::{CentralPanel, Panel, RichText};

impl eframe::App for WikiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_watcher();
        self.shortcuts(&ctx);

        self.top_bar(ui);
        self.sidebar(ui);
        self.backlinks_panel(ui);
        CentralPanel::default_margins().show(ui, |ui| self.central(ui));
        self.search_window(&ctx);
        self.dialogs(&ctx);
        self.tour_window(&ctx);

        self.autosave(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.on_close();
        }
    }
}

impl WikiApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let frame = egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(10, 7));
        Panel::top("top").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("☰").on_hover_text("Show or hide the sidebar (Ctrl+B)").clicked() {
                    self.sidebar_open = !self.sidebar_open;
                }
                if ui.add_enabled(self.hist_pos > 0, egui::Button::new("◀")).on_hover_text("Back (Alt+←)").clicked() {
                    self.back(&ctx);
                }
                let can_fwd = self.hist_pos + 1 < self.history.len();
                if ui.add_enabled(can_fwd, egui::Button::new("▶")).on_hover_text("Forward (Alt+→)").clicked() {
                    self.forward(&ctx);
                }
                ui.add_space(6.0);
                if let Some(t) = &self.current {
                    // Folder part is dimmed, page name is strong.
                    if let Some((folder, name)) = t.rsplit_once('/') {
                        ui.label(RichText::new(format!("{folder} /")).weak().size(16.0));
                        ui.label(RichText::new(name).strong().size(18.0));
                    } else {
                        ui.label(RichText::new(t).strong().size(18.0));
                    }
                    // Unsaved-changes dot fades in and out.
                    let a = ctx.animate_bool_with_time(egui::Id::unique("dirty-dot"), self.dirty, 0.25);
                    if a > 0.0 {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 3.5 * a, theme::accent(ui).gamma_multiply(a));
                        resp.on_hover_text("Unsaved changes");
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("?").on_hover_text("Show the intro tour").clicked() {
                        self.start_tour();
                    }
                    let dark = ui.visuals().dark_mode;
                    if ui.button(if dark { "☀" } else { "🌙" }).on_hover_text("Switch light / dark theme").clicked() {
                        self.config.dark_mode = Some(!dark);
                        theme::apply(&ctx, self.config.dark_mode);
                        self.config.save();
                    }
                    if ui.button("⚙").on_hover_text("Settings").clicked() {
                        self.dialog = Dialog::Settings {
                            root: self.config.wiki_root.display().to_string(),
                            autosave: self.config.autosave_secs,
                        };
                    }
                    let search = ui.button("🔍 Search").on_hover_text("Ctrl+K");
                    self.rects.search = Some(search.rect);
                    if search.clicked() {
                        self.search_open = true;
                        self.search_focus = true;
                    }
                    ui.separator();
                    if self.current.is_some() {
                        if ui.button("🗑").on_hover_text("Delete page").clicked() {
                            self.dialog = Dialog::Delete;
                        }
                        if ui.button("✏ Rename").clicked() {
                            self.dialog = Dialog::Rename(self.current.clone().unwrap_or_default());
                        }
                        ui.separator();
                        let mut modes = egui::Rect::NOTHING;
                        for (m, label) in [(Mode::Preview, "Preview"), (Mode::Split, "Split"), (Mode::Edit, "Edit")] {
                            let r = ui.selectable_value(&mut self.mode, m, label);
                            if r.clicked() {
                                self.view_changed_at = ctx.input(|i| i.time);
                            }
                            modes = modes.union(r.rect);
                        }
                        self.rects.modes = Some(modes);
                    }
                    // Status message: fades in, stays, fades out.
                    let now = ctx.input(|i| i.time);
                    if let Some((msg, at)) = &mut self.status {
                        if at.is_nan() {
                            *at = now;
                        }
                        let age = (now - *at) as f32;
                        if age < 4.0 {
                            let a = (age / 0.2).min(1.0).min((4.0 - age) / 0.6);
                            let col = ui.visuals().text_color().gamma_multiply(0.75 * a.clamp(0.0, 1.0));
                            ui.label(RichText::new(msg.as_str()).color(col));
                            ctx.request_repaint();
                        } else {
                            self.status = None;
                        }
                    }
                });
            });
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        self.rects.central = Some(ui.max_rect());
        if self.current.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No page open.\nPress Ctrl+N to create one.").weak().size(17.0))
            });
            return;
        }
        // New pages and view switches fade in and slide up a little.
        let p = theme::progress(ui.ctx(), self.view_changed_at, 0.22);
        ui.set_opacity(p);
        ui.add_space((1.0 - p) * 14.0);

        let now = ui.ctx().input(|i| i.time);
        let mut clicked = None;
        match self.mode {
            Mode::Edit => {
                if editor(ui, &mut self.buffer) {
                    self.dirty = true;
                    self.last_edit = now;
                }
            }
            Mode::Preview => {
                clicked = preview::show(ui, &mut self.md_cache, &self.wiki, &self.buffer);
            }
            Mode::Split => {
                ui.columns(2, |cols| {
                    if editor(&mut cols[0], &mut self.buffer) {
                        self.dirty = true;
                        self.last_edit = now;
                    }
                    clicked = preview::show(&mut cols[1], &mut self.md_cache, &self.wiki, &self.buffer);
                });
            }
        }
        if let Some(t) = clicked {
            let ctx = ui.ctx().clone();
            self.navigate(&t, &ctx);
        }
    }

    fn backlinks_panel(&mut self, ui: &mut egui::Ui) {
        let Some(cur) = self.current.clone() else { return };
        let links: Vec<String> = self.index.backlinks_to(&cur).into_iter().cloned().collect();
        let tags: Vec<String> = self.wiki.get(&cur).map(|p| p.tags.iter().cloned().collect()).unwrap_or_default();
        let mut go = None;
        let panel = Panel::bottom("backlinks").resizable(true).default_size(80.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Backlinks ({})", links.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if links.is_empty() {
                                ui.label(RichText::new("No pages link here yet.").weak());
                            }
                            for l in &links {
                                if ui.link(l).clicked() {
                                    go = Some(l.clone());
                                }
                            }
                        });
                    });
                if !tags.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Tags:");
                        for t in &tags {
                            let on = self.tag_filter.as_deref() == Some(t.as_str());
                            if ui.selectable_label(on, format!("#{t}")).on_hover_text("Filter sidebar by tag").clicked() {
                                self.tag_filter = if on { None } else { Some(t.clone()) };
                            }
                        }
                    });
                }
            });
        });
        self.rects.backlinks = Some(panel.response.rect);
        if let Some(t) = go {
            let ctx = ui.ctx().clone();
            self.navigate(&t, &ctx);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if matches!(self.dialog, Dialog::None) {
            return;
        }
        let mut close = false;
        let mut action: Option<Box<dyn FnOnce(&mut WikiApp, &egui::Context)>> = None;
        let is_new = matches!(self.dialog, Dialog::NewPage(_));
        let modal = egui::Modal::new(egui::Id::unique("mindwiki-dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            match &mut self.dialog {
                Dialog::None => {}
                Dialog::NewPage(title) | Dialog::Rename(title) => {
                    ui.heading(if is_new { "New page" } else { "Rename page" });
                    ui.label(RichText::new("Use / for folders, e.g. notes/Rust").weak());
                    let r = ui.text_edit_singleline(title);
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let ok = ui.button(if is_new { "Create" } else { "Rename" }).clicked();
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if (ok || enter) && !title.trim().is_empty() {
                            let t = title.clone();
                            action = Some(if is_new {
                                Box::new(move |a, c| a.create_page(&t, c))
                            } else {
                                Box::new(move |a, c| a.rename_current(&t, c))
                            });
                        }
                    });
                }
                Dialog::Delete => {
                    ui.heading("Delete page?");
                    ui.label(format!("“{}” will be removed from disk.", self.current.as_deref().unwrap_or("")));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Delete").color(ui.visuals().error_fg_color)).clicked() {
                            action = Some(Box::new(|a, c| a.delete_current(c)));
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Settings { root, autosave } => {
                    ui.heading("Settings");
                    ui.label("Wiki folder:");
                    ui.text_edit_singleline(root);
                    ui.add(egui::Slider::new(autosave, 0.0..=30.0).text("autosave delay (s, 0 = off)"));
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            let (r, s) = (root.clone(), *autosave);
                            action = Some(Box::new(move |a, c| a.apply_settings(&r, s, c)));
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(f) = action {
            self.dialog = Dialog::None;
            f(self, ctx);
        } else if close {
            self.dialog = Dialog::None;
        }
    }
}

/// The Markdown source editor. Returns true if the text changed.
fn editor(ui: &mut egui::Ui, text: &mut String) -> bool {
    egui::ScrollArea::vertical()
        .id_salt("editor")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(text)
                    .code_editor()
                    .lock_focus(true)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::symmetric(14, 10))
                    .desired_width(f32::INFINITY),
            )
            .changed()
        })
        .inner
}
