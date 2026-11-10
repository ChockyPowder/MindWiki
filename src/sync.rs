//! Keeps a local wiki folder and a server in step.
//!
//! For every page we remember the content hash from the last successful sync
//! (the "base"). Comparing local, server and base tells us what happened:
//!
//! * only the server changed  -> download it (or delete our copy)
//! * only we changed          -> upload it (or delete it on the server)
//! * both changed differently -> keep ours as "… (conflicting copy)" and
//!   take the server's version, so nobody's text is lost

use crate::page::key;
use crate::remote::{Client, Error};
use crate::store::Wiki;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

const STATE_FILE: &str = ".mindwiki-sync.json";
const POLL: Duration = Duration::from_secs(10);

pub fn hash(body: &str) -> String {
    hex::encode(Sha256::digest(body.as_bytes()))
}

#[derive(Default, Debug)]
pub struct Report {
    pub pulled: usize,
    pub pushed: usize,
    /// Titles of the "conflicting copy" pages created this round.
    pub conflicts: Vec<String>,
    pub read_only: bool,
    /// Local edits that could not be uploaded because we are read-only.
    pub not_uploaded: usize,
}

/// lowercased title -> hash at the last sync
type Base = BTreeMap<String, String>;

fn load_base(dir: &Path) -> Base {
    std::fs::read_to_string(dir.join(STATE_FILE)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_base(dir: &Path, base: &Base) {
    if let Ok(json) = serde_json::to_string(base) {
        let tmp = dir.join(format!("{STATE_FILE}.tmp"));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, dir.join(STATE_FILE));
        }
    }
}

fn conflict_title(wiki: &Wiki, title: &str) -> String {
    let first = format!("{title} (conflicting copy)");
    if !wiki.exists(&first) {
        return first;
    }
    (2..).map(|n| format!("{title} (conflicting copy {n})")).find(|t| !wiki.exists(t)).unwrap()
}

fn io(e: std::io::Error) -> Error {
    Error::Other(format!("could not write to the local wiki folder: {e}"))
}

/// One round of syncing. Progress is saved even when it stops on an error.
pub fn sync_once(dir: &Path, client: &Client) -> Result<Report, Error> {
    let mut wiki = Wiki::open(dir).map_err(io)?;
    let mut base = load_base(dir);
    let before = base.clone();
    let mut report = Report::default();
    let result = run(&mut wiki, client, &mut base, &mut report);
    if base != before {
        save_base(dir, &base);
    }
    result.map(|()| report)
}

