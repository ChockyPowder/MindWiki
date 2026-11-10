//! First-run walkthrough: a small window that moves next to each part of the
//! app and explains it. Shown until finished or skipped; "?" in the top bar
//! replays it.

use super::theme;
use crate::app::WikiApp;
use egui::{Align2, RichText};

/// The part of the app a step points at (it gets a glowing outline).
#[derive(Clone, Copy)]
enum Target {
    None,
    Sidebar,
    Modes,
    Central,
    Backlinks,
    Search,
}

struct Step {
    target: Target,
    title: &'static str,
    text: &'static str,
    /// Where the window sits, so it is next to the thing it describes.
    anchor: Align2,
    offset: [f32; 2],
}

const STEPS: &[Step] = &[
    Step {
        target: Target::None,
        title: "Welcome to mindwiki",
        text: "This is your personal wiki: a place to write down things you know and link them together.\n\n\
               Every page is a plain Markdown file in your wiki folder, so your notes are never locked in.\n\n\
               This short tour shows the main parts. It takes about a minute.",
        anchor: Align2::CENTER_CENTER,
        offset: [0.0, 0.0],
    },
    Step {
        target: Target::Sidebar,
        title: "Pages",
        text: "The sidebar on the left lists all your pages. Click one to open it.\n\n\
               Use “＋ New page” (or Ctrl+N) to make a page. A name like notes/Rust puts it in a folder.\n\n\
               The filter box narrows the list as you type.",
        anchor: Align2::LEFT_TOP,
        offset: [250.0, 70.0],
    },
    Step {
        target: Target::Modes,
        title: "Writing and reading",
        text: "The buttons at the top right switch how you see a page:\n\n\
               • Edit: write Markdown\n• Preview: read the formatted page\n• Split: both side by side\n\n\
               Ctrl+E flips between Edit and Preview. Pages save by themselves a moment after you stop typing.",
        anchor: Align2::RIGHT_TOP,
        offset: [-20.0, 50.0],
    },
    Step {
        target: Target::Central,
        title: "Linking pages",
        text: "Put a page name in double square brackets to link to it:\n\n\
               [[Page Name]]\n[[Page Name|text to show]]\n\n\
               In Preview, click a link to go there. If the page doesn't exist yet it is marked (new), \
               and clicking it creates it. That is the quickest way to grow your wiki.",
        anchor: Align2::CENTER_CENTER,
        offset: [0.0, 0.0],
    },
    Step {
        target: Target::Backlinks,
        title: "Backlinks and tags",
        text: "The panel at the bottom shows Backlinks: every page that links to the one you are on.\n\n\
               Write #tag anywhere in a page to tag it. Tags appear in the sidebar; \
               click one to show only pages with that tag.",
        anchor: Align2::CENTER_BOTTOM,
        offset: [0.0, -110.0],
    },
    Step {
        target: Target::Search,
        title: "Search",
        text: "Press Ctrl+K (or click 🔍 Search) to search every page.\n\n\
               Type words to find them anywhere, or #tag to search tags. \
               Use ↑ ↓ and Enter to open a result.",
        anchor: Align2::CENTER_TOP,
        offset: [0.0, 60.0],
    },
    Step {
        target: Target::None,
        title: "You're ready",
        text: "A few more things:\n\n\
               • Alt+← and Alt+→ go back and forward\n\
               • ✏ Rename also fixes links on other pages\n\
               • ⚙ changes your wiki folder\n\
               • ? in the top bar shows this tour again\n\n\
               Start by editing the Welcome page, or make a new one.",
        anchor: Align2::CENTER_CENTER,
        offset: [0.0, 0.0],
    },
];

impl WikiApp {
    pub fn start_tour(&mut self) {
        self.tour = Some(0);
        self.tour_changed_at = f64::NAN;
        self.sidebar_open = true;
    }

    /// Draws a softly pulsing outline around the part a step talks about.
    fn tour_highlight(&self, ctx: &egui::Context, target: Target, fade: f32) {
        let rect = match target {
            Target::None => None,
            Target::Sidebar => self.rects.sidebar,
            Target::Modes => self.rects.modes,
            Target::Central => self.rects.central,
            Target::Backlinks => self.rects.backlinks,
            Target::Search => self.rects.search,
        };
        let Some(rect) = rect else { return };
        let dark = ctx.global_style().visuals.dark_mode;
        let accent = if dark { theme::ACCENT_DARK } else { theme::ACCENT_LIGHT };
        let pulse = (ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
        let small = rect.height() < 60.0;
        let rect = if small { rect.expand(4.0 + pulse * 2.0) } else { rect.shrink(3.0) };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("tour-highlight")));
        painter.rect_filled(rect, 8.0, accent.gamma_multiply(0.07 * fade));
        painter.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(2.0 + pulse, accent.gamma_multiply((0.55 + 0.45 * pulse) * fade)),
            egui::StrokeKind::Outside,
        );
        ctx.request_repaint();
    }

    fn end_tour(&mut self) {
        self.tour = None;
        if !self.config.intro_done {
            self.config.intro_done = true;
            self.config.save();
        }
    }

    pub(super) fn tour_window(&mut self, ctx: &egui::Context) {
        let Some(i) = self.tour else { return };
        let i = i.min(STEPS.len() - 1);
        let step = &STEPS[i];
        let last = i + 1 == STEPS.len();
        let mut next = None;
        let mut done = false;
        if self.tour_changed_at.is_nan() {
            self.tour_changed_at = ctx.input(|i| i.time);
        }
        // Each step fades in and its outline fades with it.
        let fade = theme::progress(ctx, self.tour_changed_at, 0.3);
        self.tour_highlight(ctx, step.target, fade);

        egui::Window::new("tour")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(step.anchor, step.offset)
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.set_opacity(fade);
                ui.horizontal(|ui| {
                    ui.heading(step.title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{} / {}", i + 1, STEPS.len())).weak());
                    });
                });
                // Progress bar that glides between steps.
                let frac = ctx.animate_value_with_time(
                    egui::Id::unique("tour-progress"),
                    (i + 1) as f32 / STEPS.len() as f32,
                    0.3,
                );
                let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, 2.0, ui.visuals().faint_bg_color);
                let mut filled = bar;
                filled.set_width(bar.width() * frac);
                ui.painter().rect_filled(filled, 2.0, theme::accent(ui));
                ui.add_space(6.0);
                ui.label(step.text);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if !last && ui.button("Skip tour").clicked() {
                        done = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(if last { "Finish" } else { "Next ▶" }).clicked() {
                            if last {
                                done = true;
                            } else {
                                next = Some(i + 1);
                            }
                        }
                        if i > 0 && ui.button("◀ Back").clicked() {
                            next = Some(i - 1);
                        }
                    });
                });
            });

        if done {
            self.end_tour();
        } else if let Some(n) = next {
            self.tour = Some(n);
            self.tour_changed_at = f64::NAN;
        }
    }
}
