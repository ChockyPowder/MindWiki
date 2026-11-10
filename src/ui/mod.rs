//! Drawing: top bar, editor/preview, backlinks panel and dialogs.

pub mod preview;
mod search;
mod sidebar;
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
        Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.hist_pos > 0, egui::Button::new("◀")).on_hover_text("Back (Alt+←)").clicked() {
                    self.back(&ctx);
                }
                let can_fwd = self.hist_pos + 1 < self.history.len();
                if ui.add_enabled(can_fwd, egui::Button::new("▶")).on_hover_text("Forward (Alt+→)").clicked() {
                    self.forward(&ctx);
                }
                ui.separator();
                if let Some(t) = &self.current {
                    let mark = if self.dirty { " •" } else { "" };
                    ui.heading(format!("{t}{mark}"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("?").on_hover_text("Show the intro tour").clicked() {
                        self.start_tour();
                    }
                    if ui.button("⚙").on_hover_text("Settings").clicked() {
                        self.dialog = Dialog::Settings {
                            root: self.config.wiki_root.display().to_string(),
                            autosave: self.config.autosave_secs,
                        };
                    }
                    if ui.button("🔍 Search").on_hover_text("Ctrl+K").clicked() {
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
                        for (m, label) in [(Mode::Preview, "Preview"), (Mode::Split, "Split"), (Mode::Edit, "Edit")] {
                            ui.selectable_value(&mut self.mode, m, label);
                        }
                    }
                    let now = ctx.input(|i| i.time);
                    if let Some((msg, at)) = &mut self.status {
                        if at.is_nan() {
                            *at = now;
                        }
                        if now - *at < 5.0 {
                            ui.label(RichText::new(msg.as_str()).weak());
                            ctx.request_repaint_after(std::time::Duration::from_millis(500));
                        } else {
                            self.status = None;
                        }
                    }
                });
            });
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        if self.current.is_none() {
            ui.centered_and_justified(|ui| ui.label("No page open. Press Ctrl+N to create one."));
            return;
        }
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
        Panel::bottom("backlinks").resizable(true).default_size(80.0).show(ui, |ui| {
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
                egui::TextEdit::multiline(text).code_editor().lock_focus(true).desired_width(f32::INFINITY),
            )
            .changed()
        })
        .inner
}
