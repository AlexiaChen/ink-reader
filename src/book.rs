use anyhow::Result;

pub(crate) const INLINE_REF_OPEN: char = '\u{E000}';
pub(crate) const INLINE_REF_CLOSE: char = '\u{E001}';

/// A single chapter in the book
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Chapter {
    pub index: usize,
    pub title: String,
    /// Internal resource identifier (EPUB href, source path, etc.)
    pub resource_id: String,
}

/// Top-level book metadata
#[derive(Debug, Clone)]
pub struct BookMeta {
    pub title: String,
    #[allow(dead_code)]
    pub author: Option<String>,
    pub chapters: Vec<Chapter>,
}

/// A structured block of content within a chapter
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ContentBlock {
    Paragraph(String),
    Heading {
        level: u8,
        text: String,
    },
    SectionMarker(String),
    Image {
        data: Vec<u8>,
        alt: String,
        mime: String,
    },
    PageBreak,
}

/// Core trait: all format readers must implement this
pub trait BookReader: Send {
    fn meta(&self) -> &BookMeta;
    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>>;
    /// Return the book cover image as `(bytes, mime_type)`, if available.
    fn cover_image(&self) -> Option<(&[u8], &str)> {
        None
    }
}

/// An image on a rendered page
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PageImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub alt: String,
}

/// A rendered page: wrapped text lines + optional image
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub lines: Vec<String>,
    #[allow(dead_code)]
    pub image: Option<PageImage>,
    /// Logical position: index of the first ContentBlock on this page
    #[allow(dead_code)]
    pub first_block: usize,
    pub section_title: Option<String>,
}

/// Cache key for lazy pagination
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PaginationKey {
    pub chapter: usize,
    pub width: u16,
    pub height: u16,
}

/// Detect image MIME type from magic bytes.
/// Returns `"image/unknown"` for unrecognised formats (e.g. SVG, broken data).
pub(crate) fn detect_image_mime(data: &[u8]) -> &'static str {
    if data.starts_with(b"\xFF\xD8") {
        "image/jpeg"
    } else if data.starts_with(b"\x89PNG") {
        "image/png"
    } else if data.starts_with(b"GIF8") {
        "image/gif"
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/unknown"
    }
}

