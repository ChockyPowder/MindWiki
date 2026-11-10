//! Left panel: page list with filter, tag list, new page button.

use crate::app::{Dialog, WikiApp};
use egui::{Panel, RichText};

impl WikiApp {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut go = None;
        // Slides open and closed (☰ or Ctrl+B).
        let mut open = self.sidebar_open;
        let panel = Panel::left("sidebar").resizable(true).default_size(240.0).show_collapsible(ui, &mut open, |ui| {
            ui.add_space(4.0);
            let new_btn = egui::Button::new(RichText::new("＋ New page").strong())
                .fill(super::theme::accent(ui).gamma_multiply(0.25))
                .min_size(egui::vec2(ui.available_width(), 30.0));
            if ui.add(new_btn).on_hover_text("Ctrl+N").clicked() {
                self.dialog = Dialog::NewPage(String::new());
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter pages…")
                    .desired_width(f32::INFINITY),
            );
            if let Some(tag) = self.tag_filter.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("Tag: #{tag}"));
                    if ui.small_button("✕").clicked() {
                        self.tag_filter = None;
                    }
                });
            }
            ui.separator();

            let f = self.filter.to_lowercase();
            let tagged = self.tag_filter.as_ref().and_then(|t| self.index.tags.get(t));
            let cur = self.current.as_deref().map(crate::page::key);

            // Tags take up to ~40% of the height; pages get the rest.
            let tags_h = (ui.available_height() * 0.4).min(24.0 * (self.index.tags.len() as f32 + 1.5));
            let pages_h = (ui.available_height() - tags_h - 30.0).max(80.0);

            ui.label(RichText::new(format!("Pages ({})", self.wiki.pages.len())).strong());
            egui::ScrollArea::vertical().id_salt("pages").max_height(pages_h).auto_shrink([false, true]).show(
                ui,
                |ui| {
                    // Full-width rows with left-aligned text.
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    for (k, p) in &self.wiki.pages {
                        if !f.is_empty() && !p.title_lc.contains(&f) {
                            continue;
                        }
                        if tagged.is_some_and(|set| !set.contains(&p.title)) {
                            continue;
                        }
                        if ui.selectable_label(cur.as_ref() == Some(k), p.title.as_str()).clicked() {
                            go = Some(p.title.clone());
                        }
                    }
                    });
                },
            );

            ui.separator();
            ui.label(RichText::new("Tags").strong());
            egui::ScrollArea::vertical().id_salt("tags").auto_shrink([false, true]).show(ui, |ui| {
                if self.index.tags.is_empty() {
                    ui.label(RichText::new("Add #tags to pages").weak());
                }
                ui.horizontal_wrapped(|ui| {
                    for (tag, pages) in &self.index.tags {
                        let on = self.tag_filter.as_deref() == Some(tag.as_str());
                        if ui.selectable_label(on, format!("#{tag} {}", pages.len())).clicked() {
                            self.tag_filter = if on { None } else { Some(tag.clone()) };
                        }
                    }
                });
            });
        });
        self.sidebar_open = open;
        self.rects.sidebar = panel.map(|p| p.response.rect);
        if let Some(t) = go {
            let ctx = ui.ctx().clone();
            self.navigate(&t, &ctx);
        }
    }
}
