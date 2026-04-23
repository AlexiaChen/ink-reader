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
