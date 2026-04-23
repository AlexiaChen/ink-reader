use std::path::Path;

use anyhow::Result;
use rbook::{Epub, prelude::*};

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock};

pub struct EpubReader {
    epub: Epub,
    meta: BookMeta,
}

impl EpubReader {
    pub fn open(path: &Path) -> Result<Self> {
        let epub = Epub::open(path.to_str().unwrap_or(""))?;

        let title = epub
            .metadata()
            .title()
            .map(|e| e.value().to_string())
            .unwrap_or_else(|| "Untitled".to_string());

        let author = epub
            .metadata()
            .creators()
            .next()
            .map(|e| e.value().to_string());

        let chapters = Self::collect_chapters(&epub);

        let meta = BookMeta {
            title,
            author,
            chapters,
        };

        Ok(Self { epub, meta })
    }

    fn collect_chapters(epub: &Epub) -> Vec<Chapter> {
        let mut out = Vec::new();
        if let Some(root) = epub.toc().contents() {
            for (idx, entry) in root.iter().enumerate() {
                let title = entry.label().to_string();
                // Get the href/path from the resource
                let resource_id = entry
                    .resource()
                    .and_then(|r| r.key().value().map(|v| v.to_string()))
                    .unwrap_or_default();
                out.push(Chapter {
                    index: idx,
                    title,
                    resource_id,
                });
            }
        }
        // Fallback: no ToC → single synthetic chapter
        if out.is_empty() {
            out.push(Chapter {
                index: 0,
                title: "Chapter 1".to_string(),
                resource_id: String::new(),
            });
        }
        out
    }
}

impl BookReader for EpubReader {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
        let chapter = match self.meta.chapters.get(chapter_idx) {
            Some(c) => c,
            None => return Ok(vec![]),
        };

        let href = &chapter.resource_id;
        if href.is_empty() {
            return Ok(vec![ContentBlock::Paragraph(
                "[Empty chapter]".to_string(),
            )]);
        }

        let html_bytes = self.epub.read_resource_bytes(href.as_str())?;

        let text = html2text::from_read(html_bytes.as_slice(), 80)
            .unwrap_or_else(|_| String::from_utf8_lossy(&html_bytes).into_owned());

        let blocks: Vec<ContentBlock> = text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| ContentBlock::Paragraph(s.to_string()))
            .collect();

        if blocks.is_empty() {
            Ok(vec![ContentBlock::Paragraph("[Empty chapter]".to_string())])
        } else {
            Ok(blocks)
        }
    }
}

