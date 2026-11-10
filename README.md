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
- `src/ui/`: drawing (top bar, sidebar, preview, search popup, dialogs)
