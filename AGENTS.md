# Ink Reader — Project Knowledge Base

## Overview
Terminal TUI e-book reader written in Rust. Supports EPUB, PDF, and TXT formats.
Runs on Linux terminal with image display via Kitty/Sixel protocol.

## Architecture

### Module Structure
```
src/
├── main.rs           # Entry point, CLI args parsing
├── app.rs            # Application state machine (ratatui event loop)
├── book.rs           # Unified Book/Page representation
├── copilot.rs        # Rig reading agent + background streaming state
├── math_render.rs    # Markdown math events + terminal-native 2D LaTeX layout
├── formats/
│   ├── mod.rs        # BookReader trait definition
│   ├── epub.rs       # EPUB parser (uses `rbook` crate)
│   ├── pdf.rs        # PDF parser/extractor (uses `pdf_oxide` crate)
│   └── txt.rs        # Plain text reader
├── ui/
│   ├── mod.rs
│   ├── reader.rs     # Main reading view (paginated text + images)
│   ├── copilot.rs    # Reading Copilot responsive right panel
│   ├── toc.rs        # Table of contents / chapter selection popup
│   └── bookmarks.rs  # Bookmark management popup
└── storage.rs        # Bookmark persistence (~/.local/share/ink-reader/)
```

### Core Trait
```rust
pub trait BookReader {
    fn meta(&self) -> &BookMeta;
    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>>;
    fn toc_entries(&self) -> Option<&[TocEntry]> { None } // optional authored outline
    fn cover_image(&self) -> Option<(&[u8], &str)> { None }  // default: no cover
}
```

`BookMeta` contains `title`, `author: Option<String>`, and `chapters: Vec<Chapter>`.
`ContentBlock` is an enum: `Paragraph(String)`, `Heading { level, text }`, `SectionMarker(String)`, `Image { data, alt, mime }`, `PageBreak`.
`Page` has `lines: Vec<String>`, `image: Option<PageImage>`, `first_block: usize`, and `section_title: Option<String>`.

### Key Functions in book.rs
- `pub(crate) fn detect_image_mime(data: &[u8]) -> &'static str` — magic-byte MIME sniff.
  Returns `"image/jpeg"`, `"image/png"`, `"image/gif"`, `"image/webp"`, or `"image/unknown"`.
  **All format readers must use this** — never write a local copy. `"image/unknown"` fallback
  is intentional (not `"image/jpeg"`).
- `paginate_blocks(blocks, width, height)` — reflow ContentBlocks into pages.

### PDF Extraction (pdf.rs)
PDF support uses exactly `pdf_oxide 0.3.77` with its `rendering` feature. Each source PDF page is
one logical `Chapter`, so existing page/chapter navigation, bookmarks, resize reflow, and status
counters work without a separate PDF UI mode. `PdfDocument::to_markdown()` supplies structure-tree-
first reading order, heading detection, form values, and tagged/spatial table extraction; the adapter
converts headings to `ContentBlock::Heading` and retains GFM tables as multi-line paragraphs.

`PdfDocument::get_outline()` is flattened into optional `TocEntry` values that jump to the target
PDF page; named destinations are resolved with `resolve_named_destination()`. If no authored outline
exists, the ToC overlay falls back to the per-page chapter list. PDF page labels are used for fallback
page titles when present.

Embedded PDF images are extracted from page content streams (including nested Form XObjects),
normalized with `PdfImage::to_png_bytes()`, and emitted as `ContentBlock::Image`. Full decode remains
deferred to `App::refresh_current_image()`. The first PDF page is rendered at 96 DPI with the
pure-Rust renderer and exposed through `cover_image()`; rendering failure is non-fatal.

PDF metadata prefers XMP Dublin Core title/creators, then falls back to the traditional trailer
`/Info` Title/Author strings and finally the filename.

### EPUB Inline Image & Reference Extraction (epub.rs)
`collect_chapters()` must follow the **EPUB spine**, but chapter identity is now **fragment-aware**:
flatten the ToC, group labels by XHTML resource, and expand each spine resource into one or more logical
chapters in spine order. If a resource carries multiple ToC anchors (for example
`Text/Section0001.xhtml#hh2-1` / `#hh2-2`), each fragment becomes its own `Chapter.resource_id`
(`path.xhtml#fragment`) so the status bar, `n` / `p` navigation, ToC, and `x/y ch` counter all track
the visible logical chapter instead of the coarse resource count.

