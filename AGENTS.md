# Ink Reader — Project Knowledge Base

## Overview
Terminal TUI e-book reader written in Rust. Supports EPUB and TXT formats.
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
    fn cover_image(&self) -> Option<(&[u8], &str)> { None }  // default: no cover
}
```

`BookMeta` contains `title`, `author: Option<String>`, and `chapters: Vec<Chapter>`.
`ContentBlock` is an enum: `Paragraph(String)`, `Heading { level, text }`, `Image { data, alt, mime }`, `PageBreak`.
`Page` has `lines: Vec<String>`, `image: Option<PageImage>`, `first_block: usize`.

### Key Functions in book.rs
- `pub(crate) fn detect_image_mime(data: &[u8]) -> &'static str` — magic-byte MIME sniff.
  Returns `"image/jpeg"`, `"image/png"`, `"image/gif"`, `"image/webp"`, or `"image/unknown"`.
  **All format readers must use this** — never write a local copy. `"image/unknown"` fallback
  is intentional (not `"image/jpeg"`).
- `paginate_blocks(blocks, width, height)` — reflow ContentBlocks into pages.

### EPUB Inline Image Extraction (epub.rs)
`collect_chapters()` must follow the **EPUB spine**, not just top-level ToC entries. The ToC is only a
title source: flatten it, strip fragments, and let the **first label for each XHTML resource** name the
spine chapter. This matters for books whose NCX nests multiple section anchors inside one spine document
(for example `Text/Section0001.xhtml#hh2-1`).

`chapter_blocks()` uses a **sentinel injection** pattern to preserve image position through html2text:
1. Scan raw HTML for `<img>` tags → collect `(src, alt)` pairs (`extract_img_tags`)
2. Replace each `<img>` with `</p><p>__INKIMG_N__</p><p>` in the HTML string
3. Run html2text on the modified HTML
4. Split result on `\n\n`; swap `__INKIMG_N__` paragraphs back to `ContentBlock::Image`
5. Failed/unsupported (SVG) images emit `[Image: alt]` placeholder paragraph

Image pages may also carry **caption lines** in `Page.lines`: `paginate_blocks()` keeps the
immediate figure/table caption blocks (for example `图1 …` plus following parenthetical source note)
with the image page, and `ui/reader.rs` renders those lines **below** the image instead of treating
them as normal body paragraphs.

Helper functions (module-level in epub.rs):
- `extract_img_tags(html)` → `Vec<(src, alt)>` — case-insensitive, handles `data-src` shadowing
- `extract_attr(tag, attr)` → `Option<String>` — iterates all occurrences to skip false matches
- `resolve_href(chapter_href, img_src)` — handles `./`, `../` (clamped), fragment, external URLs
- `normalize_path(path)` — strips `.`, resolves `..` without going above root
- `resource_path(resource_id)` — strips fragment suffix before `read_resource_bytes()`
- `parse_img_sentinel(para)` — detects `__INKIMG_N__` paragraphs, returns index N

Image bytes are stored raw at chapter load; full decode via `image::load_from_memory` is deferred to display time in `refresh_current_image()` to avoid decompression-bomb risk.

## Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| ratatui | 0.30 | TUI framework |
| crossterm | 0.29 | Terminal backend |
| ratatui-image | 10.x | Terminal image display (Kitty/Sixel/half-block) — use `Picker::halfblocks()` as fallback, NOT `Picker::new()` |
| rbook | 0.7 | EPUB 2+3 parsing |
| html2text | 0.17 | HTML→plain text for EPUB content |
| textwrap | 0.16 | Word-wrap text to terminal width |
| serde + serde_json | 1.x | Bookmark serialization |
| dirs | 5.x | XDG paths (~/.local/share) |
| anyhow | 1.x | Application-level error handling |
| clap | 4.x | CLI argument parsing |

## Build & Run

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
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
- **Cover image**: Displayed on open for EPUB (manifest cover-image or id/href hint)
- **Inline illustrations**: EPUB chapter illustrations rendered in-place; SVG/unsupported images shown as `[Image: alt]` placeholder
- **Images**: Auto-detect terminal protocol; fallback to half-block if unsupported
- **Formats**: EPUB, TXT

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
