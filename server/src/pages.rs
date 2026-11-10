//! The shared wiki on the server's disk: Markdown files under `<data>/wiki`.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn hash(body: &str) -> String {
    hex::encode(Sha256::digest(body.as_bytes()))
}

/// A title is safe when it is made of normal path parts: no empty parts, no
/// `.`/`..`, nothing hidden, no backslashes or characters the client rejects.
/// This is what stops a request from reaching files outside the wiki folder.
pub fn valid_title(title: &str) -> bool {
    !title.is_empty()
        && title.len() <= 300
        && title.split('/').all(|p| {
            !p.is_empty()
                && p == p.trim()
                && !p.starts_with('.')
                && !p.chars().any(|c| matches!(c, '\\' | '[' | ']' | '|' | ':') || c.is_control())
        })
}

pub enum WriteError {
    BadTitle,
    /// The page changed since the client last saw it. Holds the current hash.
    Conflict(Option<String>),
    /// Another page has the same name with different capital letters.
    CaseClash(String),
    Io(io::Error),
}

pub struct Pages {
    root: PathBuf,
    /// title -> content hash
    index: BTreeMap<String, String>,
}

impl Pages {
    pub fn open(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let mut pages = Pages { root: root.to_path_buf(), index: BTreeMap::new() };
        let mut files = Vec::new();
        collect(root, root, &mut files)?;
        for (title, path) in files {
            if let Ok(body) = fs::read_to_string(&path) {
                pages.index.insert(title, hash(&body));
            }
        }
        Ok(pages)
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn list(&self) -> Vec<(String, String)> {
        self.index.iter().map(|(t, h)| (t.clone(), h.clone())).collect()
    }

    fn path(&self, title: &str) -> PathBuf {
        self.root.join(format!("{title}.md"))
    }

    pub fn read(&self, title: &str) -> Option<(String, String)> {
        if !valid_title(title) || !self.index.contains_key(title) {
            return None;
        }
        let body = fs::read_to_string(self.path(title)).ok()?;
        let h = hash(&body);
        Some((body, h))
    }

    /// Saves a page if `base` matches what the server currently has
    /// (`None` = the client believes the page doesn't exist yet).
    pub fn write(&mut self, title: &str, body: &str, base: Option<&str>) -> Result<String, WriteError> {
        if !valid_title(title) {
            return Err(WriteError::BadTitle);
        }
        let current = self.index.get(title).map(String::as_str);
        let new_hash = hash(body);
        if current == Some(new_hash.as_str()) {
            return Ok(new_hash);
        }
        if current != base {
            return Err(WriteError::Conflict(current.map(str::to_string)));
        }
        if current.is_none() {
            let lower = title.to_lowercase();
            if let Some(other) = self.index.keys().find(|t| t.to_lowercase() == lower) {
                return Err(WriteError::CaseClash(other.clone()));
            }
        }
        let path = self.path(title);
        let dir = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir).map_err(WriteError::Io)?;
        let tmp = dir.join(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
        fs::write(&tmp, body).and_then(|()| fs::rename(&tmp, &path)).map_err(WriteError::Io)?;
        self.index.insert(title.to_string(), new_hash.clone());
        Ok(new_hash)
    }

    pub fn delete(&mut self, title: &str, base: Option<&str>) -> Result<(), WriteError> {
        if !valid_title(title) {
            return Err(WriteError::BadTitle);
        }
        let current = self.index.get(title).map(String::as_str);
        if current.is_none() {
            return Ok(());
        }
        if current != base {
            return Err(WriteError::Conflict(current.map(str::to_string)));
        }
        let path = self.path(title);
        fs::remove_file(&path).map_err(WriteError::Io)?;
        self.index.remove(title);
        // Tidy up folders that are now empty.
        let mut dir = path.parent();
        while let Some(d) = dir {
            if d == self.root || fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
        Ok(())
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect(root, &path, out)?;
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "md") {
            let rel = path.strip_prefix(root).unwrap_or(&path).with_extension("");
            let title: Vec<String> =
                rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let title = title.join("/");
            if valid_title(&title) {
                out.push((title, path));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        for bad in ["", "../x", "a/../b", "/abs", "a//b", ".hidden", "a/.b", "a\\b", " a", "a\0b", "C:x"] {
            assert!(!valid_title(bad), "{bad:?} should be rejected");
        }
        assert!(valid_title("Rust"));
        assert!(valid_title("notes/My Page"));
    }

    #[test]
    fn write_needs_matching_base() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Pages::open(dir.path()).unwrap();
        let h1 = p.write("A", "one", None).ok().unwrap();
        assert!(matches!(p.write("A", "two", None), Err(WriteError::Conflict(Some(_)))));
        let h2 = p.write("A", "two", Some(&h1)).ok().unwrap();
        assert!(matches!(p.write("a", "x", None), Err(WriteError::CaseClash(_))));
        assert!(matches!(p.delete("A", Some(&h1)), Err(WriteError::Conflict(_))));
        p.write("sub/B", "b", None).ok().unwrap();
        p.delete("sub/B", Some(&hash("b"))).ok().unwrap();
        assert!(!dir.path().join("sub").exists());

        let p2 = Pages::open(dir.path()).unwrap();
        assert_eq!(p2.list(), vec![("A".to_string(), h2)]);
        assert!(matches!(p.write("../evil", "x", None), Err(WriteError::BadTitle)));
    }
}
