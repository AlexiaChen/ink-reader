use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use pdf_oxide::PdfDocument;
use pdf_oxide::converters::ConversionOptions;
use pdf_oxide::extractors::page_labels::{PageLabelExtractor, PageLabelRange};
use pdf_oxide::extractors::xmp::XmpExtractor;
use pdf_oxide::object::Object;
use pdf_oxide::outline::{Destination, OutlineItem};
use pdf_oxide::rendering::{RenderOptions, render_page};

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock, TocEntry, detect_image_mime};

/// PDF reader backed by pdf_oxide.
///
/// Each source PDF page is exposed as a logical chapter. This preserves exact
/// source-page navigation while allowing the existing terminal paginator to
/// reflow extracted text for the current viewport.
pub struct PdfReader {
    document: PdfDocument,
    meta: BookMeta,
    toc: Option<Vec<TocEntry>>,
    cover: Option<(Vec<u8>, String)>,
}

impl PdfReader {
    pub fn open(path: &Path) -> Result<Self> {
        let document = PdfDocument::open(path)
            .with_context(|| format!("failed to open PDF: {}", path.display()))?;
        let page_count = document
            .page_count()
            .context("failed to read PDF page count")?;
        if page_count == 0 {
            anyhow::bail!("PDF contains no pages");
        }

        let xmp = XmpExtractor::extract(&document).ok().flatten();
        let fallback_title = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("Untitled")
            .to_string();
        let title = xmp
            .as_ref()
            .and_then(|metadata| metadata.dc_title.as_deref())
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
            .or_else(|| document_info_string(&document, "Title"))
            .unwrap_or(fallback_title);
        let xmp_author = xmp.as_ref().and_then(|metadata| {
            let creators: Vec<&str> = metadata
                .dc_creator
                .iter()
                .map(String::as_str)
                .map(str::trim)
                .filter(|creator| !creator.is_empty())
                .collect();
            (!creators.is_empty()).then(|| creators.join(", "))
        });
        let author = xmp_author.or_else(|| document_info_string(&document, "Author"));

        let outline = document.get_outline().ok().flatten().unwrap_or_default();
        let toc = build_toc_entries(&document, &outline, page_count);
        let page_titles: HashMap<usize, String> = toc
            .iter()
            .filter_map(|entry| {
                let title = entry.title.trim().to_string();
                (!title.is_empty()).then_some((entry.chapter, title))
            })
            .collect();
        let page_label_ranges = PageLabelExtractor::extract(&document).unwrap_or_default();
        let chapters = (0..page_count)
            .map(|page| Chapter {
                index: page,
                title: page_titles
                    .get(&page)
                    .cloned()
                    .unwrap_or_else(|| page_title(page, &page_label_ranges)),
                resource_id: format!("pdf-page:{page}"),
            })
            .collect();

        // A PDF has no distinct cover resource. Its first page is the cover in
        // reader terms, so render it to PNG with pdf_oxide's pure-Rust renderer.
        // Failure is optional: text reading still works without image support.
        let cover = render_page(&document, 0, &RenderOptions::with_dpi(96))
            .ok()
            .and_then(|rendered| {
                (!rendered.data.is_empty()).then_some((rendered.data, "image/png".to_string()))
            });

        Ok(Self {
            document,
            meta: BookMeta {
                title,
                author,
                chapters,
            },
            toc: (!toc.is_empty()).then_some(toc),
            cover,
        })
    }

