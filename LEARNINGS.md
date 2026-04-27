# Project Learnings

> Append-only knowledge base maintained during issue processing.
> The agent reads this before starting each issue to avoid repeating mistakes.
> Human edits welcome — add, annotate, or mark as [OBSOLETE].

---

### L-001: [gotcha] ratatui-image Picker has no `new()` method (2025-01-01)
- **Issue**: #47 — 开发构建一个终端TUI电子书阅读器
- **Trigger**: ratatui-image, picker, image, terminal, protocol
- **Pattern**: `Picker::new()` doesn't exist in ratatui-image 10.x. The query-based constructor is `Picker::from_query_stdio()` (returns `Result`) and the infallible fallback is `Picker::halfblocks()`.
- **Evidence**: `~/.cargo/registry/src/.../ratatui-image-10.0.6/src/picker/mod.rs`
- **Confidence**: 10/10
- **Action**: Always use `Picker::from_query_stdio().ok().or_else(|| Some(Picker::halfblocks()))` pattern — never `Picker::new(...)`.

### L-002: [gotcha] mobi 0.8 `title()` returns String, not Option (2025-01-01)
- **Issue**: #47 — 开发构建一个终端TUI电子书阅读器
- **Trigger**: mobi, title, metadata, unwrap, Option
- **Pattern**: Unlike most metadata APIs, `mobi::Mobi::title()` returns `String` directly (not `Option<String>`). Calling `.unwrap_or_else()` on it is a compile error.
- **Evidence**: `~/.cargo/registry/src/.../mobi-0.8.0/src/lib.rs`
- **Confidence**: 10/10
- **Action**: Use `m.title()` directly; `m.author()` is the `Option<String>` field.

### L-003: [gotcha] pdf_oxide `page_count()` returns Result (2025-01-01)
- **Issue**: #47 — 开发构建一个终端TUI电子书阅读器
- **Trigger**: pdf_oxide, page_count, pdf, usize, infallible
- **Pattern**: `PdfDocument::page_count()` returns `Result<usize>`, not `usize`. Must use `?` operator. The infallible variant is `page_count_u32()` returning `u32`.
- **Evidence**: `~/.cargo/registry/src/.../pdf_oxide-0.3.37/src/lib.rs`
- **Confidence**: 10/10
- **Action**: Always append `?` when calling `doc.page_count()`; use `page_count_u32()` if you need an infallible count.

### L-004: [gotcha] sudo make + rustup shim needs HOME set (2025-01-01)
- **Issue**: #48 — 加入Makefile
- **Trigger**: sudo, Makefile, rustup, cargo, install, HOME
- **Pattern**: `sudo make install` fails with "rustup could not choose a version of cargo" because rustup's cargo shim reads HOME to find `~/.rustup/toolchains`, but `sudo` resets `HOME=/root`.
- **Evidence**: `cargo install` under sudo, error "one wasn't specified explicitly, and no default is configured"
- **Confidence**: 10/10
- **Action**: In Makefiles, detect `SUDO_USER` and prepend `HOME=$(REAL_HOME)` to every cargo invocation:
  ```makefile
  ifdef SUDO_USER
      REAL_HOME := $(shell eval echo ~$(SUDO_USER))
      CARGO := HOME=$(REAL_HOME) $(REAL_HOME)/.cargo/bin/cargo
  else
      CARGO := $(shell command -v cargo 2>/dev/null || echo $$HOME/.cargo/bin/cargo)
  endif
  ```

### L-005: [gotcha] textwrap AsciiSpace breaks CJK paragraph indent (2025-01-01)
- **Issue**: #50 — 提升阅读体验
- **Trigger**: textwrap, indent, CJK, Chinese, paragraph, initial_indent, wrap
- **Pattern**: `WordSeparator::AsciiSpace` treats an entire CJK string (no ASCII spaces) as a single "word". When that word's display width exceeds `wrap_width - indent_width`, textwrap emits just the indent on line 0 with no content, then the text on line 1 with no indent — making `initial_indent` appear non-functional.
- **Evidence**: `src/book.rs:183` — switching to `UnicodeBreakProperties` fixes the issue.
- **Confidence**: 9/10
- **Action**: Always use `WordSeparator::UnicodeBreakProperties` (not `AsciiSpace`) when the content may contain CJK or other non-space-separated scripts.

### L-006: [gotcha] rbook ManifestEntry::read_bytes() 可直接调用，无需 epub.read_resource_bytes() (2026-04-23)
- **Issue**: #52 — 不是支持图片的渲染吗？
- **Trigger**: epub, manifest, cover, image, read_bytes, rbook
- **Pattern**: `ManifestEntry` trait 自带 `read_bytes()` 方法，可在借用 `manifest` 期间直接读取资源字节，无需先提取 `href` 再调用 `epub.read_resource_bytes()`，从根本上规避了生命周期冲突问题。
- **Evidence**: `src/formats/epub.rs` `extract_cover()` 函数
- **Confidence**: 9/10
- **Action**: 直接用 `entry.read_bytes().ok()` 在同一 `{}` 块内完成读取；不要拆分成"取 href → 再读字节"两步。