`chapter_blocks()` now performs three EPUB-specific preprocess passes before `html2text`:
1. **Inline reference expansion**: footnote/noteref-style anchors such as `#note_2` or `notes.xhtml#n2`
   are resolved to their target block text and wrapped with hidden single-character sentinels in the
   paginated text data. `ui/reader.rs` then renders those sentinels as parenthesized inline notes
   with cyan + italic styling, so they read differently from body text without leaking raw markers
   into wrapped lines. Some EPUBs put the target `id` on an inline backlink anchor inside a
   footnote paragraph (for example `<p class="kindle-cn-footnote"><a id="ft12">[12]</a>正文…</p>`):
   extraction must fall back to the nearest enclosing block container and strip the inline target
   anchor itself before `html2text`, otherwise the backlink is re-emitted as markdown-style link
   definitions in the inline note text. Image-only note markers (`<a><img ...></a>`) also count as
   references when the target fragment / target block looks footnote-like, which avoids leaking
   html2text output such as `[__INKIMG_0__][1]` for image-backed footnote markers while keeping
   ordinary image navigation links untouched.
2. **Image sentinel injection**: preserve image position through html2text by:
   1. Scanning raw HTML for `<img>` tags → collect `(src, alt)` pairs (`extract_img_tags`)
   2. Replacing each `<img>` with `</p><p>__INKIMG_N__</p><p>` in the HTML string
   3. Running html2text on the modified HTML
   4. Splitting result on `\n\n`; swapping `__INKIMG_N__` paragraphs back to `ContentBlock::Image`
   5. Falling back to `[Image: alt]` placeholder paragraphs for failed/unsupported (SVG) images
3. **Section sentinel injection**: ToC fragment labels within the sliced XHTML section are resolved
   back onto matching `id` / `xml:id` / `name` anchors and injected as `__INKSEC_N__` paragraphs.
   After `html2text`, those paragraphs become `ContentBlock::SectionMarker`, letting
   `paginate_blocks()` stamp `Page.section_title` so the status bar and bookmark titles follow the
   visible in-resource section instead of staying pinned to the first spine label.

Image pages may also carry **caption lines** in `Page.lines`: `paginate_blocks()` keeps the
immediate figure/table caption blocks (for example `图1 …` plus following parenthetical source note)
with the image page, and `ui/reader.rs` renders those lines **below** the image instead of treating
them as normal body paragraphs.

Helper functions (module-level in epub.rs):
- `extract_img_tags(html)` → `Vec<(src, alt)>` — case-insensitive, handles `data-src` shadowing
- `extract_attr(tag, attr)` → `Option<String>` — iterates all occurrences to skip false matches
- `resolve_href(chapter_href, img_src)` — handles `./`, `../` (clamped), fragment, external URLs
- `resolve_reference_target(chapter_href, link_href)` — resolves `#id` / `path.xhtml#id` reference links
- `inline_reference_links(html, chapter_href, load_resource_html)` — expands footnote markers inline
- `slice_resource_html(html, start_fragment, end_fragment)` — trims one XHTML resource down to the current logical chapter span
- `inject_section_sentinels(html, section_labels)` — injects `__INKSEC_N__` before matching fragment anchors
- `normalize_path(path)` — strips `.`, resolves `..` without going above root
- `resource_path(resource_id)` — strips fragment suffix before `read_resource_bytes()`
- `parse_img_sentinel(para)` — detects `__INKIMG_N__` paragraphs and markdown link-wrapped forms like `[__INKIMG_N__][1]`, returns index N
- `parse_section_sentinel(para)` — detects `__INKSEC_N__` paragraphs, returns index N

Image bytes are stored raw at chapter load; full decode via `image::load_from_memory` is deferred to display time in `refresh_current_image()` to avoid decompression-bomb risk.

### Terminal Image & Overlay Invariant
Kitty/Sixel/iTerm2 graphics are not ordinary ratatui character cells. `ui/reader.rs` must not render
`StatefulImage` while a ToC or bookmark overlay is active. Entering or leaving an overlay, and every
transition between text and images or between two images, must request `Terminal::clear()` through
`App::take_terminal_clear_request()` before the next draw. Closing an overlay rebuilds the current
image protocol so the reading page can be restored after that full clear. `Clear` in the popup widgets
is still required for their character-cell background, but is not sufficient to erase terminal graphics.

### Reading Copilot / Agent Boundary
`copilot.rs` builds a page-scoped Rig Agent and streams its output on a dedicated thread/runtime;
model IO must never block the ratatui event loop. The first version gives the Agent no tools and
copies only the current visible text page plus book/location labels into its context. Cover and
image-only pages are rejected rather than silently sending unrelated or empty context.

