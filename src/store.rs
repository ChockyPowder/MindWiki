//! The wiki folder on disk: loading, saving, renaming and deleting pages.

use crate::page::{key, rewrite_links, Page};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

pub struct Wiki {
    pub root: PathBuf,
    /// Keyed by lowercased title.
    pub pages: BTreeMap<String, Page>,
}

pub const WELCOME: &str = "---
tags: [help]
---
# Welcome to mindwiki

This is your personal wiki. Every page is a Markdown file in your wiki folder.

- Link to another page with `[[Page Name]]` or `[[Page Name|shown text]]`. Links to
  pages that don't exist yet are marked *(new)*; click one to create it.
- Tag a page with `#tag` in the text, or in front matter at the top:
  `tags: [rust, ideas]`
- **Ctrl+K** search · **Ctrl+N** new page · **Ctrl+E** edit/preview · **Ctrl+S** save
- **Alt+←/→** back and forward

Try it: [[My First Note]] #help
";

impl Wiki {
    pub fn open(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let mut wiki = Wiki { root: root.to_path_buf(), pages: BTreeMap::new() };
        let mut files = Vec::new();
        collect_md(root, &mut files)?;
        for path in files {
            wiki.reload_path(&path);
        }
        Ok(wiki)
    }

    /// Writes a Welcome page if the wiki is empty.
    pub fn seed_if_empty(&mut self) -> io::Result<()> {
        if self.pages.is_empty() {
            self.save("Welcome", WELCOME)?;
        }
        Ok(())
    }

    pub fn get(&self, title: &str) -> Option<&Page> {
        self.pages.get(&key(title))
    }

    pub fn exists(&self, title: &str) -> bool {
        self.pages.contains_key(&key(title))
    }

    /// Converts a title like `notes/Rust` into a path under the root.
    pub fn path_for(&self, title: &str) -> io::Result<PathBuf> {
        let title = clean_title(title).ok_or_else(|| invalid("invalid page title"))?;
        let rel = PathBuf::from(format!("{title}.md"));
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(invalid("invalid page title"));
        }
        Ok(self.root.join(rel))
    }

    /// Title for a file under the root, or None if it isn't a visible `.md` file.
    pub fn title_for(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        if rel.extension()? != "md" {
            return None;
        }
        let parts: Vec<String> = rel
            .with_extension("")
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.is_empty() || parts.iter().any(|p| p.starts_with('.')) {
            return None;
        }
        Some(parts.join("/"))
    }

    /// Saves a page (creating it if needed). Keeps the existing title's
    /// capitalization when the page already exists.
    pub fn save(&mut self, title: &str, body: &str) -> io::Result<()> {
        let (title, path) = match self.get(title) {
            Some(p) => (p.title.clone(), p.path.clone()),
            None => (clean_title(title).ok_or_else(|| invalid("invalid page title"))?, self.path_for(title)?),
        };
        atomic_write(&path, body)?;
        self.pages.insert(key(&title), Page::new(title, path, body.to_string()));
        Ok(())
    }

    /// Re-reads one file from disk (used at startup and by the file watcher).
    /// Returns true if anything changed.
    pub fn reload_path(&mut self, path: &Path) -> bool {
        let Some(title) = self.title_for(path) else { return false };
        let k = key(&title);
        match fs::read_to_string(path) {
            Ok(body) => {
                if self.pages.get(&k).is_some_and(|p| p.body == body) {
                    return false;
                }
                self.pages.insert(k, Page::new(title, path.to_path_buf(), body));
                true
            }
            Err(_) if !path.exists() => self.pages.remove(&k).is_some(),
            Err(_) => false,
        }
    }

    pub fn delete(&mut self, title: &str) -> io::Result<()> {
        if let Some(p) = self.pages.remove(&key(title)) {
            fs::remove_file(&p.path)?;
            remove_empty_parents(&self.root, &p.path);
        }
        Ok(())
    }

    /// Renames a page and rewrites links to it in every page.
    /// Returns the number of other pages whose links were updated.
    pub fn rename(&mut self, old: &str, new: &str) -> io::Result<usize> {
        let new = clean_title(new).ok_or_else(|| invalid("invalid page title"))?;
        let page = self.get(old).ok_or_else(|| invalid("page not found"))?.clone();
        let same_page = key(&new) == key(&page.title);
        if !same_page && self.exists(&new) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("“{new}” already exists")));
        }
        let new_path = self.path_for(&new)?;
        if let Some(dir) = new_path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::rename(&page.path, &new_path)?;
        remove_empty_parents(&self.root, &page.path);
        self.pages.remove(&key(&page.title));
        let body = rewrite_links(&page.body, &page.title, &new).unwrap_or(page.body);
        atomic_write(&new_path, &body)?;
        self.pages.insert(key(&new), Page::new(new.clone(), new_path, body));

        let mut updated = 0;
        let others: Vec<(String, String)> = self
            .pages
            .values()
            .filter(|p| key(&p.title) != key(&new))
            .filter_map(|p| rewrite_links(&p.body, &page.title, &new).map(|b| (p.title.clone(), b)))
            .collect();
        for (title, body) in others {
            self.save(&title, &body)?;
            updated += 1;
        }
        Ok(updated)
    }
}