### L-007: [convention] MOBI/AZW3 第一个图片记录即封面 (2026-04-23)
- **Issue**: #52 — 不是支持图片的渲染吗？
- **Trigger**: mobi, azw3, cover, image, image_records, first_image_index
- **Pattern**: `mobi` crate 的 `image_records()` 从 `first_image_index` 起过滤出所有图片记录，按 MOBI 格式惯例第一个即为封面图片。
- **Evidence**: `src/formats/mobi.rs` `MobiReader::open()`
- **Confidence**: 8/10
- **Action**: 用 `m.image_records().first()` 并复制 `r.content.to_vec()` 获取封面字节；`m` 须在整个操作期间保持存活。

### L-008: [architecture] Sentinel injection for html2text image extraction (2025-01-01)
- **Issue**: #53 — 书籍中的插图也要支持图片
- **Trigger**: epub, html2text, inline image, img tag, chapter_blocks, sentinel, placeholder
- **Pattern**: To extract images in document order from HTML processed by html2text: (1) scan raw HTML for `<img>` tags first to collect (src, alt) pairs; (2) replace each `<img>` tag with `</p><p>__INKIMG_N__</p><p>` before passing to html2text; (3) after html2text, split on `\n\n` and detect sentinel paragraphs to swap back for ContentBlock::Image. This works because html2text preserves unknown text through the paragraph-level delimiters.
- **Evidence**: `src/formats/epub.rs` `chapter_blocks()`, `extract_img_tags()`, `parse_img_sentinel()`
- **Confidence**: 9/10
- **Action**: Always scan img tags in a separate pass BEFORE injecting sentinels (same left-to-right order preserves index mapping). Defer `image::load_from_memory` decode to display time — validate only via magic bytes at chapter load to avoid decompression-bomb risk.

### L-009: [convention] Shared detect_image_mime in book.rs (2025-01-01)
- **Issue**: #53 — 书籍中的插图也要支持图片
- **Trigger**: detect_mime, image mime, magic bytes, image/jpeg, image/unknown
- **Pattern**: Magic-byte MIME detection should live in `book.rs` as `pub(crate) fn detect_image_mime(data: &[u8]) -> &'static str` and be shared by all format readers. The fallback must be `"image/unknown"`, not `"image/jpeg"` — returning jpeg for unknown bytes causes image::load_from_memory to fail with a confusing error on valid non-jpeg files.
- **Evidence**: `src/book.rs` `detect_image_mime()`, `src/formats/mobi.rs`
- **Confidence**: 9/10
- **Action**: Import `crate::book::detect_image_mime` in all format readers. Never write a local `detect_mime` again.

### L-010: [architecture] EPUB 顺序阅读必须跟随 spine，不是顶层 ToC (2026-04-24)
- **Issue**: #54 — 插图的图片还是不显示
- **Trigger**: epub, spine, toc, ncx, nested navPoint, section0001, chapter order
- **Pattern**: 对顺序阅读来说，canonical reading order 来自 EPUB spine。NCX/ToC 可用于导航和命名，但不能直接拿顶层 navPoint 当阅读序列；否则像 `Text/Section0001.xhtml#hh2-1` 这种嵌套目录对应的正文文档会被整段跳过。
- **Evidence**: `src/formats/epub.rs` `collect_chapters()`
- **Confidence**: 10/10
- **Action**: 章节列表先按 spine 生成，再用扁平化 ToC 的首个 label 为每个 XHTML 资源命名；读取资源前剥掉 `#fragment`。

### L-011: [gotcha] 图片页如果只渲染图片，会把 caption 和正文一起吞掉 (2026-04-24)
- **Issue**: #54 — 插图的图片还是不显示
- **Trigger**: image page, caption, figure title, ui render, Page.lines, ratatui-image
- **Pattern**: 如果分页允许 image page 同时携带 `Page.lines`，但 UI 在检测到图片后直接 early-return 只画图，那么紧随图片的 caption 甚至正文都会被视觉上“丢失”。
- **Evidence**: `src/book.rs` `paginate_blocks()` 与 `src/ui/reader.rs` `render_content()`
- **Confidence**: 9/10
- **Action**: 图片页要么只保留图片+caption 并在 UI 一起渲染，要么在分页阶段把后续正文显式拆到下一页，不能让 image page 隐式吞文本。

### L-012: [convention] 给 Rust 项目加 CI 前，先本地跑完整 gate 命令链 (2026-04-24)
- **Issue**: #55 — 加入github action
- **Trigger**: github actions, ci, clippy, fmt, cargo build, cargo test, workflow
- **Pattern**: 对 Rust 项目补 CI 时，真正的 blocker 往往不是 workflow YAML，而是仓库当前是否已经满足 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`cargo build --release` 这些 gate。先本地跑完整命令链，才能避免把已有 lint debt 直接“上传成红灯 CI”。
- **Evidence**: `src/app.rs` 与 `src/formats/epub.rs` 在加 workflow 前先修了 clippy blockers；`.github/workflows/ci.yml` 复用了同一套命令
- **Confidence**: 10/10
- **Action**: 设计 CI 时，先固定 gate 命令，再在本地连续跑通；只有命令链本地干净通过后，再把它们写进 workflow。

