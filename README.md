# mindwiki

A personal desktop wiki for storing things you know. Built with Rust + egui.
Pages are plain Markdown files in a folder (default `~/wiki`), so you can also
edit them with any editor or keep them in git.

## Run

```sh
cargo run --release            # uses ~/wiki (or the folder from settings)
cargo run --release -- ~/notes # use another folder
```

## Features

- **Intro tour** on first open that walks through the app (replay it with **?**)
- **Edit / Split / Preview** modes with rendered Markdown
- **Wiki links**: `[[Page]]` or `[[Page|shown text]]`. Clicking a link to a page
  that doesn't exist yet (marked *(new)*) creates it.
- **Backlinks**: the bottom panel lists every page linking to the current one
- **Renaming** a page updates `[[links]]` to it in all other pages
- **Full-text search** (Ctrl+K): every word must match; `#tag` matches tags only
- **Tags**: `#tag` in the text, or front matter at the top of a page:
  ```
  ---
  tags: [rust, ideas]
  ---
  ```
  Click a tag in the sidebar to filter the page list.
- **Light and dark themes** (☀ / 🌙 in the top bar; follows the system until you pick one)
- **Folders**: a title like `notes/Rust` is saved as `notes/Rust.md`
- Autosave (2s after you stop typing; set it in ⚙ Settings), and pages changed
  by other programs reload automatically

## Sharing a wiki with a server

`mindwiki-server` shares one wiki with the people you give accounts to.
Everything goes over HTTPS, and nobody gets pages without logging in.

### Run the server

```sh
cargo build --release -p mindwiki-server
cd target/release

./mindwiki-server user add alice            # asks for a password (8+ characters)
./mindwiki-server user add bob --read-only  # bob can read but not change pages
./mindwiki-server serve --host wiki.example.com
```

- It listens on port 8443 (`--addr 0.0.0.0:8443`) and keeps everything in
  `./mindwiki-data` (`--data DIR`): accounts, the certificate and the pages.
- With no certificate it makes a **self-signed** one and prints its
  **fingerprint**. Give the fingerprint to your users. `--host` is the name or
  IP address they will type to reach the server (repeat it for several); it is
  only used when the certificate is first made. To change it later, delete
  `cert.pem` and `key.pem` and start again.
- With a real certificate (for example Let's Encrypt) use
  `--cert fullchain.pem --key privkey.pem`; users then need no fingerprint.
- Other commands: `user list`, `user passwd NAME`, `user readonly NAME on|off`,
  `user remove NAME`. They take effect without restarting the server.

### Connect from the app

Click **☁** in the top bar and enter the server address
(`https://wiki.example.com:8443`), your username and password, and the
fingerprint if the server is self-signed. The button then shows the sync
state (Synced, Syncing, Offline, Read-only). Click it again to sync now or
disconnect, which returns you to your local wiki.

- The app keeps a copy of the shared wiki on your computer, so it works
  offline; edits are sent when the server is reachable again.
- Changes by other people arrive within about 10 seconds.
- If two people edit the same page at once, the first upload wins and the
  other person's text is kept as a page named `… (conflicting copy)`.

### Security notes

- `http://` addresses are refused; the app only talks HTTPS.
- Passwords are stored on the server as Argon2 hashes. The app stores a login
  token (valid 30 days), never your password. The token is in
  `~/.config/mindwiki/config.toml`, readable only by you.
- Five wrong passwords lock that username and address out for a minute.
- For a self-signed server the app trusts only the exact certificate whose
  fingerprint you entered.

## Shortcuts

| Key | Action |
|---|---|
| Ctrl+K / Ctrl+P | Search |
| Ctrl+N | New page |
| Ctrl+S | Save |
| Ctrl+E | Toggle edit / preview |
| Ctrl+B | Show / hide the sidebar |
| Alt+← / Alt+→ | Back / forward |

Settings are stored in `~/.config/mindwiki/config.toml`.

## Code layout

- `src/page.rs`: parses front matter, tags and links
- `src/store.rs`: the folder on disk (load, atomic save, rename, delete)
- `src/index.rs`: backlinks and the tag map
- `src/search.rs`: search ranking and snippets
- `src/app.rs`: app state, history, file watcher, shortcuts
- `src/remote.rs`: HTTPS client for the server (login, pages)
- `src/sync.rs`: keeps the local copy and the server in step
- `server/`: the `mindwiki-server` program
- `src/ui/`: drawing (top bar, sidebar, preview, search popup, dialogs)
