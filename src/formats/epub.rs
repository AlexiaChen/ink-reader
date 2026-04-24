use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rbook::Epub;

use crate::book::{BookMeta, BookReader, Chapter, ContentBlock, detect_image_mime};
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
                entry
                    .read_bytes()
                    .ok()
                    .and_then(|data| image::load_from_memory(&data).ok().map(|_| (data, mime)))
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
        let toc_titles = Self::collect_toc_titles(epub);
        let out = build_chapters_from_spine(
            epub.spine().iter().filter_map(|entry| {
                entry
                    .manifest_entry()
                    .map(|manifest| (manifest.href_raw().as_ref().to_string(), entry.is_linear()))
            }),
            &toc_titles,
        );

        if !out.is_empty() {
            return out;
        }

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

    fn collect_toc_titles(epub: &Epub) -> HashMap<String, String> {
        let Some(root) = epub.toc().contents() else {
            return HashMap::new();
        };

        toc_label_map(root.flatten().filter_map(|entry| {
            entry.href_raw().map(|href| {
                (
                    href.path().as_str().trim_start_matches('/').to_string(),
                    entry.label().to_string(),
                )
            })
        }))
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

        let href = resource_path(&chapter.resource_id);
        if href.is_empty() {
            return Ok(vec![ContentBlock::Paragraph("[Empty chapter]".to_string())]);
        }

        let html_bytes = self.epub.read_resource_bytes(href)?;
        let html = String::from_utf8_lossy(&html_bytes).into_owned();

        // 1. Collect all <img> (src, alt) pairs from the raw HTML (left-to-right order).
        let img_list = extract_img_tags(&html);

        // 2. Replace each <img> tag with a sentinel paragraph so html2text emits it
        //    as a distinct double-newline-separated paragraph we can detect later.
        let processed_html = if img_list.is_empty() {
            html.clone()
        } else {
            let html_lower = html.to_ascii_lowercase();
            let mut out = String::with_capacity(html.len() + img_list.len() * 32);
            let mut pos = 0;
            let mut img_idx = 0usize;
            loop {
                let Some(rel_start) = html_lower[pos..].find("<img") else {
                    out.push_str(&html[pos..]);
                    break;
                };
                let abs_start = pos + rel_start;
                let next = html_lower.as_bytes().get(abs_start + 4).copied();
                if !matches!(
                    next,
                    Some(b' ')
                        | Some(b'\t')
                        | Some(b'\n')
                        | Some(b'\r')
                        | Some(b'>')
                        | Some(b'/')
                        | None
                ) {
                    out.push_str(&html[pos..abs_start + 4]);
                    pos = abs_start + 4;
                    continue;
                }
                let Some(rel_end) = html_lower[abs_start..].find('>') else {
                    out.push_str(&html[pos..]);
                    break;
                };
                out.push_str(&html[pos..abs_start]);
                out.push_str(&format!("</p><p>__INKIMG_{}__</p><p>", img_idx));
                pos = abs_start + rel_end + 1;
                img_idx += 1;
            }
            out
        };

        // 3. Run html2text; fall back to original HTML on failure.
        let text = html2text::from_read(processed_html.as_bytes(), 80).unwrap_or_else(|_| {
            html2text::from_read(html_bytes.as_slice(), 80)
                .unwrap_or_else(|_| String::from_utf8_lossy(&html_bytes).into_owned())
        });

        // 4. Split into blocks, swapping sentinels for Image (or placeholder) blocks.
        let mut blocks: Vec<ContentBlock> = Vec::new();
        for para in text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if let Some(idx) = parse_img_sentinel(para) {
                if let Some((src, alt)) = img_list.get(idx) {
                    let resolved = resolve_href(href, src);
                    if let Some(block) = self.load_image_block(&resolved, alt) {
                        blocks.push(block);
                    } else {
                        // Emit a text placeholder so the reader knows content was here.
                        let placeholder = if alt.is_empty() {
                            "[Image]".to_string()
                        } else {
                            format!("[Image: {}]", alt)
                        };
                        blocks.push(ContentBlock::Paragraph(placeholder));
                    }
                }
            } else {
                blocks.push(ContentBlock::Paragraph(para.to_string()));
            }
        }

        if blocks.is_empty() {
            Ok(vec![ContentBlock::Paragraph("[Empty chapter]".to_string())])
        } else {
            Ok(blocks)
        }
    }
}