### L-013: [convention] 删除格式支持时先切断扩展分派入口 (2026-04-24)
- **Issue**: #56 — 删除mobi的支持
- **Trigger**: format support, extension dispatch, load_reader, unsupported file format, reader reuse
- **Pattern**: 当某个文件格式要退役，但底层 parser 仍服务同一格式族的其他扩展时，应先在 `load_reader()` 的扩展匹配层移除入口，而不是直接删除 parser 模块或 crate；并补一个针对该扩展的 Unsupported 回归测试来锁定行为。
- **Evidence**: `src/formats/mod.rs`
- **Confidence**: 9/10
- **Action**: 以后做格式退役，先检查 parser 是否仍被其他扩展复用；若是，只改扩展分派并补回归测试。

### L-014: [convention] 收缩支持矩阵时要同步删掉孤儿模块和依赖 (2026-04-24)
- **Issue**: #57 — 删除PDF和azw3的支持
- **Trigger**: support matrix, dependency pruning, parser module, format retirement, doc alignment
- **Pattern**: 当产品支持的文件格式收缩到更小集合时，不能只改扩展分派；还要同步删除失去入口的 parser 模块、crate 依赖，以及 README/AGENTS 等对外和对内知识源里的对应描述，避免代码、依赖和文档三方漂移。
- **Evidence**: `Cargo.toml`, `src/formats/mod.rs`, `README.md`, `AGENTS.md`
- **Confidence**: 9/10
- **Action**: 以后做格式退役或功能下线，按“入口 → 模块 → 依赖 → 文档 → 回归测试”顺序完整收口。

### L-015: [convention] 书签必须持久化逻辑位置，不是页号或固定 0 (2026-04-24)
- **Issue**: #60 — 修复书签的BUG
- **Trigger**: bookmark, page restore, first_block, resize, book_path
- **Pattern**: 书签如果只保存 `chapter`、或把 `block_index` 写死成 `0`，跳转时就只能回到章节开头，无法回到原 page。正确做法是用真实 `book_path` 作为书籍键，并持久化当前页的 `Page.first_block`；跳转时在重新分页后的 `pages` 里按 `first_block` 解析回目标页，这样 resize 后也能稳定恢复阅读位置。
- **Evidence**: `src/app.rs:169-171`, `src/app.rs:286-294`, `src/app.rs:436-467`, `src/ui/bookmarks.rs:10-18`
- **Confidence**: 10/10
- **Action**: 以后改书签/阅读进度时，一律保存 `chapter + first_block + canonical book_path`，不要保存瞬时页号，也不要在 UI 和存储层各自发明不同的 book key。

### L-016: [gotcha] EPUB 脚注目标常是 `dl/dt/dd`，不是普通段落 (2026-04-27)
- **Issue**: #62 — 阅读体验增强
- **Trigger**: epub, footnote, noteref, dl, dd, note_x, reference
- **Pattern**: 有些 EPUB 的正文脚注链接会从 `<sup><a class="noteref">2</a></sup>` 指向 `<dl id="note_2" class="footnote"><dt>回跳</dt><dd>正文</dd></dl>`。如果只按 `p/li/aside` 去抓目标，会漏掉真实书籍；如果直接吃整个 `dl`，又会把 `dt` 里的回跳箭头一起带进正文。
- **Evidence**: `src/formats/epub.rs` 中 `preferred_reference_fragment()` 与 `inlines_definition_list_footnotes()`
- **Confidence**: 9/10
- **Action**: 以后做 EPUB 引用/脚注解析时，优先读取目标 `dd` 正文；`dl` 只能当容器，不能把整个 `dt/dd` 一起塞回正文。

### L-017: [gotcha] 多字符内联标记会被 textwrap 拆烂，渲染层应改用单字符哨兵 (2026-04-27)
- **Issue**: #63 — 脚注内联展开的优化
- **Trigger**: inline reference, textwrap, pagination, wrap, marker, render, sentinel
- **Pattern**: 把内联引用先编码成 `{{ ... }}` 这种多字符文本标记，再交给 `textwrap` 分页，会被拆成 `{    { ... }}` 之类的碎片，导致渲染层既看见原始标记又无法稳定上样式。根治办法不是继续修花括号解析，而是改成单字符哨兵，再由渲染层把哨兵映射成真实 UI 表现。
- **Evidence**: `src/book.rs` 的 `INLINE_REF_OPEN/CLOSE`，`src/ui/reader.rs` 的 `stylize_inline_reference_lines()`
- **Confidence**: 10/10
- **Action**: 以后做分页后仍需二次渲染的内联语义（脚注、批注、高亮）时，不要把语义编码成多字符可见文本标记；优先用单字符哨兵或结构化数据。
