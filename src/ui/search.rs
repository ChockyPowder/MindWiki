//! Ctrl+K search popup with live results and keyboard selection.

use crate::app::WikiApp;
use egui::{Key, Modifiers, RichText};

const LIMIT: usize = 50;

impl WikiApp {
    pub(super) fn search_window(&mut self, ctx: &egui::Context) {
        if !self.search_open {
            return;
        }
        let hits = crate::search::search(&self.wiki, &self.query, LIMIT);
        // Offer "create page" as the last row when the query isn't an existing title.
        let can_create = !self.query.trim().is_empty() && !self.wiki.exists(self.query.trim());
        let rows = hits.len() + can_create as usize;

        let (down, up, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down && rows > 0 {
            self.search_sel = (self.search_sel + 1).min(rows - 1);
        }
        if up {
            self.search_sel = self.search_sel.saturating_sub(1);
        }
        self.search_sel = self.search_sel.min(rows.saturating_sub(1));

        let mut pick: Option<usize> = enter.then_some(self.search_sel).filter(|_| rows > 0);
        let mut open = true;
        egui::Window::new("Search")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
            .default_width(520.0)
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search pages…  (#tag for tags)")
                        .desired_width(f32::INFINITY),
                );
                if self.search_focus {
                    r.request_focus();
                    self.search_focus = false;
                }
                if r.changed() {
                    self.search_sel = 0;
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for (i, h) in hits.iter().enumerate() {
                        let sel = i == self.search_sel;
                        let resp = ui.selectable_label(sel, RichText::new(&h.title).strong());
                        if !h.snippet.is_empty() {
                            ui.label(RichText::new(&h.snippet).weak().small());
                        }
                        if sel && (down || up) {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            pick = Some(i);
                        }
                        ui.add_space(2.0);
                    }
                    if can_create {
                        let i = hits.len();
                        let label = format!("＋ Create page “{}”", self.query.trim());
                        if ui.selectable_label(i == self.search_sel, label).clicked() {
                            pick = Some(i);
                        }
                    } else if hits.is_empty() && !self.query.trim().is_empty() {
                        ui.label(RichText::new("No matches").weak());
                    }
                });
            });

        if let Some(i) = pick {
            let title = match hits.get(i) {
                Some(h) => h.title.clone(),
                None => self.query.trim().to_string(),
            };
            self.close_search();
            if hits.get(i).is_some() {
                self.navigate(&title, ctx);
            } else {
                self.create_page(&title, ctx);
            }
        } else if esc || !open {
            self.close_search();
        }
    }

    fn close_search(&mut self) {
        self.search_open = false;
        self.query.clear();
        self.search_sel = 0;
    }
}
