//! Backlinks and tags, rebuilt from the pages whenever something changes.
//! This only walks the already-parsed link/tag sets, so it is cheap even for
//! thousands of pages.

use crate::page::key;
use crate::store::Wiki;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Index {
    /// Target key -> titles of pages linking to it.
    pub backlinks: BTreeMap<String, BTreeSet<String>>,
    /// Tag -> titles of pages with that tag.
    pub tags: BTreeMap<String, BTreeSet<String>>,
}

impl Index {
    pub fn build(wiki: &Wiki) -> Self {
        let mut idx = Index::default();
        for page in wiki.pages.values() {
            for link in &page.links {
                if *link != key(&page.title) {
                    idx.backlinks.entry(link.clone()).or_default().insert(page.title.clone());
                }
            }
            for tag in &page.tags {
                idx.tags.entry(tag.clone()).or_default().insert(page.title.clone());
            }
        }
        idx
    }

    pub fn backlinks_to(&self, title: &str) -> Vec<&String> {
        self.backlinks.get(&key(title)).map(|s| s.iter().collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backlinks_and_tags() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Wiki::open(dir.path()).unwrap();
        w.save("A", "[[B]] [[a]] #x").unwrap();
        w.save("C", "[[b]] #x #y").unwrap();
        let idx = Index::build(&w);
        assert_eq!(idx.backlinks_to("B"), vec!["A", "C"]);
        assert!(idx.backlinks_to("A").is_empty(), "self links are ignored");
        assert_eq!(idx.tags["x"].len(), 2);
        assert_eq!(idx.tags["y"].iter().collect::<Vec<_>>(), vec!["C"]);

        w.save("C", "nothing").unwrap();
        let idx = Index::build(&w);
        assert_eq!(idx.backlinks_to("B"), vec!["A"]);
        assert!(!idx.tags.contains_key("y"));
    }
}
