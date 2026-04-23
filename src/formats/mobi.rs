use std::path::Path;

use anyhow::Result;
use mobi::Mobi;

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock};

/// Reader for MOBI / AZW / AZW3 files.
/// MOBI has no native ToC support in the `mobi` crate; the entire
/// content is treated as one chapter.
pub struct MobiReader {
    meta: BookMeta,
    text: String,
}

impl MobiReader {
    pub fn open(path: &Path) -> Result<Self> {
        let m = Mobi::from_path(path)?;

        let title = m.title();
        let author = m.author();

        // Decode content — the crate returns HTML
        let html = m.content_as_string().unwrap_or_default();
        let text = html2text::from_read(html.as_bytes(), 80)
            .unwrap_or_else(|_| html.clone());

        let meta = BookMeta {
            title: title.clone(),
            author,
            chapters: vec![Chapter {
                index: 0,
                title,
                resource_id: path.to_string_lossy().into_owned(),
            }],
        };

        Ok(Self { meta, text })
    }
}

impl BookReader for MobiReader {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
        if chapter_idx != 0 {
            return Ok(vec![]);
        }
        let blocks: Vec<ContentBlock> = self
            .text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| ContentBlock::Paragraph(s.to_string()))
            .collect();

        if blocks.is_empty() {
            Ok(vec![ContentBlock::Paragraph("[Empty book]".to_string())])
        } else {
            Ok(blocks)
        }
    }
}