/// Paginate content blocks into pages that fit the terminal.
/// Reserves 3 lines for status bar + help bar.
pub fn paginate_blocks(blocks: &[ContentBlock], width: u16, height: u16) -> Vec<Page> {
    let page_h = (height.saturating_sub(3)).max(1) as usize;
    let wrap_w = (width as usize).max(10);

    let mut pages: Vec<Page> = Vec::new();
    let mut cur_lines: Vec<String> = Vec::new();
    let mut cur_first: usize = 0;
    let mut cur_image: Option<PageImage> = None;
    let mut active_section_title: Option<String> = None;
    let mut page_section_title: Option<String> = None;

    let flush = |pages: &mut Vec<Page>,
                 lines: &mut Vec<String>,
                 first: &mut usize,
                 img: &mut Option<PageImage>,
                 section_title: &mut Option<String>,
                 next_first: usize,
                 next_section_title: Option<String>| {
        if !lines.is_empty() || img.is_some() {
            pages.push(Page {
                lines: std::mem::take(lines),
                image: img.take(),
                first_block: *first,
                section_title: section_title.clone(),
            });
            *first = next_first;
            *section_title = next_section_title;
        }
    };

    let mut block_idx = 0usize;
    while let Some(block) = blocks.get(block_idx) {
        match block {
            ContentBlock::Paragraph(text) => {
                for line in wrap_paragraph(text, wrap_w) {
                    if cur_lines.len() >= page_h {
                        flush(
                            &mut pages,
                            &mut cur_lines,
                            &mut cur_first,
                            &mut cur_image,
                            &mut page_section_title,
                            block_idx,
                            active_section_title.clone(),
                        );
                    }
                    cur_lines.push(line);
                }
                // Blank line after each paragraph (spacing between blocks)
                if cur_lines.len() < page_h {
                    cur_lines.push(String::new());
                }
                block_idx += 1;
            }
            ContentBlock::Heading { level, text } => {
                let marker = "#".repeat((*level).clamp(1, 6) as usize);
                let heading = format!("{marker} {text}");
                // Blank line before heading
                if !cur_lines.is_empty() {
                    if cur_lines.len() >= page_h {
                        flush(
                            &mut pages,
                            &mut cur_lines,
                            &mut cur_first,
                            &mut cur_image,
                            &mut page_section_title,
                            block_idx,
                            active_section_title.clone(),
                        );
                    }
                    cur_lines.push(String::new());
                }
                for line in wrap_text(&heading, wrap_w) {
                    if cur_lines.len() >= page_h {
                        flush(
                            &mut pages,
                            &mut cur_lines,
                            &mut cur_first,
                            &mut cur_image,
                            &mut page_section_title,
                            block_idx,
                            active_section_title.clone(),
                        );
                    }
                    cur_lines.push(line);
                }
                // Blank line after heading
                if cur_lines.len() < page_h {
                    cur_lines.push(String::new());
                }
                block_idx += 1;
            }
            ContentBlock::SectionMarker(title) => {
                active_section_title = Some(title.clone());
                page_section_title = Some(title.clone());
                block_idx += 1;
            }
            ContentBlock::Image { data, alt, mime } => {
                // Flush current text, then give image its own page.
                flush(
                    &mut pages,
                    &mut cur_lines,
                    &mut cur_first,
                    &mut cur_image,
                    &mut page_section_title,
                    block_idx,
                    active_section_title.clone(),
                );

                let mut caption_lines = Vec::new();
                let mut next_idx = block_idx + 1;
                let mut saw_primary_caption = false;

                while let Some(text) = blocks
                    .get(next_idx)
                    .and_then(|next| image_caption_text(next, saw_primary_caption))
                {
                    saw_primary_caption = true;
                    caption_lines.extend(wrap_text(&text, wrap_w));
                    caption_lines.push(String::new());
                    next_idx += 1;
                }

                if matches!(caption_lines.last(), Some(last) if last.is_empty()) {
                    caption_lines.pop();
                }

                pages.push(Page {
                    lines: caption_lines,
                    image: Some(PageImage {
                        data: data.clone(),
                        mime: mime.clone(),
                        alt: alt.clone(),
                    }),
                    first_block: block_idx,
                    section_title: active_section_title.clone(),
                });
                cur_first = next_idx;
                page_section_title = active_section_title.clone();
                block_idx = next_idx;
            }
            ContentBlock::PageBreak => {
                flush(
                    &mut pages,
                    &mut cur_lines,
                    &mut cur_first,
                    &mut cur_image,
                    &mut page_section_title,
                    block_idx + 1,
                    active_section_title.clone(),
                );
                block_idx += 1;
            }
        }
    }

    // Flush remaining content
    if !cur_lines.is_empty() || cur_image.is_some() {
        pages.push(Page {
            lines: cur_lines,
            image: cur_image,
            first_block: cur_first,
            section_title: page_section_title,
        });
    }

    // Always return at least one (possibly empty) page
    if pages.is_empty() {
        pages.push(Page::default());
    }

    pages
}

/// Wrap text to fit within `width` columns, sanitizing control characters.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    wrap_with_opts(text, width, "", "")
}

/// Wrap a paragraph with a 4-space first-line indent (段落首行缩进).
/// Uses textwrap's `initial_indent` so the first-line content is correctly
/// limited to `width - 4` characters, avoiding overflow.
fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    wrap_with_opts(text, width, "    ", "")
}

