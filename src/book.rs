use anyhow::Result;

/// A single chapter in the book
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Chapter {
    pub index: usize,
    pub title: String,
    /// Internal resource identifier (EPUB href, PDF page string, etc.)
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
    Heading { level: u8, text: String },
    Image { data: Vec<u8>, alt: String, mime: String },
    PageBreak,
}

/// Core trait: all format readers must implement this
pub trait BookReader: Send {
    fn meta(&self) -> &BookMeta;
    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>>;
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
}

/// Cache key for lazy pagination
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PaginationKey {
    pub chapter: usize,
    pub width: u16,
    pub height: u16,
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

    let flush = |pages: &mut Vec<Page>,
                     lines: &mut Vec<String>,
                     first: &mut usize,
                     img: &mut Option<PageImage>,
                     next_first: usize| {
        if !lines.is_empty() || img.is_some() {
            pages.push(Page {
                lines: std::mem::take(lines),
                image: img.take(),
                first_block: *first,
            });
            *first = next_first;
        }
    };

    for (block_idx, block) in blocks.iter().enumerate() {
        match block {
            ContentBlock::Paragraph(text) => {
                for line in wrap_paragraph(text, wrap_w) {
                    if cur_lines.len() >= page_h {
                        flush(&mut pages, &mut cur_lines, &mut cur_first, &mut cur_image, block_idx);
                    }
                    cur_lines.push(line);
                }
                // Blank line after each paragraph (spacing between blocks)
                if cur_lines.len() < page_h {
                    cur_lines.push(String::new());
                }
            }
            ContentBlock::Heading { level, text } => {
                let marker = "#".repeat((*level).clamp(1, 6) as usize);
                let heading = format!("{marker} {text}");
                // Blank line before heading
                if !cur_lines.is_empty() {
                    if cur_lines.len() >= page_h {
                        flush(&mut pages, &mut cur_lines, &mut cur_first, &mut cur_image, block_idx);
                    }
                    cur_lines.push(String::new());
                }
                for line in wrap_text(&heading, wrap_w) {
                    if cur_lines.len() >= page_h {
                        flush(&mut pages, &mut cur_lines, &mut cur_first, &mut cur_image, block_idx);
                    }
                    cur_lines.push(line);
                }
                // Blank line after heading
                if cur_lines.len() < page_h {
                    cur_lines.push(String::new());
                }
            }
            ContentBlock::Image { data, alt, mime } => {
                // Flush current text, then give image its own page
                flush(&mut pages, &mut cur_lines, &mut cur_first, &mut cur_image, block_idx);
                cur_image = Some(PageImage {
                    data: data.clone(),
                    mime: mime.clone(),
                    alt: alt.clone(),
                });
            }
            ContentBlock::PageBreak => {
                flush(&mut pages, &mut cur_lines, &mut cur_first, &mut cur_image, block_idx + 1);
            }
        }
    }

    // Flush remaining content
    if !cur_lines.is_empty() || cur_image.is_some() {
        pages.push(Page {
            lines: cur_lines,
            image: cur_image,
            first_block: cur_first,
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
        assert!(pages.len() > 1, "content should be split across multiple pages");
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
}
