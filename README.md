# ink-reader

> Ink on the terminal — read ebooks in your terminal.

A fast, keyboard-driven TUI e-book reader for Linux/macOS built with Rust and
[Ratatui](https://github.com/ratatui/ratatui). Open EPUB and TXT files without
leaving the command line. It now renders book cover art, expands footnotes into
styled inline references, and shows inline illustrations directly inside
supported terminals.

![Rust](https://img.shields.io/badge/rust-2024_edition-orange)
[![CI](https://github.com/AlexiaChen/ink-reader/actions/workflows/ci.yml/badge.svg)](https://github.com/AlexiaChen/ink-reader/actions/workflows/ci.yml)
![License](https://img.shields.io/badge/license-MIT-blue)

---

## Preview

<table>
  <tr>
    <td align="center" width="33.33%">
      <img src="doc/image/cover.png" alt="ink-reader displaying a book cover inside the terminal" />
    </td>
    <td align="center" width="33.33%">
      <img src="doc/image/inline.jpg" alt="ink-reader rendering inline illustrations inside book content" />
    </td>
    <td align="center" width="33.33%">
      <img src="doc/image/footprint.png" alt="ink-reader expanding EPUB footnotes into styled inline references" />
    </td>
  </tr>
  <tr>
    <td align="center"><strong>Cover art</strong><br />Open a book and see its cover before you start reading.</td>
    <td align="center"><strong>Inline illustrations</strong><br />Keep images and nearby captions in the reading flow.</td>
    <td align="center"><strong>Inline references</strong><br />Expand EPUB footnotes into readable inline notes with distinct styling.</td>
  </tr>
</table>

Image rendering uses terminal image protocols when available and gracefully
falls back so reading still works in text-only environments.

---

## Features

| Feature | Details |
|---------|---------|
| **Format support** | EPUB, TXT |
| **Table of Contents** | Overlay (`t`) to jump to any chapter instantly |
| **Bookmarks** | Save/overwrite (`s`), browse (`b`), delete (`d`), jump to the saved bookmark |
| **Page navigation** | `↓` / `Space` next page · `↑` prev page |
| **Chapter navigation** | `n` next chapter · `p` prev chapter |
| **Page-flip animation** | Smooth fan-in/fan-out effect when turning pages |
| **Paragraph indent** | 4-space first-line indent for comfortable reading |
| **Cover art** | Displays EPUB covers in-terminal when image rendering is available |
| **Inline references** | Expands EPUB footnote/reference markers like `[4]` into parenthesized inline citation text with distinct styling |
| **Inline illustrations** | Renders chapter images in place and keeps nearby captions with the figure |
| **Persistent state** | One bookmark per book, auto-saved on quit to `~/.local/share/ink-reader/bookmarks.json` |
| **Responsive layout** | Reflows text automatically on terminal resize |

---

## Installation

### Prerequisites

- Rust toolchain (edition 2024) — install via [rustup](https://rustup.rs/)

### From source

```bash
git clone https://github.com/AlexiaChen/ink-reader
cd ink-reader
cargo build --release
# binary at: target/release/ink-reader
```

### System-wide install

```bash
# without sudo
cargo install --path .

# with sudo (Makefile handles the rustup HOME quirk automatically)
sudo make install
```

---

## Usage

```
ink-reader <FILE>
```

### Keyboard shortcuts

#### Reading mode

| Key | Action |
|-----|--------|
| `↓` / `Space` | Next page |
| `↑` | Previous page |
| `n` | Next chapter |
| `p` | Previous chapter |
| `t` | Open Table of Contents |
| `b` | Open Bookmarks |
| `s` | Save or overwrite the bookmark at the current position |
| `q` / `Esc` / `Ctrl-c` | Quit |

#### Table of Contents overlay (`t`)

| Key | Action |
|-----|--------|
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Jump to selected chapter |
| `t` / `q` / `Esc` | Close overlay |

#### Bookmarks overlay (`b`)

| Key | Action |
|-----|--------|
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Jump to selected bookmark |
| `d` | Delete selected bookmark |
| `b` / `q` / `Esc` | Close overlay |

---

## Build & Development

```bash
# Check formatting
cargo fmt --check

# Run clippy with CI-level strictness
cargo clippy --all-targets -- -D warnings

# Build (also runs clippy)
make build

# Run tests
make test

# Install to /usr/local/bin
make install

# Remove build artifacts
make clean
```

## CI

GitHub Actions runs on pull requests and pushes to `master`, checking:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo build --release`

---

## Project structure

```
src/
├── main.rs          # Entry point — event loop, terminal setup/teardown
├── app.rs           # Application state machine (reading / ToC / bookmarks modes)
├── book.rs          # Core types, pagination, text-wrapping
├── formats/
│   ├── epub.rs      # EPUB reader (rbook)
│   └── txt.rs       # Plain-text reader
├── storage.rs       # Bookmark persistence (JSON via serde)
└── ui/
    └── reader.rs    # Ratatui rendering (status bar, content, help bar, animation)
```

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | Terminal UI framework |
| `crossterm` | Cross-platform terminal control |
| `ratatui-image` | Inline image rendering |
| `rbook` | EPUB parsing |
| `html2text` | HTML-to-plain-text for EPUB content |
| `textwrap` | Unicode-aware text wrapping with indent support |
| `clap` | CLI argument parsing |
| `serde` / `serde_json` | Bookmark serialization |

---

## License

MIT — see [LICENSE](LICENSE).