fn wrap_with_opts(text: &str, width: usize, initial: &str, subsequent: &str) -> Vec<String> {
    // Sanitize: strip control chars that could inject terminal escape sequences
    let sanitized: String = text
        .chars()
        .filter(|&c| c >= ' ' || c == '\n' || c == '\t')
        .collect::<String>()
        .replace('\t', "    ");

    let options = textwrap::Options::new(width)
        .initial_indent(initial)
        .subsequent_indent(subsequent)
        .break_words(true)
        .word_separator(textwrap::WordSeparator::UnicodeBreakProperties);

    let mut result = Vec::new();
    for line in sanitized.split('\n') {
        let wrapped = textwrap::wrap(line.trim_end(), &options);
        if wrapped.is_empty() {
            result.push(String::new());
        } else {
            result.extend(wrapped.into_iter().map(|s| s.to_string()));
        }
    }
    result
}

fn image_caption_text(block: &ContentBlock, saw_primary_caption: bool) -> Option<String> {
    let text = match block {
        ContentBlock::Paragraph(text) => normalize_image_caption(text),
        ContentBlock::Heading { text, .. } => normalize_image_caption(text),
        _ => return None,
    };

    if text.is_empty() {
        return None;
    }

    if !saw_primary_caption {
        is_primary_image_caption(&text).then_some(text)
    } else {
        is_secondary_image_caption(&text).then_some(text)
    }
}

fn normalize_image_caption(text: &str) -> String {
    text.trim().trim_start_matches('#').trim().to_string()
}

fn is_primary_image_caption(text: &str) -> bool {
    looks_like_cjk_caption_label(text, '图')
        || looks_like_cjk_caption_label(text, '表')
        || looks_like_ascii_caption_prefix(text)
}

fn is_secondary_image_caption(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('(')
        || trimmed.starts_with('（')
        || trimmed.starts_with('[')
        || trimmed.starts_with("来源")
        || trimmed.to_ascii_lowercase().starts_with("source")
}

fn looks_like_cjk_caption_label(text: &str, prefix: char) -> bool {
    let Some(rest) = text.strip_prefix(prefix) else {
        return false;
    };

    let rest = rest.trim_start_matches([' ', '　']);
    matches!(
        rest.chars().next(),
        Some(c)
            if c.is_ascii_digit()
                || matches!(
                    c,
                    '０'..='９'
                        | '〇'
                        | '零'
                        | '一'
                        | '二'
                        | '三'
                        | '四'
                        | '五'
                        | '六'
                        | '七'
                        | '八'
                        | '九'
                        | '十'
                        | '百'
                        | '千'
                        | '甲'
                        | '乙'
                        | '丙'
                        | '丁'
                        | '上'
                        | '中'
                        | '下'
                )
    )
}

