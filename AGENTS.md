# Ink Reader — Project Knowledge Base

## Overview
Terminal TUI e-book reader written in Rust. Supports EPUB, MOBI, AZW3, TXT, PDF formats.
Runs on Linux terminal with image display via Kitty/Sixel protocol.

## Architecture

### Module Structure
```
src/
├── main.rs           # Entry point, CLI args parsing
├── app.rs            # Application state machine (ratatui event loop)
├── book.rs           # Unified Book/Page representation
├── formats/
│   ├── mod.rs        # BookReader trait definition
│   ├── epub.rs       # EPUB parser (uses `epub` crate)
│   ├── mobi.rs       # MOBI + AZW3 parser (uses `mobi` crate)
│   ├── pdf.rs        # PDF text extraction (uses `pdf-extract`)
│   └── txt.rs        # Plain text reader
├── ui/
│   ├── mod.rs
│   ├── reader.rs     # Main reading view (paginated text + images)
│   ├── toc.rs        # Table of contents / chapter selection popup
│   └── bookmarks.rs  # Bookmark management popup
└── storage.rs        # Bookmark persistence (~/.local/share/ink-reader/)
```

### Core Trait
```rust
pub trait BookReader {
    fn meta(&self) -> &BookMeta;
    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>>;
}
```

`BookMeta` contains `title`, `author: Option<String>`, and `chapters: Vec<Chapter>`.
`ContentBlock` is an enum: `Paragraph(String)`, `Heading { level, text }`, `Image { data, alt, mime }`, `PageBreak`.
`Page` has `lines: Vec<String>`, `image: Option<PageImage>`, `first_block: usize`.

### Data Flow
CLI arg (file path) → detect format → load BookReader impl → App state → ratatui render loop

## Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| ratatui | 0.30 | TUI framework |
| crossterm | 0.29 | Terminal backend |
| ratatui-image | 10.x | Terminal image display (Kitty/Sixel/half-block) — use `Picker::halfblocks()` as fallback, NOT `Picker::new()` |
| rbook | 0.7 | EPUB 2+3 parsing |
| mobi | 0.8 | MOBI + AZW3 — `title()` returns `String`, `author()` returns `Option<String>` |
| pdf_oxide | 0.3 | PDF text — `page_count()` returns `Result<usize>`, use `?` |
| html2text | 0.17 | HTML→plain text for MOBI/EPUB content |
| textwrap | 0.16 | Word-wrap text to terminal width |
| serde + serde_json | 1.x | Bookmark serialization |
| dirs | 5.x | XDG paths (~/.local/share) |
| anyhow | 1.x | Application-level error handling |
| clap | 4.x | CLI argument parsing |

## Build & Run

```bash
cargo build
cargo run -- /path/to/book.epub
cargo test
```

## Key Bindings
- `←` / `→` or `h` / `l`: Previous / next page
- `t` or `T`: Open ToC (chapter selection)
- `b` or `B`: Open bookmarks panel
- `a`: Add bookmark at current position
- `q` or `Esc`: Quit (or close popup)
- `j` / `k`: Scroll within popup lists

## Features
- **Pagination**: Text is reflowed to terminal dimensions on resize
- **Bookmarks**: Stored in `~/.local/share/ink-reader/bookmarks.json`
- **Chapter navigation**: Popup ToC with selectable chapters
- **Images**: Auto-detect terminal protocol; fallback to half-block if unsupported
- **Formats**: EPUB, MOBI, AZW3, TXT, PDF

## Code Conventions
- Use `anyhow::Result` for all error handling in binary code
- Use `thiserror` for library-level custom errors
- All format parsers implement the `BookReader` trait
- UI components are stateless render functions (state lives in `App`)
- Bookmark file: `~/.local/share/ink-reader/bookmarks.json`

## Critical Rules
- Never panic on malformed ebook data — return errors gracefully
- Terminal dimensions must be re-queried before paginating (handle resize events)
- Image display is always optional — reader must work in text-only mode
