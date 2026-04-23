use std::path::Path;

use anyhow::Result;
use pdf_oxide::PdfDocument;

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock};

/// Reader for PDF files.
/// Each PDF page becomes one chapter for simple navigation.
pub struct PdfReader {
    meta: BookMeta,
    page_texts: Vec<String>,
}

impl PdfReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut doc = PdfDocument::open(path.to_str().unwrap_or(""))?;

        let page_count = doc.page_count()?;
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let mut chapters = Vec::with_capacity(page_count);
        let mut page_texts = Vec::with_capacity(page_count);

        for i in 0..page_count {
            let text = doc.extract_text(i).unwrap_or_default();
            chapters.push(Chapter {
                index: i,
                title: format!("Page {}", i + 1),
                resource_id: i.to_string(),
            });
            page_texts.push(text);
        }

        if chapters.is_empty() {
            chapters.push(Chapter {
                index: 0,
                title: "Page 1".to_string(),
                resource_id: "0".to_string(),
            });
            page_texts.push(String::new());
        }

        let meta = BookMeta {
            title,
            author: None,
            chapters,
        };

        Ok(Self { meta, page_texts })
    }
}

impl BookReader for PdfReader {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
        let text = match self.page_texts.get(chapter_idx) {
            Some(t) => t,
            None => return Ok(vec![]),
        };

        let blocks: Vec<ContentBlock> = text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| ContentBlock::Paragraph(s.to_string()))
            .collect();

        if blocks.is_empty() {
            Ok(vec![ContentBlock::Paragraph("[Empty page]".to_string())])
        } else {
            Ok(blocks)
        }
    }
}