fn looks_like_ascii_caption_prefix(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "figure ", "figure:", "fig. ", "fig ", "table ", "table:", "image ", "map ", "plate ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paginate_empty_blocks() {
        let pages = paginate_blocks(&[], 80, 24);
        assert_eq!(pages.len(), 1, "empty input must yield one page");
    }

    #[test]
    fn paginate_respects_height() {
        // 20 paragraphs, terminal height 10 (page_h = 7 after subtracting 3)
        let blocks: Vec<ContentBlock> = (0..20)
            .map(|i| ContentBlock::Paragraph(format!("Line {i}")))
            .collect();
        let pages = paginate_blocks(&blocks, 80, 10);
        // Each paragraph is one line; page_h = 7
        assert!(
            pages.len() > 1,
            "content should be split across multiple pages"
        );
    }

    #[test]
    fn paginate_handles_zero_height() {
        let blocks = vec![ContentBlock::Paragraph("Hello".to_string())];
        // height=0 should not panic
        let pages = paginate_blocks(&blocks, 80, 0);
        assert!(!pages.is_empty());
    }

    #[test]
    fn paragraph_indent_ascii() {
        let blocks = vec![ContentBlock::Paragraph(
            "The quick brown fox jumps over the lazy dog.".to_string(),
        )];
        let pages = paginate_blocks(&blocks, 80, 24);
        let first_line = &pages[0].lines[0];
        assert!(
            first_line.starts_with("    "),
            "paragraph first line must have 4-space indent, got: {:?}",
            first_line
        );
    }

    #[test]
    fn paragraph_indent_cjk() {
        let blocks = vec![ContentBlock::Paragraph(
            "这是一个中文段落，需要正确地缩进首行显示。".to_string(),
        )];
        let pages = paginate_blocks(&blocks, 40, 24);
        let first_line = &pages[0].lines[0];
        assert!(
            first_line.starts_with("    "),
            "CJK paragraph first line must have 4-space indent, got: {:?}",
            first_line
        );
    }

    #[test]
    fn image_page_keeps_following_caption_blocks() {
        let blocks = vec![
            ContentBlock::Image {
                data: vec![1, 2, 3],
                alt: String::new(),
                mime: "image/jpeg".to_string(),
            },
            ContentBlock::Paragraph("#### 图1 安史之乱前期河南节度使所辖十三州".to_string()),
            ContentBlock::Paragraph("##### （此图以《中国历史地图集》为底图改绘）".to_string()),
            ContentBlock::Paragraph("这里才是后续正文。".to_string()),
        ];

        let pages = paginate_blocks(&blocks, 80, 24);
        assert!(
            pages[0].image.is_some(),
            "first page should still be an image page"
        );
        assert!(
            pages[0]
                .lines
                .iter()
                .any(|line| line.contains("图1 安史之乱前期河南节度使所辖十三州")),
            "image page should keep the primary caption, got: {:?}",
            pages[0].lines
        );
        assert!(
            pages[0].lines.iter().any(|line| line.contains("底图改绘")),
            "image page should keep the secondary caption, got: {:?}",
            pages[0].lines
        );
        assert!(
            pages.iter().skip(1).any(|page| page
                .lines
                .iter()
                .any(|line| line.contains("这里才是后续正文"))),
            "body text must remain after the image page"
        );
    }

    #[test]
    fn regular_paragraph_after_image_is_not_treated_as_caption() {
        let blocks = vec![
            ContentBlock::Image {
                data: vec![1, 2, 3],
                alt: String::new(),
                mime: "image/jpeg".to_string(),
            },
            ContentBlock::Paragraph("这里是普通正文，不是图注。".to_string()),
        ];

        let pages = paginate_blocks(&blocks, 80, 24);
        assert!(pages[0].image.is_some());
        assert!(
            pages[0].lines.is_empty(),
            "plain body text must not stay on the image page: {:?}",
            pages[0].lines
        );
        assert!(
            pages.iter().skip(1).any(|page| page
                .lines
                .iter()
                .any(|line| line.contains("这里是普通正文"))),
            "plain body text should remain in later text pages"
        );
    }

    #[test]
    fn image_caption_text_strips_heading_markers() {
        let block =
            ContentBlock::Paragraph("#### 图1 安史之乱前期河南节度使所辖十三州".to_string());
        assert_eq!(
            image_caption_text(&block, false).as_deref(),
            Some("图1 安史之乱前期河南节度使所辖十三州")
        );
    }

    #[test]
    fn section_markers_assign_page_titles() {
        let blocks = vec![
            ContentBlock::SectionMarker("序章".to_string()),
            ContentBlock::Paragraph("这一页仍在序章。".to_string()),
            ContentBlock::PageBreak,
            ContentBlock::SectionMarker("第一章".to_string()),
            ContentBlock::Paragraph("这里已经进入第一章。".to_string()),
        ];

        let pages = paginate_blocks(&blocks, 80, 10);
        assert_eq!(pages[0].section_title.as_deref(), Some("序章"));
        assert_eq!(pages[1].section_title.as_deref(), Some("第一章"));
    }
}