fn run(wiki: &mut Wiki, client: &Client, base: &mut Base, report: &mut Report) -> Result<(), Error> {
    let (remote_list, read_only) = client.list()?;
    report.read_only = read_only;
    // key -> (title as written, hash)
    let remote: BTreeMap<String, (String, String)> = remote_list.into_iter().map(|(t, h)| (key(&t), (t, h))).collect();
    let local: BTreeMap<String, (String, String)> =
        wiki.pages.iter().map(|(k, p)| (k.clone(), (p.title.clone(), hash(&p.body)))).collect();
    let keys: BTreeSet<String> = local.keys().chain(remote.keys()).chain(base.keys()).cloned().collect();

    for k in keys {
        let l = local.get(&k).map(|x| x.1.as_str());
        let r = remote.get(&k).map(|x| x.1.as_str());
        let b = base.get(&k).cloned();
        let b = b.as_deref();
        // Prefer the server's spelling of the title.
        let title = remote.get(&k).or(local.get(&k)).map(|x| x.0.clone()).unwrap_or_else(|| k.clone());

        if l == r {
            set(base, &k, l);
            continue;
        }
        let local_changed = l != b;
        let remote_changed = r != b;

        if local_changed && remote_changed && l.is_some() && r.is_some() {
            // Both edited: keep our text under another name.
            let ours = wiki.get(&title).map(|p| p.body.clone()).unwrap_or_default();
            let copy = conflict_title(wiki, &title);
            wiki.save(&copy, &ours).map_err(io)?;
            report.conflicts.push(copy);
        }

        if remote_changed && (r.is_some() || !local_changed) {
            // Take the server's version (this also wins over a local delete).
            match r {
                Some(rh) => {
                    let body = client.get(&title)?;
                    wiki.save(&title, &body).map_err(io)?;
                    set(base, &k, Some(rh));
                }
                None => {
                    wiki.delete(&title).map_err(io)?;
                    set(base, &k, None);
                }
            }
            report.pulled += 1;
        } else {
            // Only we changed it (or the server deleted a page we edited).
            if read_only {
                report.not_uploaded += 1;
                continue;
            }
            let outcome = match l {
                Some(_) => {
                    let body = wiki.get(&title).map(|p| p.body.clone()).unwrap_or_default();
                    client.put(&title, &body, r)
                }
                None => client.delete(&title, r),
            };
            match outcome {
                Ok(()) => {
                    set(base, &k, l);
                    report.pushed += 1;
                }
                // Someone else got there first; the next round sorts it out.
                Err(Error::Conflict) => {}
                Err(Error::Forbidden) => {
                    report.read_only = true;
                    report.not_uploaded += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

fn set(base: &mut Base, k: &str, hash: Option<&str>) {
    match hash {
        Some(h) => base.insert(k.to_string(), h.to_string()),
        None => base.remove(k),
    };
}

// ---------- background thread ----------

#[derive(Clone, Debug)]
pub enum Status {
    Syncing,
    Synced { read_only: bool, not_uploaded: usize, conflicts: Vec<String> },
    Offline(String),
    /// The server no longer accepts our login.
    LoggedOut,
}

pub struct SyncHandle {
    /// Send to sync right away. Dropping the handle stops the thread.
    wake: Sender<()>,
    pub status: Receiver<Status>,
}

impl SyncHandle {
    pub fn sync_now(&self) {
        let _ = self.wake.send(());
    }
}

/// Syncs `dir` now, whenever asked, and every few seconds. `notify` is
/// called after each round (the app uses it to redraw).
pub fn spawn(dir: PathBuf, client: Client, notify: impl Fn() + Send + 'static) -> SyncHandle {
    let (wake, wake_rx) = channel::<()>();
    let (status_tx, status) = channel();
    std::thread::spawn(move || {
        loop {
            let s = match sync_once(&dir, &client) {
                Ok(r) => Status::Synced { read_only: r.read_only, not_uploaded: r.not_uploaded, conflicts: r.conflicts },
                Err(Error::Auth) => Status::LoggedOut,
                Err(e) => Status::Offline(e.to_string()),
            };
            let stop = matches!(s, Status::LoggedOut);
            if status_tx.send(s).is_err() || stop {
                notify();
                break;
            }
            notify();
            match wake_rx.recv_timeout(POLL) {
                Ok(()) => while wake_rx.try_recv().is_ok() {},
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    SyncHandle { wake, status }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{self, Remote};
    use mindwiki_server::users::Users;
    use std::fs;

    struct Server {
        url: String,
        fingerprint: String,
        handle: mindwiki_server::Running,
        _data: tempfile::TempDir,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.handle.handle.shutdown();
        }
    }

    /// Starts a real HTTPS server on a free port with users alice (can edit)
    /// and rob (read-only).
    fn server() -> Server {
        let data = tempfile::tempdir().unwrap();
        let users = Users::new(data.path());
        users.add("alice", "alice-password", false).unwrap();
        users.add("rob", "rob-password", true).unwrap();
        let opts = mindwiki_server::Options {
            data: data.path().to_path_buf(),
            addr: "127.0.0.1:0".parse().unwrap(),
            cert: None,
            key: None,
            hosts: vec![],
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let (running, task) = mindwiki_server::start(opts).await.unwrap();
                tx.send(running).unwrap();
                let _ = task.await;
            });
        });
        let handle = rx.recv().unwrap();
        Server {
            url: format!("https://127.0.0.1:{}", handle.addr.port()),
            fingerprint: handle.fingerprint.clone(),
            handle,
            _data: data,
        }
    }

    fn login(s: &Server, user: &str, pass: &str) -> Remote {
        remote::connect(&s.url, user, pass, &s.fingerprint).unwrap()
    }

    #[test]
    fn login_rules() {
        let s = server();
        // Self-signed server without a fingerprint is not trusted.
        let e = remote::connect(&s.url, "alice", "alice-password", "").unwrap_err();
        assert!(e.contains("certificate"), "{e}");
        // Wrong fingerprint is refused before any password is sent.
        let e = remote::connect(&s.url, "alice", "alice-password", &"0".repeat(64)).unwrap_err();
        assert!(e.contains("doesn't match"), "{e}");
        // Wrong password and unknown user get the same answer.
        let e1 = remote::connect(&s.url, "alice", "nope-nope", &s.fingerprint).unwrap_err();
        let e2 = remote::connect(&s.url, "nobody", "nope-nope", &s.fingerprint).unwrap_err();
        assert_eq!(e1, e2);
        assert!(e1.contains("wrong username or password"));

        // A made-up token gets nothing.
        let mut r = login(&s, "alice", "alice-password");
        let good = r.token.clone();
        r.token = "f".repeat(64);
        assert!(matches!(Client::new(&r).unwrap().list(), Err(Error::Auth)));
        // Path tricks are rejected.
        r.token = good;
        let c = Client::new(&r).unwrap();
        assert!(matches!(c.put("../../evil", "x", None), Err(Error::Other(_))));
        assert!(c.get("../users").is_err());
        // After logout the token is dead.
        c.logout();
        assert!(matches!(c.list(), Err(Error::Auth)));
    }

    #[test]
    fn lockout_after_repeated_failures() {
        let s = server();
        for _ in 0..5 {
            let _ = remote::connect(&s.url, "alice", "bad-password", &s.fingerprint);
        }
        let e = remote::connect(&s.url, "alice", "alice-password", &s.fingerprint).unwrap_err();
        assert!(e.contains("too many"), "{e}");
    }

    #[test]
    fn two_computers_share_a_wiki() {
        let s = server();
        let alice = Client::new(&login(&s, "alice", "alice-password")).unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();

        // First sync downloads the server's Welcome page.
        let r = sync_once(a.path(), &alice).unwrap();
        assert_eq!(r.pulled, 1);
        assert!(a.path().join("Welcome.md").exists());

        // A writes a page; B receives it.
        fs::create_dir_all(a.path().join("notes")).unwrap();
        fs::write(a.path().join("notes/Rust.md"), "v1").unwrap();
        assert_eq!(sync_once(a.path(), &alice).unwrap().pushed, 1);
        sync_once(b.path(), &alice).unwrap();
        assert_eq!(fs::read_to_string(b.path().join("notes/Rust.md")).unwrap(), "v1");

        // B edits; A receives the edit. Nothing more to do afterwards.
        fs::write(b.path().join("notes/Rust.md"), "v2").unwrap();
        assert_eq!(sync_once(b.path(), &alice).unwrap().pushed, 1);
        assert_eq!(sync_once(a.path(), &alice).unwrap().pulled, 1);
        assert_eq!(fs::read_to_string(a.path().join("notes/Rust.md")).unwrap(), "v2");
        let r = sync_once(a.path(), &alice).unwrap();
        assert_eq!((r.pulled, r.pushed), (0, 0));

        // Both edit the same page: B's upload wins, A keeps a conflicting copy.
        fs::write(a.path().join("notes/Rust.md"), "from A").unwrap();
        fs::write(b.path().join("notes/Rust.md"), "from B").unwrap();
        sync_once(b.path(), &alice).unwrap();
        let r = sync_once(a.path(), &alice).unwrap();
        assert_eq!(r.conflicts, vec!["notes/Rust (conflicting copy)"]);
        assert_eq!(fs::read_to_string(a.path().join("notes/Rust.md")).unwrap(), "from B");
        assert_eq!(fs::read_to_string(a.path().join("notes/Rust (conflicting copy).md")).unwrap(), "from A");
        // The copy is uploaded on the next round and reaches B.
        assert_eq!(sync_once(a.path(), &alice).unwrap().pushed, 1);
        sync_once(b.path(), &alice).unwrap();
        assert!(b.path().join("notes/Rust (conflicting copy).md").exists());

        // A deletes a page; it disappears for B too.
        fs::remove_file(a.path().join("Welcome.md")).unwrap();
        sync_once(a.path(), &alice).unwrap();
        sync_once(b.path(), &alice).unwrap();
        assert!(!b.path().join("Welcome.md").exists());

        // A deletes while B edits: the edit survives and comes back to A.
        fs::remove_file(a.path().join("notes/Rust.md")).unwrap();
        fs::write(b.path().join("notes/Rust.md"), "still wanted").unwrap();
        sync_once(b.path(), &alice).unwrap();
        sync_once(a.path(), &alice).unwrap();
        assert_eq!(fs::read_to_string(a.path().join("notes/Rust.md")).unwrap(), "still wanted");
    }

    #[test]
    fn read_only_user_cannot_change_the_server() {
        let s = server();
        let rob = Client::new(&login(&s, "rob", "rob-password")).unwrap();
        let alice = Client::new(&login(&s, "alice", "alice-password")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        sync_once(dir.path(), &rob).unwrap();
        fs::write(dir.path().join("Welcome.md"), "defaced").unwrap();
        fs::write(dir.path().join("New.md"), "new").unwrap();
        let r = sync_once(dir.path(), &rob).unwrap();
        assert!(r.read_only);
        assert_eq!((r.pushed, r.not_uploaded), (0, 2));
        assert!(matches!(rob.put("New", "new", None), Err(Error::Forbidden)));
        assert!(matches!(rob.delete("Welcome", None), Err(Error::Forbidden)));
        let (pages, _) = alice.list().unwrap();
        assert_eq!(pages.len(), 1);
        assert!(!alice.get("Welcome").unwrap().contains("defaced"));
    }
}