    fn page_blocks(&self, page: usize) -> Result<Vec<ContentBlock>> {
        // The default conversion options enable heading detection, structure-
        // tree-first reading order, form values, and structured/spatial table
        // extraction. Images are extracted separately below so they can reuse
        // the terminal's existing raw ContentBlock::Image path.
        let markdown = self
            .document
            .to_markdown(page, &ConversionOptions::default())
            .or_else(|_| self.document.extract_text(page))
            .with_context(|| format!("failed to extract PDF page {}", page + 1))?;
        let mut blocks = markdown_to_blocks(&markdown);

        // pdf_oxide normalizes JPEG and raw PDF pixel buffers to a decodable
        // PNG. A failed individual image never makes the text page unreadable.
        if let Ok(images) = self.document.extract_images(page) {
            for (index, image) in images.into_iter().enumerate() {
                let Ok(data) = image.to_png_bytes() else {
                    continue;
                };
                if data.is_empty() || detect_image_mime(&data) == "image/unknown" {
                    continue;
                }
                blocks.push(ContentBlock::Image {
                    data,
                    alt: format!(
                        "PDF page {} image {} ({}x{})",
                        page + 1,
                        index + 1,
                        image.width(),
                        image.height()
                    ),
                    mime: "image/png".to_string(),
                });
            }
        }

        if blocks.is_empty() {
            blocks.push(ContentBlock::Paragraph("[Empty PDF page]".to_string()));
        }
        Ok(blocks)
    }
}

fn document_info_string(document: &PdfDocument, key: &str) -> Option<String> {
    fn resolve(document: &PdfDocument, object: &Object) -> Option<Object> {
        match object {
            Object::Reference(reference) => document.load_object(*reference).ok(),
            _ => Some(object.clone()),
        }
    }

    let info = document.trailer().as_dict()?.get("Info")?;
    let info = resolve(document, info)?;
    let value = info.as_dict()?.get(key)?;
    let value = resolve(document, value)?;
    let decoded = pdf_oxide::optional_content::decode_pdf_text_string(value.as_string()?);
    let decoded = decoded.trim();
    (!decoded.is_empty()).then(|| decoded.to_string())
}

fn page_title(page: usize, ranges: &[PageLabelRange]) -> String {
    let label = ranges
        .iter()
        .filter(|range| range.start_page <= page)
        .max_by_key(|range| range.start_page)
        .map(|range| range.format_label(page))
        .filter(|label| !label.is_empty());

    label
        .map(|label| format!("Page {label}"))
        .unwrap_or_else(|| format!("Page {}", page + 1))
}

impl BookReader for PdfReader {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
        if chapter_idx >= self.meta.chapters.len() {
            return Ok(Vec::new());
        }
        self.page_blocks(chapter_idx)
    }

    fn toc_entries(&self) -> Option<&[TocEntry]> {
        self.toc.as_deref()
    }

    fn cover_image(&self) -> Option<(&[u8], &str)> {
        self.cover
            .as_ref()
            .map(|(data, mime)| (data.as_slice(), mime.as_str()))
    }
}

fn build_toc_entries(
    document: &PdfDocument,
    outline: &[OutlineItem],
    page_count: usize,
) -> Vec<TocEntry> {
    fn visit(
        document: &PdfDocument,
        items: &[OutlineItem],
        page_count: usize,
        depth: usize,
        out: &mut Vec<TocEntry>,
    ) {
        for item in items {
            let chapter = match &item.dest {
                Some(Destination::PageIndex(page)) => Some(*page),
                Some(Destination::Named(name)) => {
                    document.resolve_named_destination(name).ok().flatten()
                }
                None => None,
            };
            let title = item.title.trim();
            if let Some(chapter) = chapter
                && chapter < page_count
                && !title.is_empty()
            {
                out.push(TocEntry {
                    title: format!("{}{}", "  ".repeat(depth), title),
                    chapter,
                });
            }
            visit(document, &item.children, page_count, depth + 1, out);
        }
    }

    let mut entries = Vec::new();
    visit(document, outline, page_count, 0, &mut entries);
    entries
}

/// Convert pdf_oxide's semantic Markdown into the reader's existing blocks.
/// GFM tables and lists remain multi-line paragraphs, while headings become
/// styled `ContentBlock::Heading` values consumed by the current UI.
fn markdown_to_blocks(markdown: &str) -> Vec<ContentBlock> {
    fn flush_paragraph(lines: &mut Vec<&str>, blocks: &mut Vec<ContentBlock>) {
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n");
        let text = text.trim();
        if !text.is_empty() {
            blocks.push(ContentBlock::Paragraph(text.to_string()));
        }
        lines.clear();
    }

    let mut blocks = Vec::new();
    let mut paragraph = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            continue;
        }

        if let Some((level, text)) = parse_markdown_heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(ContentBlock::Heading { level, text });
        } else {
            paragraph.push(line.trim_end());
        }
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

