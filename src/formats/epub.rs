use std::path::Path;

use anyhow::Result;
use rbook::Epub;

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock};
pub struct EpubReader {
    epub: Epub,
    meta: BookMeta,
    cover: Option<(Vec<u8>, String)>,
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

        let cover = Self::extract_cover(&epub);
        let chapters = Self::collect_chapters(&epub);

        let meta = BookMeta {
            title,
            author,
            chapters,
        };

        Ok(Self { epub, meta, cover })
    }

    /// Extract the cover image from the EPUB manifest.
    ///
    /// Tries the manifest's dedicated cover-image entry first; falls back to
    /// any manifest image whose `id` or `href` contains "cover".  Returns
    /// `None` if no decodable cover image is found (e.g. SVG-only covers).
    fn extract_cover(epub: &Epub) -> Option<(Vec<u8>, String)> {
        // Primary path: manifest item with the "cover-image" property (EPUB3)
        // or the item referenced by the <meta name="cover"> element (EPUB2).
        let primary = {
            let manifest = epub.manifest();
            manifest.cover_image().and_then(|entry| {
                let mime = entry.media_type().to_string();
                entry.read_bytes().ok().and_then(|data| {
                    image::load_from_memory(&data).ok().map(|_| (data, mime))
                })
            })
        };

        if primary.is_some() {
            return primary;
        }

        // Fallback: search image entries whose id or href hints at a cover.
        // We intentionally do NOT scan all images to avoid picking up publisher
        // logos or other decorative artwork.
        let manifest = epub.manifest();
        manifest.images().find_map(|entry| {
            let href_lc = entry.href().as_ref().to_lowercase();
            let id_lc = entry.id().to_lowercase();
            if !href_lc.contains("cover") && !id_lc.contains("cover") {
                return None;
            }
            let mime = entry.media_type().to_string();
            entry
                .read_bytes()
                .ok()
                .and_then(|data| image::load_from_memory(&data).ok().map(|_| (data, mime)))
        })
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

    fn cover_image(&self) -> Option<(&[u8], &str)> {
        self.cover.as_ref().map(|(d, m)| (d.as_slice(), m.as_str()))
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