/// Trims a title, normalizes slashes and rejects empty or unsafe ones.
pub fn clean_title(title: &str) -> Option<String> {
    let parts: Vec<&str> = title
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty()
        || parts.iter().any(|p| *p == "." || *p == ".." || p.starts_with('.') || p.contains(['\0', '[', ']', '|']))
    {
        return None;
    }
    Some(parts.join("/"))
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_string())
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_md(&path, out)?;
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Writes to a hidden temp file next to `path`, then renames it into place.
fn atomic_write(path: &Path, body: &str) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| invalid("bad path"))?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or_else(|| invalid("bad path"))?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Removes now-empty subfolders between `path` and the wiki root.
fn remove_empty_parents(root: &Path, path: &Path) {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_and_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Wiki::open(dir.path()).unwrap();
        w.save("Rust", "hello [[Tokio]]").unwrap();
        w.save("notes/Idea", "#idea").unwrap();
        assert!(dir.path().join("notes/Idea.md").exists());

        let w2 = Wiki::open(dir.path()).unwrap();
        assert_eq!(w2.pages.len(), 2);
        assert_eq!(w2.get("rust").unwrap().body, "hello [[Tokio]]");
        assert_eq!(w2.get("NOTES/idea").unwrap().title, "notes/Idea");
    }

    #[test]
    fn rejects_bad_titles() {
        assert!(clean_title("../etc/passwd").is_none());
        assert!(clean_title("  ").is_none());
        assert!(clean_title(".hidden").is_none());
        assert_eq!(clean_title(" a / b ").as_deref(), Some("a/b"));
    }

    #[test]
    fn rename_rewrites_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Wiki::open(dir.path()).unwrap();
        w.save("B", "I am B, see [[b]]").unwrap();
        w.save("A", "link to [[B]] and [[b|bee]]").unwrap();
        w.save("C", "no links").unwrap();
        let n = w.rename("B", "dir/Bee").unwrap();
        assert_eq!(n, 1);
        assert_eq!(w.get("A").unwrap().body, "link to [[dir/Bee]] and [[dir/Bee|bee]]");
        assert_eq!(w.get("dir/bee").unwrap().body, "I am B, see [[dir/Bee]]");
        assert!(!dir.path().join("B.md").exists());
        assert!(dir.path().join("dir/Bee.md").exists());
        assert!(w.rename("A", "C").is_err());
    }

    #[test]
    fn delete_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Wiki::open(dir.path()).unwrap();
        w.save("sub/X", "x").unwrap();
        w.delete("sub/x").unwrap();
        assert!(w.pages.is_empty());
        assert!(!dir.path().join("sub").exists());

        let p = dir.path().join("Ext.md");
        fs::write(&p, "external").unwrap();
        assert!(w.reload_path(&p));
        assert!(!w.reload_path(&p), "unchanged content is not a change");
        fs::remove_file(&p).unwrap();
        assert!(w.reload_path(&p));
        assert!(w.pages.is_empty());
        assert!(!w.reload_path(&dir.path().join(".Ext.md.tmp")));
    }
}
