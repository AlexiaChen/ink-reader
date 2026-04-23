use std::path::Path;

use anyhow::Result;

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock};

/// Reader for plain text and Markdown files.
/// Treats the entire file as a single chapter with no images.
pub struct TxtReader {
    meta: BookMeta,
    content: String,
}

impl TxtReader {
    pub fn open(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path)?;
        // Decode as UTF-8, replacing invalid bytes with replacement char
        let content = String::from_utf8_lossy(&raw).into_owned();

        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let meta = BookMeta {
            title: title.clone(),
            author: None,
            chapters: vec![Chapter {
                index: 0,
                title,
                resource_id: path.to_string_lossy().into_owned(),
            }],
        };

        Ok(Self { meta, content })
    }
}

impl BookReader for TxtReader {
    fn meta(&self) -> &BookMeta {
        &self.meta
    }

    fn chapter_blocks(&self, chapter_idx: usize) -> Result<Vec<ContentBlock>> {
        if chapter_idx != 0 {
            return Ok(vec![]);
        }
        // Split into paragraphs on blank lines
        let blocks: Vec<ContentBlock> = self
            .content
            .split("\n\n")
            .map(|para| para.trim())
            .filter(|para| !para.is_empty())
            .map(|para| ContentBlock::Paragraph(para.to_string()))
            .collect();

        if blocks.is_empty() {
            Ok(vec![ContentBlock::Paragraph(self.content.clone())])
        } else {
            Ok(blocks)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_txt(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn single_chapter() {
        let f = write_txt("Hello world\n\nSecond paragraph");
        let r = TxtReader::open(f.path()).unwrap();
        assert_eq!(r.meta().chapters.len(), 1);
    }

    #[test]
    fn paragraphs_split_on_blank_lines() {
        let f = write_txt("Para one\n\nPara two\n\nPara three");
        let r = TxtReader::open(f.path()).unwrap();
        let blocks = r.chapter_blocks(0).unwrap();
        assert_eq!(blocks.len(), 3);
    }

    #[test]
    fn empty_file_yields_one_block() {
        let f = write_txt("");
        let r = TxtReader::open(f.path()).unwrap();
        let blocks = r.chapter_blocks(0).unwrap();
        assert!(!blocks.is_empty());
    }

    #[test]
    fn out_of_range_chapter() {
        let f = write_txt("hello");
        let r = TxtReader::open(f.path()).unwrap();
        let blocks = r.chapter_blocks(99).unwrap();
        assert!(blocks.is_empty());
    }
}
