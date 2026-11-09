//! A single wiki page: parsing front matter, `#tags` and `[[links]]`.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;

/// `[[Target]]` or `[[Target|shown text]]`.
pub static LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\[\]|\n]+)(?:\|([^\[\]\n]+))?\]\]").unwrap());

/// `#tag` at the start of a line or after whitespace. Must start with a letter so
/// `# Heading` and `#123` are not tags.
static TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)#([A-Za-z][A-Za-z0-9_\-/]*)").unwrap());

#[derive(Debug, Clone)]
pub struct Page {
    pub title: String,
    pub path: PathBuf,
    pub body: String,
    pub tags: BTreeSet<String>,
    /// Lowercased link targets.
    pub links: BTreeSet<String>,
    /// Lowercased title + body, cached for search.
    pub body_lc: String,
    pub title_lc: String,
}

impl Page {
    pub fn new(title: String, path: PathBuf, body: String) -> Self {
        let mut p = Page {
            title_lc: title.to_lowercase(),
            title,
            path,
            body: String::new(),
            tags: BTreeSet::new(),
            links: BTreeSet::new(),
            body_lc: String::new(),
        };
        p.set_body(body);
        p
    }

    pub fn set_body(&mut self, body: String) {
        let (tags, links) = parse(&body);
        self.tags = tags;
        self.links = links;
        self.body_lc = body.to_lowercase();
        self.body = body;
    }
}

/// Case-insensitive key used for matching titles and links.
pub fn key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// Splits off a leading `---` front matter block. Returns (front matter, rest).
pub fn split_front_matter(body: &str) -> (Option<&str>, &str) {
    let Some(rest) = body.strip_prefix("---\n").or_else(|| body.strip_prefix("---\r\n")) else {
        return (None, body);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, body)
}

/// Calls `f` for every line outside fenced code blocks.
fn for_prose_lines(text: &str, mut f: impl FnMut(&str)) {
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            f(line);
        }
    }
}

/// Removes `inline code` spans from a line.
fn strip_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
        } else if !in_code {
            out.push(c);
        }
    }
    out
}

pub fn parse(body: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut tags = BTreeSet::new();
    let mut links = BTreeSet::new();
    let (fm, rest) = split_front_matter(body);

    if let Some(fm) = fm {
        for line in fm.lines() {
            if let Some(v) = line.trim().strip_prefix("tags:") {
                let v = v.trim().trim_start_matches('[').trim_end_matches(']');
                for t in v.split(',') {
                    let t = t.trim().trim_matches(['"', '\'']).trim_start_matches('#');
                    if !t.is_empty() {
                        tags.insert(t.to_lowercase());
                    }
                }
            }
        }
    }

    for_prose_lines(rest, |line| {
        let line = strip_inline_code(line);
        for c in TAG_RE.captures_iter(&line) {
            tags.insert(c[1].trim_end_matches(['/', '-']).to_lowercase());
        }
        for c in LINK_RE.captures_iter(&line) {
            let k = key(&c[1]);
            if !k.is_empty() {
                links.insert(k);
            }
        }
    });
    (tags, links)
}

/// Rewrites `[[old]]` / `[[old|text]]` links (case-insensitive) to point at `new`.
/// Returns None when nothing changed.
pub fn rewrite_links(body: &str, old: &str, new: &str) -> Option<String> {
    let old_k = key(old);
    let mut changed = false;
    let out = LINK_RE.replace_all(body, |c: &regex::Captures| {
        if key(&c[1]) == old_k {
            changed = true;
            match c.get(2) {
                Some(alias) => format!("[[{new}|{}]]", alias.as_str()),
                None => format!("[[{new}]]"),
            }
        } else {
            c[0].to_string()
        }
    });
    changed.then(|| out.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn links_and_aliases() {
        let (_, links) = parse("See [[Rust]] and [[Tokio Runtime|tokio]]. [[ rust ]]");
        assert_eq!(links, set(&["rust", "tokio runtime"]));
    }

    #[test]
    fn front_matter_tags() {
        let body = "---\ntags: [Rust, \"net\", #async]\n---\n# Title\nbody #extra\n";
        let (tags, _) = parse(body);
        assert_eq!(tags, set(&["rust", "net", "async", "extra"]));
        let (fm, rest) = split_front_matter(body);
        assert!(fm.unwrap().contains("tags"));
        assert!(rest.starts_with("# Title"));
    }

    #[test]
    fn headings_and_code_are_not_tags_or_links() {
        let body = "# Heading\n## Sub\n`#notatag [[nolink]]`\n```\n#nope [[nope]]\n```\n#yes [[Yes]]";
        let (tags, links) = parse(body);
        assert_eq!(tags, set(&["yes"]));
        assert_eq!(links, set(&["yes"]));
    }

    #[test]
    fn unterminated_front_matter_is_body() {
        let (fm, rest) = split_front_matter("---\nno end");
        assert!(fm.is_none());
        assert_eq!(rest, "---\nno end");
    }

    #[test]
    fn rewrite() {
        let out = rewrite_links("a [[Old]] b [[old|alias]] c [[Other]]", "OLD", "New").unwrap();
        assert_eq!(out, "a [[New]] b [[New|alias]] c [[Other]]");
        assert!(rewrite_links("nothing", "Old", "New").is_none());
    }
}