fn parse_markdown_heading(line: &str) -> Option<(u8, String)> {
    let marker_count = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&marker_count) {
        return None;
    }
    let text = line.get(marker_count..)?.strip_prefix(' ')?.trim();
    let text = text.trim_end_matches('#').trim();
    (!text.is_empty()).then(|| (marker_count as u8, text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_oxide::api::PdfBuilder;

    #[test]
    fn markdown_headings_and_tables_become_reader_blocks() {
        let blocks = markdown_to_blocks(
            "# Report\n\nIntro text.\n\n| Name | Value |\n| --- | --- |\n| A | 42 |",
        );

        assert!(matches!(
            &blocks[0],
            ContentBlock::Heading { level: 1, text } if text == "Report"
        ));
        assert!(matches!(&blocks[1], ContentBlock::Paragraph(text) if text == "Intro text."));
        assert!(matches!(&blocks[2], ContentBlock::Paragraph(text) if text.contains("| A | 42 |")));
    }

    #[test]
    fn opens_generated_pdf_and_extracts_text() {
        let file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        let mut pdf = PdfBuilder::new()
            .title("PDF Test")
            .author("Ink Reader")
            .from_markdown("# Hello PDF\n\nReadable body text.")
            .unwrap();
        pdf.save(file.path()).unwrap();

        let reader = PdfReader::open(file.path()).unwrap();
        assert_eq!(reader.meta().title, "PDF Test");
        assert_eq!(reader.meta().author.as_deref(), Some("Ink Reader"));
        assert!(!reader.meta().chapters.is_empty());
        let blocks = reader.chapter_blocks(0).unwrap();
        assert!(blocks.iter().any(|block| match block {
            ContentBlock::Heading { text, .. } | ContentBlock::Paragraph(text) => {
                text.contains("Hello PDF") || text.contains("Readable body text")
            }
            _ => false,
        }));
        assert!(reader.cover_image().is_some());
    }

    #[test]
    fn out_of_range_page_is_empty() {
        let file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        let mut pdf = PdfBuilder::new().from_markdown("one page").unwrap();
        pdf.save(file.path()).unwrap();
        let reader = PdfReader::open(file.path()).unwrap();

        assert!(reader.chapter_blocks(usize::MAX).unwrap().is_empty());
    }

    #[test]
    fn extracts_embedded_images_as_png_blocks() {
        let image_file = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        let pixels = image::RgbImage::from_pixel(32, 24, image::Rgb([25, 125, 225]));
        pixels.save(image_file.path()).unwrap();

        let pdf_file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        let mut pdf = pdf_oxide::api::Pdf::from_image(image_file.path()).unwrap();
        pdf.save(pdf_file.path()).unwrap();

        let reader = PdfReader::open(pdf_file.path()).unwrap();
        let blocks = reader.chapter_blocks(0).unwrap();
        assert!(blocks.iter().any(|block| matches!(
            block,
            ContentBlock::Image { data, mime, .. }
                if mime == "image/png" && detect_image_mime(data) == "image/png"
        )));
    }

    #[test]
    fn nested_outline_entries_keep_depth_and_page_targets() {
        let file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        let mut pdf = PdfBuilder::new().from_markdown("outline target").unwrap();
        pdf.save(file.path()).unwrap();
        let document = PdfDocument::open(file.path()).unwrap();
        let outline = vec![OutlineItem {
            title: "Part One".to_string(),
            dest: Some(Destination::PageIndex(0)),
            children: vec![OutlineItem {
                title: "Section A".to_string(),
                dest: Some(Destination::PageIndex(0)),
                children: Vec::new(),
            }],
        }];

        let entries = build_toc_entries(&document, &outline, 1);
        assert_eq!(
            entries[0],
            TocEntry {
                title: "Part One".to_string(),
                chapter: 0,
            }
        );
        assert_eq!(
            entries[1],
            TocEntry {
                title: "  Section A".to_string(),
                chapter: 0,
            }
        );
    }
}
