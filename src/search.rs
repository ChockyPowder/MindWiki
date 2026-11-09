//! Full-text search over the in-memory pages.
//!
//! Every query word must match (in the title, a tag or the body). A word
//! starting with `#` matches tags only. Results are ranked title > tag > body.

use crate::store::Wiki;

pub struct Hit {
    pub title: String,
    pub score: u32,
    pub snippet: String,
}

pub fn search(wiki: &Wiki, query: &str, limit: usize) -> Vec<Hit> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit> = wiki
        .pages
        .values()
        .filter_map(|p| {
            let mut score = 0u32;
            let mut first_body: Option<(usize, usize)> = None;
            for t in &terms {
                if let Some(tag) = t.strip_prefix('#') {
                    if tag.is_empty() {
                        continue;
                    }
                    if p.tags.contains(tag) {
                        score += 20;
                    } else if p.tags.iter().any(|x| x.starts_with(tag)) {
                        score += 10;
                    } else {
                        return None;
                    }
                    continue;
                }
                let mut s = 0;
                if p.title_lc == *t {
                    s += 100;
                } else if p.title_lc.contains(t.as_str()) {
                    s += 30;
                }
                if p.tags.contains(t) {
                    s += 15;
                }
                let n = p.body_lc.matches(t.as_str()).count() as u32;
                if n > 0 {
                    s += n.min(20);
                    if first_body.is_none() {
                        first_body = p.body_lc.find(t.as_str()).map(|i| (i, t.len()));
                    }
                }
                if s == 0 {
                    return None;
                }
                score += s;
            }
            let snippet = match first_body {
                Some((i, len)) => snippet(&p.body, &p.body_lc, i, len),
                None => first_line(&p.body),
            };
            Some(Hit { title: p.title.clone(), score, snippet })
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    hits.truncate(limit);
    hits
}

/// Text around a match. `i` is a byte offset into `lc` (the lowercased body),
/// which can differ in length from the original, so we map by char count.
fn snippet(body: &str, lc: &str, i: usize, len: usize) -> String {
    let start_chars = lc[..i].chars().count();
    let match_chars = lc[i..i + len].chars().count();
    let chars: Vec<char> = body.chars().collect();
    let from = start_chars.saturating_sub(40);
    let to = (start_chars + match_chars + 60).min(chars.len());
    let from = from.min(to);
    let mut s: String = chars[from..to].iter().collect::<String>().replace(['\n', '\r', '\t'], " ");
    if from > 0 {
        s.insert(0, '…');
    }
    if to < chars.len() {
        s.push('…');
    }
    s
}

fn first_line(body: &str) -> String {
    let (_, rest) = crate::page::split_front_matter(body);
    rest.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(100).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiki() -> (tempfile::TempDir, Wiki) {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Wiki::open(dir.path()).unwrap();
        w.save("Rust", "A systems language. #lang").unwrap();
        w.save("Tokio", "Async runtime for rust. rust rust").unwrap();
        w.save("Cooking", "Pasta recipe #food").unwrap();
        w.save("Ünïcode", "ÄÖÜ ẞ long text before the WORD and after").unwrap();
        (dir, w)
    }

    #[test]
    fn ranking_title_first() {
        let (_d, w) = wiki();
        let hits = search(&w, "rust", 10);
        assert_eq!(hits.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), vec!["Rust", "Tokio"]);
    }

    #[test]
    fn all_terms_must_match() {
        let (_d, w) = wiki();
        assert_eq!(search(&w, "async rust", 10).len(), 1);
        assert!(search(&w, "rust pasta", 10).is_empty());
        assert!(search(&w, "   ", 10).is_empty());
    }

    #[test]
    fn tag_queries() {
        let (_d, w) = wiki();
        let hits = search(&w, "#food", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Cooking");
        assert_eq!(search(&w, "#la", 10)[0].title, "Rust");
    }

    #[test]
    fn unicode_snippet_does_not_panic() {
        let (_d, w) = wiki();
        let hits = search(&w, "word", 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("WORD"));
    }
}
