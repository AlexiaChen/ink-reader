use std::path::Path;

use anyhow::Result;
use mobi::Mobi;

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock, detect_image_mime};

/// Reader for MOBI / AZW / AZW3 files.
/// MOBI has no native ToC support in the `mobi` crate; the entire
/// content is treated as one chapter.
pub struct MobiReader {
    meta: BookMeta,
    text: String,
    cover: Option<(Vec<u8>, String)>,
}

impl MobiReader {
    pub fn open(path: &Path) -> Result<Self> {
        let m = Mobi::from_path(path)?;

        let title = m.title();
        let author = m.author();

        // Decode content — the crate returns HTML
        let html = m.content_as_string().unwrap_or_default();
        let text = html2text::from_read(html.as_bytes(), 80).unwrap_or_else(|_| html.clone());

        // Extract cover image: first image record is conventionally the cover
        let cover = {
            let records = m.image_records();
            records.first().and_then(|r| {
                let data = r.content.to_vec();
                let mime = detect_image_mime(&data);
                if mime == "image/unknown" {
                    return None;
                }
                image::load_from_memory(&data)
                    .ok()
                    .map(|_| (data, mime.to_string()))
            })
        };

        let meta = BookMeta {
            title: title.clone(),
            author,
            chapters: vec![Chapter {
                index: 0,
                title,
                resource_id: path.to_string_lossy().into_owned(),
            }],
        };

        Ok(Self { meta, text, cover })
    }
}

impl BookReader for MobiReader {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn cover_image(&self) -> Option<(&[u8], &str)> {
        self.cover.as_ref().map(|(d, m)| (d.as_slice(), m.as_str()))
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