The default provider is local Ollama at `http://127.0.0.1:11434`, with `qwen3.5:4b` for all tasks.
An optional `--copilot-reasoning-model` is used only for deep analysis. Provider/model/endpoint/API
key can be overridden by CLI/env; non-loopback endpoints must be visibly marked remote because the
page excerpt leaves the machine. Never display or persist API keys. Ollama is an optional runtime,
not a build/test prerequisite.

Rig is the long-term Agent First boundary. Future RAG, conversation memory, and book tools should use
Rig abstractions without moving provider IO into `App` or UI modules. New Agent tools start read-only;
any state-changing tool needs explicit product authorization and tests.

On terminals at least 90 columns wide, Copilot is a 40–64 column right panel and the book remains
visible on the left. Opening, closing, or resizing the panel must repaginate against the actual reader
pane width and preserve the approximate chapter position. Narrow terminals use a full-screen fallback.
Unlike a popup, a side-by-side Copilot panel may render a terminal image only inside the disjoint left
pane; transitions still require a full terminal clear. ToC/bookmark popups must continue suppressing
all terminal image rendering.

Copilot answer math uses `pulldown-cmark` `ENABLE_MATH` events and `term-maths`, not terminal images.
Completed `$...$` / `$$...$$` regions become Unicode 2D formula lines at their answer position;
unfinished streaming delimiters remain source text. Inline single-row formulae stay in prose, while
multi-row formulae become centered blocks. Code spans must not be parsed as math. Formula parsing is
untrusted model output: preserve the length/nesting limits, panic boundary, bounded cache, and visible
LaTeX fallback for over-wide/unsafe input. Never silently discard part of a formula.
`AGENT_SYSTEM_PROMPT` is the producer side of this protocol: it must keep requiring only `$...$` and
`$$...$$`, reject alternate math delimiters/code fences, and steer models to the supported TeX subset.
Any renderer syntax change must update the prompt contract and its regression test in the same slice.

## Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| ratatui | 0.30 | TUI framework |
| crossterm | 0.29 | Terminal backend |
| ratatui-image | 10.x | Terminal image display (Kitty/Sixel/half-block) — use `Picker::halfblocks()` as fallback, NOT `Picker::new()` |
| rbook | 0.7 | EPUB 2+3 parsing |
| pdf_oxide | 0.3.77 + rendering | PDF metadata, outline, text/table/image extraction, page rendering |
| rig-core | 0.40 | Reading Agent, Ollama provider, streaming, future tools/RAG/memory |
| tokio + futures-util | 1.x / 0.3 | Background Agent runtime and response stream |
| html2text | 0.17 | HTML→plain text for EPUB content |
| textwrap | 0.16 | Word-wrap text to terminal width |
| pulldown-cmark + term-maths | 0.13 / 1.0 | Markdown math events and 2D Unicode LaTeX rendering |
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
cargo run -- /path/to/book.pdf
cargo test
```

## Key Bindings
- `←` / `→` or `h` / `l`: Previous / next page
- `t` or `T`: Open ToC (chapter selection)
- `b` or `B`: Open bookmarks panel
- `s`: Save or overwrite the bookmark at the current position
- `c`: Open Reading Copilot for the visible page
- `q` or `Esc`: Quit (or close popup)
- `j` / `k`: Scroll within popup lists

## Features
- **Pagination**: Text is reflowed to terminal dimensions on resize
- **Bookmarks**: One bookmark per book, stored in `~/.local/share/ink-reader/bookmarks.json`, with manual save on `s` and auto-save on quit
- **Chapter navigation**: Popup ToC with selectable chapters
- **Cover image**: Displayed on open for EPUB (manifest cover-image or id/href hint) and PDF (rendered first page)
- **Styled headings**: Lines emitted from `ContentBlock::Heading` keep their `#` / `##` markers and are colorized by level in `ui/reader.rs`; wrapped continuation lines inherit the same heading style until the following blank line
- **Inline references**: EPUB footnote/reference markers such as `[4]` or image-backed note icons are expanded inline and rendered in a subdued style
- **Inline illustrations**: EPUB chapter illustrations rendered in-place; SVG/unsupported images shown as `[Image: alt]` placeholder
- **Images**: Auto-detect terminal protocol; fallback to half-block if unsupported
- **PDF extraction**: Reflowed text, styled headings, structured tables, embedded images, XMP/Info metadata, native outline, page labels
- **Reading Copilot**: Page-scoped Rig Agent in a responsive right panel with explain/translate/summarize/deep-analysis/custom-question actions, streaming output, cancellation, local/remote privacy label
- **Formats**: EPUB, PDF, TXT

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
- Never render a terminal image behind a popup; synchronize terminal graphics and ratatui's diff buffer with a full clear on image/overlay transitions
- Never block the TUI event loop on Agent/provider IO or silently send book content to a remote endpoint