impl EpubReader {
    /// Load an image by its resolved EPUB manifest href.
    /// Validates via magic-byte check only — full decode is deferred to display time.
    fn load_image_block(&self, resolved: &str, alt: &str) -> Option<ContentBlock> {
        if resolved.is_empty() {
            return None;
        }
        let data = self.epub.read_resource_bytes(resolved).ok()?;
        if data.is_empty() {
            return None;
        }
        let mime = detect_image_mime(&data);
        if mime == "image/unknown" {
            return None; // SVG or unsupported — caller emits placeholder
        }
        Some(ContentBlock::Image {
            data,
            alt: alt.to_string(),
            mime: mime.to_string(),
        })
    }
}

// ── Module-level helpers ──────────────────────────────────────────────────────

/// Extract all `<img>` tags from an HTML string, returning `(src, alt)` pairs
/// in document order.
fn extract_img_tags(html: &str) -> Vec<(String, String)> {
    let html_lower = html.to_ascii_lowercase();
    let mut imgs = Vec::new();
    let mut pos = 0;
    while pos < html.len() {
        let Some(rel_start) = html_lower[pos..].find("<img") else {
            break;
        };
        let abs_start = pos + rel_start;
        // Verify `<img` is a word boundary, not e.g. `<imgfoo>`.
        let next = html_lower.as_bytes().get(abs_start + 4).copied();
        if !matches!(
            next,
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'>') | Some(b'/') | None
        ) {
            pos = abs_start + 4;
            continue;
        }
        // Find closing `>`.  Note: `>` inside quoted attributes must be &gt; in
        // valid XHTML, so the first `>` reliably ends the tag in conformant EPUBs.
        let Some(rel_end) = html_lower[abs_start..].find('>') else {
            break;
        };
        let tag = &html[abs_start..abs_start + rel_end + 1];
        let src = extract_attr(tag, "src").unwrap_or_default();
        let alt = extract_attr(tag, "alt").unwrap_or_default();
        imgs.push((src, alt));
        pos = abs_start + rel_end + 1;
    }
    imgs
}

/// Extract an attribute value from a single HTML tag string.
///
/// Handles `"`, `'`, and unquoted values.  Iterates all occurrences of
/// `attr=` so that names like `data-src=` do not falsely shadow `src=`.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let tag_lower = tag.to_ascii_lowercase();
    let search = format!("{}=", attr);
    let mut pos = 0;
    while pos < tag_lower.len() {
        let rel = tag_lower[pos..].find(search.as_str())?;
        let abs = pos + rel;
        // Require that the character before `attr=` is not alphanumeric / `-` / `_`
        // (which would mean we matched inside a longer attribute name).
        let prev_ok = abs == 0 || {
            let prev = tag_lower.as_bytes()[abs - 1];
            !prev.is_ascii_alphanumeric() && prev != b'-' && prev != b'_'
        };
        if prev_ok {
            let rest = &tag[abs + search.len()..];
            return if rest.starts_with('"') {
                rest[1..]
                    .find('"')
                    .and_then(|e| rest.get(1..1 + e))
                    .map(str::to_string)
            } else if rest.starts_with('\'') {
                rest[1..]
                    .find('\'')
                    .and_then(|e| rest.get(1..1 + e))
                    .map(str::to_string)
            } else {
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .unwrap_or(rest.len());
                if end > 0 {
                    Some(rest[..end].to_string())
                } else {
                    None
                }
            };
        }
        pos = abs + 1;
    }
    None
}

/// Resolve a relative image `src` against the chapter's href, producing the
/// path suitable for `epub.read_resource_bytes()`.
///
/// Handles `./`, `../` (clamped at root), fragment-only hrefs, and empty src.
fn resolve_href(chapter_href: &str, img_src: &str) -> String {
    let src = img_src.trim();
    if src.is_empty() || src.starts_with('#') {
        return String::new();
    }
    if src.starts_with("http://") || src.starts_with("https://") {
        return String::new(); // external images are not supported in TUI reader
    }
    // Strip any fragment suffix from the src path.
    let src_path = src.splitn(2, '#').next().unwrap_or(src);
    if src_path.is_empty() {
        return String::new();
    }
    // Absolute paths within the ZIP: strip leading `/` and treat as root-relative.
    if src_path.starts_with('/') {
        return normalize_path(src_path.trim_start_matches('/'));
    }
    let base_dir = chapter_href
        .rfind('/')
        .map(|i| &chapter_href[..=i])
        .unwrap_or("");
    normalize_path(&format!("{}{}", base_dir, src_path))
}

fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

fn toc_label_map<I>(entries: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut labels = HashMap::new();
    for (href, label) in entries {
        if !href.is_empty() {
            labels.entry(href).or_insert(label);
        }
    }
    labels
}

fn build_chapters_from_spine<I>(entries: I, toc_titles: &HashMap<String, String>) -> Vec<Chapter>
where
    I: IntoIterator<Item = (String, bool)>,
{
    let mut chapters = Vec::new();

    for (resource_id, linear) in entries {
        if !linear || resource_id.is_empty() {
            continue;
        }

        let title = toc_titles
            .get(resource_id.as_str())
            .cloned()
            .unwrap_or_else(|| fallback_chapter_title(&resource_id, chapters.len()));

        chapters.push(Chapter {
            index: chapters.len(),
            title,
            resource_id,
        });
    }

    chapters
}

fn fallback_chapter_title(resource_id: &str, index: usize) -> String {
    Path::new(resource_id)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Chapter {}", index + 1))
}

fn resource_path(resource_id: &str) -> &str {
    resource_id.split('#').next().unwrap_or(resource_id)
}

/// Parse the `__INKIMG_N__` sentinel emitted by the html2text pass.
fn parse_img_sentinel(para: &str) -> Option<usize> {
    para.trim()
        .strip_prefix("__INKIMG_")
        .and_then(|s| s.strip_suffix("__"))
        .and_then(|n| n.parse::<usize>().ok())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_img_tags ──

    #[test]
    fn img_tags_basic() {
        let html = r#"<p>Text</p><img src="images/fig.png" alt="Figure 1"><p>After</p>"#;
        let imgs = extract_img_tags(html);
        assert_eq!(
            imgs,
            vec![("images/fig.png".to_string(), "Figure 1".to_string())]
        );
    }

    #[test]
    fn img_tags_self_closing() {
        let html = r#"<img src="fig.png" alt="x" />"#;
        let imgs = extract_img_tags(html);
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].0, "fig.png");
        assert_eq!(imgs[0].1, "x");
    }

    #[test]
    fn img_tags_no_alt() {
        let html = r#"<img src="fig.png">"#;
        let imgs = extract_img_tags(html);
        assert_eq!(imgs, vec![("fig.png".to_string(), String::new())]);
    }

    #[test]
    fn img_tags_multiple() {
        let html = r#"<img src="a.png"><p>text</p><img src="b.png" alt="B">"#;
        let imgs = extract_img_tags(html);
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].0, "a.png");
        assert_eq!(imgs[1].0, "b.png");
        assert_eq!(imgs[1].1, "B");
    }

    #[test]
    fn img_tags_uppercase() {
        let html = r#"<IMG SRC="fig.png" ALT="test">"#;
        let imgs = extract_img_tags(html);
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].0, "fig.png");
        assert_eq!(imgs[0].1, "test");
    }

    #[test]
    fn img_tags_skips_imgsrc_token() {
        // `<imgsrc=` must NOT be matched — only `<img ` and similar word boundaries.
        let html = r#"<imgsrc="wrong.png"><img src="right.png">"#;
        let imgs = extract_img_tags(html);
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].0, "right.png");
    }

    // ── extract_attr ──

    #[test]
    fn attr_data_src_does_not_shadow_src() {
        let tag = r#"<img data-src="fig.png" src="real.png">"#;
        assert_eq!(extract_attr(tag, "src"), Some("real.png".to_string()));
    }

    #[test]
    fn attr_single_quotes() {
        let tag = "<img src='fig.png'>";
        assert_eq!(extract_attr(tag, "src"), Some("fig.png".to_string()));
    }

    #[test]
    fn attr_missing_returns_none() {
        let tag = r#"<img src="fig.png">"#;
        assert_eq!(extract_attr(tag, "alt"), None);
    }

    // ── resolve_href ──

    #[test]
    fn resolve_simple_relative() {
        assert_eq!(
            resolve_href("OEBPS/text/ch01.html", "images/fig.png"),
            "OEBPS/text/images/fig.png"
        );
    }

    #[test]
    fn resolve_dotslash() {
        assert_eq!(
            resolve_href("OEBPS/text/ch01.html", "./images/fig.png"),
            "OEBPS/text/images/fig.png"
        );
    }

    #[test]
    fn resolve_dotdot() {
        assert_eq!(
            resolve_href("OEBPS/text/ch01.html", "../images/fig.png"),
            "OEBPS/images/fig.png"
        );
    }

    #[test]
    fn resolve_excessive_dotdot_clamped() {
        assert_eq!(
            resolve_href("OEBPS/text/ch01.html", "../../../img.png"),
            "img.png"
        );
    }

    #[test]
    fn resolve_empty_src() {
        assert_eq!(resolve_href("OEBPS/text/ch01.html", ""), "");
    }

    #[test]
    fn resolve_fragment_only() {
        assert_eq!(resolve_href("OEBPS/text/ch01.html", "#note1"), "");
    }

    #[test]
    fn resolve_external_skipped() {
        assert_eq!(
            resolve_href("OEBPS/text/ch01.html", "https://example.com/img.png"),
            ""
        );
    }

    // ── chapter collection ──

    #[test]
    fn toc_label_map_keeps_first_label_for_same_resource() {
        let labels = toc_label_map(vec![
            (
                "Text/Section0001.xhtml".to_string(),
                "第一节 河南节度使与张巡".to_string(),
            ),
            (
                "Text/Section0001.xhtml".to_string(),
                "第二节 元帅的时代".to_string(),
            ),
        ]);

        assert_eq!(
            labels.get("Text/Section0001.xhtml").map(String::as_str),
            Some("第一节 河南节度使与张巡")
        );
    }

    #[test]
    fn spine_chapters_keep_intermediate_sections_in_reading_order() {
        let labels = toc_label_map(vec![
            (
                "Text/part0006.xhtml".to_string(),
                "第一章 河南：对峙开始的地方".to_string(),
            ),
            (
                "Text/Section0001.xhtml".to_string(),
                "第一节 河南节度使与张巡".to_string(),
            ),
            (
                "Text/part0007.xhtml".to_string(),
                "第二章 关中：有关空间的命题".to_string(),
            ),
        ]);

        let chapters = build_chapters_from_spine(
            vec![
                ("Text/cover_page.xhtml".to_string(), false),
                ("Text/part0006.xhtml".to_string(), true),
                ("Text/Section0001.xhtml".to_string(), true),
                ("Text/part0007.xhtml".to_string(), true),
            ],
            &labels,
        );

        let resources: Vec<&str> = chapters.iter().map(|c| c.resource_id.as_str()).collect();
        assert_eq!(
            resources,
            vec![
                "Text/part0006.xhtml",
                "Text/Section0001.xhtml",
                "Text/part0007.xhtml",
            ]
        );
        assert_eq!(chapters[1].title, "第一节 河南节度使与张巡");
        assert_eq!(chapters[1].index, 1);
    }

    #[test]
    fn resource_path_strips_fragment_suffix() {
        assert_eq!(
            resource_path("Text/Section0001.xhtml#hh2-1"),
            "Text/Section0001.xhtml"
        );
    }

    // ── parse_img_sentinel ──

    #[test]
    fn sentinel_valid() {
        assert_eq!(parse_img_sentinel("__INKIMG_0__"), Some(0));
        assert_eq!(parse_img_sentinel("__INKIMG_42__"), Some(42));
        assert_eq!(parse_img_sentinel("  __INKIMG_7__  "), Some(7));
    }

    #[test]
    fn sentinel_invalid() {
        assert_eq!(parse_img_sentinel("Normal paragraph"), None);
        assert_eq!(parse_img_sentinel("__INKIMG_abc__"), None);
        assert_eq!(parse_img_sentinel("__INKIMG__"), None);
    }

    // ── end-to-end: sentinel survives html2text ──

    #[test]
    fn sentinel_survives_html2text_paragraph() {
        let html = b"<html><body><p>Before</p><p>__INKIMG_0__</p><p>After</p></body></html>";
        let text = html2text::from_read(&html[..], 80).unwrap();
        let paras: Vec<&str> = text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        assert!(
            paras.contains(&"__INKIMG_0__"),
            "sentinel must survive html2text round-trip: {:?}",
            paras
        );
    }

    #[test]
    fn sentinel_survives_root_level_img() {
        // Root-level <img> (not inside <p>) — the injection produces unmatched tags
        // that html5ever must handle gracefully.
        let html = b"<html><body><p>A</p></p><p>__INKIMG_0__</p><p><p>B</p></body></html>";
        let text = html2text::from_read(&html[..], 80).unwrap();
        assert!(
            text.contains("__INKIMG_0__"),
            "sentinel must survive: {:?}",
            text
        );
    }
}
