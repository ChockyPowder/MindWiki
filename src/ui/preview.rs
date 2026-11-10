//! Rendered Markdown view. `[[links]]` are rewritten into normal Markdown links
//! with a `wiki:` destination, registered as egui_commonmark link hooks, and
//! checked after rendering to see which (if any) was clicked.

use crate::page::{key, LINK_RE};
use crate::store::Wiki;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

pub const SCHEME: &str = "wiki:";

/// Returns the rewritten Markdown plus every `wiki:` destination in it.
pub fn rewrite(body: &str, wiki: &Wiki) -> (String, Vec<String>) {
    let mut dests = Vec::new();
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for line in body.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if in_fence || t.starts_with("```") || t.starts_with("~~~") {
            out.push_str(line);
            continue;
        }
        // Odd segments between backticks are inline code: leave them alone.
        for (i, seg) in line.split('`').enumerate() {
            if i > 0 {
                out.push('`');
            }
            if i % 2 == 1 {
                out.push_str(seg);
                continue;
            }
            let replaced = LINK_RE.replace_all(seg, |c: &regex::Captures| {
                let target = c[1].trim();
                let text = c.get(2).map(|m| m.as_str().trim()).unwrap_or(target);
                let dest = format!("{SCHEME}{}", target.replace(['<', '>'], ""));
                dests.push(dest.clone());
                let marker = if wiki.exists(target) { "" } else { " (new)" };
                format!("[{}{marker}](<{dest}>)", escape(text))
            });
            out.push_str(&replaced);
        }
    }
    (out, dests)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('*', "\\*").replace('_', "\\_")
}

/// Renders the page body and returns the title of a clicked wiki link.
pub fn show(ui: &mut egui::Ui, cache: &mut CommonMarkCache, wiki: &Wiki, body: &str) -> Option<String> {
    let (_, rest) = crate::page::split_front_matter(body);
    let (md, dests) = rewrite(rest, wiki);
    for d in &dests {
        cache.add_link_hook(d.clone());
    }
    egui::ScrollArea::vertical().id_salt("preview").auto_shrink([false, false]).show(ui, |ui| {
        // Keep lines a comfortable reading width, centred in wide windows.
        let avail = ui.available_width();
        let col = (avail - 32.0).clamp(100.0, 780.0);
        ui.horizontal_top(|ui| {
            ui.add_space(((avail - col) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.set_max_width(col);
                ui.add_space(10.0);
                CommonMarkViewer::new().show(ui, cache, &md);
                ui.add_space(24.0);
            });
        });
    });
    let clicked = dests
        .iter()
        .find(|d| cache.get_link_hook(d) == Some(true))
        .map(|d| d[SCHEME.len()..].to_string());
    for d in &dests {
        cache.remove_link_hook(d);
    }
    clicked.filter(|t| !key(t).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_links_but_not_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Wiki::open(dir.path()).unwrap();
        w.save("Rust", "").unwrap();
        let (md, dests) = rewrite("see [[rust]] and [[Go Lang|go_lang]] `[[Inline]]`\n```\n[[Code]]\n```\n", &w);
        assert!(md.contains("[rust](<wiki:rust>)"));
        assert!(md.contains("[go\\_lang (new)](<wiki:Go Lang>)"));
        assert!(md.contains("[[Code]]"));
        assert!(md.contains("`[[Inline]]`"));
        assert_eq!(dests, vec!["wiki:rust", "wiki:Go Lang"]);
    }
}
