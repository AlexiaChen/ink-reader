use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use anyhow::Result;
use rbook::Epub;

use crate::book::{
    BookMeta, BookReader, Chapter, ContentBlock, INLINE_REF_CLOSE, INLINE_REF_OPEN,
    detect_image_mime,
};
pub struct EpubReader {
    epub: Epub,
    meta: BookMeta,
    cover: Option<(Vec<u8>, String)>,
    section_labels: HashMap<String, Vec<SectionLabel>>,
}

struct ReferenceReplacement {
    range: Range<usize>,
    replacement_html: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionLabel {
    fragment: String,
    title: String,
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
        let toc_titles = Self::collect_toc_titles(&epub);
        let section_labels = Self::collect_toc_section_labels(&epub);
        let chapters = Self::collect_chapters(&epub, &toc_titles, &section_labels);

        let meta = BookMeta {
            title,
            author,
            chapters,
        };

        Ok(Self {
            epub,
            meta,
            cover,
            section_labels,
        })
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

    fn collect_chapters(
        epub: &Epub,
        toc_titles: &HashMap<String, String>,
        section_labels: &HashMap<String, Vec<SectionLabel>>,
    ) -> Vec<Chapter> {
        let out = build_chapters_from_spine(
            epub.spine().iter().filter_map(|entry| {
                entry
                    .manifest_entry()
                    .map(|manifest| (manifest.href_raw().as_ref().to_string(), entry.is_linear()))
            }),
            toc_titles,
            section_labels,
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

    fn collect_toc_section_labels(epub: &Epub) -> HashMap<String, Vec<SectionLabel>> {
        let Some(root) = epub.toc().contents() else {
            return HashMap::new();
        };

        section_label_map(root.flatten().filter_map(|entry| {
            let href = entry.href_raw()?;
            let fragment = href.fragment()?.trim();
            let resource = href.path().as_str().trim_start_matches('/').to_string();
            if resource.is_empty() || fragment.is_empty() {
                return None;
            }
            Some((resource, fragment.to_string(), entry.label().to_string()))
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
        let full_html = String::from_utf8_lossy(&html_bytes).into_owned();
        let next_fragment = self
            .meta
            .chapters
            .get(chapter_idx + 1)
            .filter(|next| resource_path(&next.resource_id) == href)
            .and_then(|next| resource_fragment(&next.resource_id));
        let html = slice_resource_html(
            &full_html,
            resource_fragment(&chapter.resource_id),
            next_fragment,
        );
        let section_labels = self.section_labels.get(href).cloned().unwrap_or_default();
        let chapter_html = inline_reference_links(&html, href, |resource| {
            if resource == href {
                Some(full_html.clone())
            } else {
                self.epub
                    .read_resource_bytes(resource)
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            }
        });
        let chapter_html = inject_section_sentinels(&chapter_html, &section_labels);

        // 1. Collect all <img> (src, alt) pairs from the raw HTML (left-to-right order).
        let img_list = extract_img_tags(&chapter_html);

        // 2. Replace each <img> tag with a sentinel paragraph so html2text emits it
        //    as a distinct double-newline-separated paragraph we can detect later.
        let processed_html = if img_list.is_empty() {
            chapter_html.clone()
        } else {
            let html_lower = chapter_html.to_ascii_lowercase();
            let mut out = String::with_capacity(chapter_html.len() + img_list.len() * 32);
            let mut pos = 0;
            let mut img_idx = 0usize;
            loop {
                let Some(rel_start) = html_lower[pos..].find("<img") else {
                    out.push_str(&chapter_html[pos..]);
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
                    out.push_str(&chapter_html[pos..abs_start + 4]);
                    pos = abs_start + 4;
                    continue;
                }
                let Some(rel_end) = html_lower[abs_start..].find('>') else {
                    out.push_str(&chapter_html[pos..]);
                    break;
                };
                out.push_str(&chapter_html[pos..abs_start]);
                out.push_str(&format!("</p><p>__INKIMG_{}__</p><p>", img_idx));
                pos = abs_start + rel_end + 1;
                img_idx += 1;
            }
            out
        };

        // 3. Run html2text; fall back to original HTML on failure.
        let text = html2text::from_read(processed_html.as_bytes(), 80).unwrap_or_else(|_| {
            html2text::from_read(chapter_html.as_bytes(), 80).unwrap_or(chapter_html)
        });

        // 4. Split into blocks, swapping sentinels for Image (or placeholder) blocks.
        let mut blocks: Vec<ContentBlock> = Vec::new();
        for para in text
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if let Some(idx) = parse_section_sentinel(para) {
                if let Some(section) = section_labels.get(idx) {
                    blocks.push(ContentBlock::SectionMarker(section.title.clone()));
                }
            } else if let Some(idx) = parse_img_sentinel(para) {
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

fn inline_reference_links<F>(html: &str, chapter_href: &str, mut load_resource_html: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let html_lower = html.to_ascii_lowercase();
    let current_resource = resource_path(chapter_href).to_string();
    let mut replacements = Vec::new();
    let mut resource_cache: HashMap<String, Option<String>> = HashMap::new();
    let mut pos = 0;

    while pos < html.len() {
        let Some(rel_start) = html_lower[pos..].find("<a") else {
            break;
        };
        let abs_start = pos + rel_start;
        let next = html_lower.as_bytes().get(abs_start + 2).copied();
        if !matches!(
            next,
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'>') | None
        ) {
            pos = abs_start + 2;
            continue;
        }

        let Some(rel_open_end) = html_lower[abs_start..].find('>') else {
            break;
        };
        let open_end = abs_start + rel_open_end;
        let tag = &html[abs_start..=open_end];
        let content_start = open_end + 1;
        let Some(rel_close) = html_lower[content_start..].find("</a>") else {
            break;
        };
        let close_start = content_start + rel_close;
        let anchor_end = close_start + "</a>".len();
        let marker_html = &html[content_start..close_start];
        let marker_text = collapse_whitespace(&html_fragment_to_text(marker_html));

        let Some(href) = extract_attr(tag, "href") else {
            pos = anchor_end;
            continue;
        };
        let Some((target_resource, fragment)) = resolve_reference_target(chapter_href, &href)
        else {
            pos = anchor_end;
            continue;
        };
        if !is_reference_anchor(tag, &marker_text, html, abs_start, anchor_end) {
            pos = anchor_end;
            continue;
        }

        let note_text = if target_resource == current_resource {
            extract_reference_text(html, &fragment, &marker_text).or_else(|| {
                resource_cache
                    .entry(target_resource.clone())
                    .or_insert_with(|| load_resource_html(target_resource.as_str()))
                    .as_deref()
                    .and_then(|target_html| {
                        extract_reference_text(target_html, &fragment, &marker_text)
                    })
            })
        } else {
            resource_cache
                .entry(target_resource.clone())
                .or_insert_with(|| load_resource_html(target_resource.as_str()))
                .as_deref()
                .and_then(|target_html| {
                    extract_reference_text(target_html, &fragment, &marker_text)
                })
        };

        let Some(note_text) = note_text else {
            pos = anchor_end;
            continue;
        };

        replacements.push(ReferenceReplacement {
            range: expand_reference_wrapper(html, abs_start, anchor_end),
            replacement_html: escape_html_text(&format!(
                "{INLINE_REF_OPEN}{note_text}{INLINE_REF_CLOSE}"
            )),
        });
        pos = anchor_end;
    }

    if replacements.is_empty() {
        return html.to_string();
    }

    let mut out = String::with_capacity(
        html.len()
            + replacements
                .iter()
                .map(|r| r.replacement_html.len())
                .sum::<usize>(),
    );
    let mut cursor = 0;

    for replacement in replacements {
        if replacement.range.start < cursor {
            continue;
        }
        out.push_str(&html[cursor..replacement.range.start]);
        out.push_str(&replacement.replacement_html);
        cursor = replacement.range.end;
    }
    out.push_str(&html[cursor..]);
    out
}

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
            return if let Some(rest) = rest.strip_prefix('"') {
                rest.find('"')
                    .and_then(|e| rest.get(..e))
                    .map(str::to_string)
            } else if let Some(rest) = rest.strip_prefix('\'') {
                rest.find('\'')
                    .and_then(|e| rest.get(..e))
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
    let src_path = src.split('#').next().unwrap_or(src);
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

fn resolve_reference_target(chapter_href: &str, link_href: &str) -> Option<(String, String)> {
    let href = link_href.trim();
    if href.is_empty() || href.starts_with("http://") || href.starts_with("https://") {
        return None;
    }

    let (path, fragment) = href.split_once('#')?;
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return None;
    }

    let resource = if path.trim().is_empty() {
        resource_path(chapter_href).to_string()
    } else {
        resolve_href(chapter_href, path)
    };
    if resource.is_empty() {
        None
    } else {
        Some((resource, fragment.to_string()))
    }
}

fn extract_reference_text(html: &str, fragment: &str, marker_text: &str) -> Option<String> {
    let target_html = extract_target_element_html(html, fragment)?;
    let preferred_html = preferred_reference_fragment(&target_html);
    let text = clean_reference_text(&html_fragment_to_text(preferred_html), marker_text);
    (!text.is_empty()).then_some(text)
}

fn extract_target_element_html(html: &str, fragment: &str) -> Option<String> {
    let html_lower = html.to_ascii_lowercase();
    let mut open_elements: Vec<(String, usize)> = Vec::new();
    let mut pos = 0;

    while pos < html_lower.len() {
        let Some(rel_start) = html_lower[pos..].find('<') else {
            break;
        };
        let abs_start = pos + rel_start;
        let Some(rel_end) = html_lower[abs_start..].find('>') else {
            break;
        };
        let abs_end = abs_start + rel_end;
        let tag = &html[abs_start..=abs_end];

        let Some(tag_name) = parse_tag_name(tag) else {
            pos = abs_end + 1;
            continue;
        };
        let trimmed = tag.trim_start();

        if trimmed.starts_with("</") {
            if let Some(idx) = open_elements
                .iter()
                .rposition(|(open_name, _)| open_name == &tag_name)
            {
                open_elements.truncate(idx);
            }
            pos = abs_end + 1;
            continue;
        }

        let has_id = extract_attr(tag, "id").as_deref() == Some(fragment)
            || extract_attr(tag, "xml:id").as_deref() == Some(fragment);

        if has_id {
            if is_block_container(&tag_name) {
                return extract_outer_element_html(html, abs_start, &tag_name);
            }

            if let Some((block_tag, block_start)) = open_elements
                .iter()
                .rev()
                .find(|(open_name, _)| is_block_container(open_name))
            {
                let block_range = extract_outer_element_range(html, *block_start, block_tag)?;
                let target_range = if trimmed.ends_with("/>") {
                    abs_start..abs_end + 1
                } else {
                    extract_outer_element_range(html, abs_start, &tag_name)?
                };

                if target_range.start >= block_range.start && target_range.end <= block_range.end {
                    return Some(format!(
                        "{}{}",
                        &html[block_range.start..target_range.start],
                        &html[target_range.end..block_range.end]
                    ));
                }

                return Some(html[block_range].to_string());
            }
        }

        if !trimmed.ends_with("/>") {
            open_elements.push((tag_name, abs_start));
        }

        pos = abs_end + 1;
    }

    None
}

fn extract_outer_element_html(html: &str, element_start: usize, tag_name: &str) -> Option<String> {
    let range = extract_outer_element_range(html, element_start, tag_name)?;
    Some(html[range].to_string())
}

fn extract_outer_element_range(
    html: &str,
    element_start: usize,
    tag_name: &str,
) -> Option<Range<usize>> {
    let mut depth = 0usize;
    let mut pos = element_start;

    while pos < html.len() {
        let rel_start = html[pos..].find('<')?;
        let abs_start = pos + rel_start;
        let rel_end = html[abs_start..].find('>')?;
        let abs_end = abs_start + rel_end;
        let tag = &html[abs_start..=abs_end];
        let Some(name) = parse_tag_name(tag) else {
            pos = abs_end + 1;
            continue;
        };
        if name != tag_name {
            pos = abs_end + 1;
            continue;
        }

        let trimmed = tag.trim_start();
        if trimmed.starts_with("</") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(element_start..abs_end + 1);
            }
        } else if !trimmed.ends_with("/>") {
            depth += 1;
        }

        pos = abs_end + 1;
    }

    None
}

fn parse_tag_name(tag: &str) -> Option<String> {
    let tag = tag.strip_prefix('<')?;
    let tag = tag.strip_prefix('/').unwrap_or(tag).trim_start();
    let end = tag
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(tag.len());
    (end > 0).then(|| tag[..end].to_ascii_lowercase())
}

fn is_block_container(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "aside"
            | "blockquote"
            | "dl"
            | "dd"
            | "div"
            | "dt"
            | "li"
            | "ol"
            | "p"
            | "section"
            | "td"
            | "tr"
            | "ul"
    )
}

fn html_fragment_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 80).unwrap_or_else(|_| html.to_string())
}

fn preferred_reference_fragment(html: &str) -> &str {
    extract_first_tag(html, "dd").unwrap_or(html)
}

fn clean_reference_text(raw: &str, marker_text: &str) -> String {
    let mut text = collapse_whitespace(raw);

    while let Some(stripped) = text.strip_suffix("↩︎").or_else(|| text.strip_suffix("↩")) {
        text = stripped.trim_end().to_string();
    }

    text = text
        .trim_start_matches(['•', '*', '-'])
        .trim_start()
        .to_string();

    for prefix in [
        marker_text.trim(),
        strip_reference_brackets(marker_text.trim()),
    ] {
        if prefix.is_empty() {
            continue;
        }
        if let Some(rest) = text.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| {
                c.is_whitespace() || matches!(c, '.' | ',' | ':' | ';' | '、' | '，' | '。')
            });
            if !rest.is_empty() {
                text = rest.to_string();
                break;
            }
        }
    }

    text
}

fn strip_reference_brackets(text: &str) -> &str {
    text.trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | '{' | '}' | '^'))
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_reference_anchor(tag: &str, marker_text: &str, html: &str, start: usize, end: usize) -> bool {
    if extract_attr(tag, "epub:type")
        .map(|value| value.to_ascii_lowercase().contains("noteref"))
        .unwrap_or(false)
        || extract_attr(tag, "role")
            .map(|value| value.eq_ignore_ascii_case("doc-noteref"))
            .unwrap_or(false)
    {
        return true;
    }

    is_reference_marker(marker_text) || expand_reference_wrapper(html, start, end).start < start
}

fn is_reference_marker(text: &str) -> bool {
    let marker = text.trim();
    if marker.is_empty() || marker.len() > 24 {
        return false;
    }

    let mut saw_signal = false;
    for ch in marker.chars() {
        if ch.is_ascii_digit() {
            saw_signal = true;
            continue;
        }
        if ch.is_ascii_whitespace() {
            continue;
        }
        if ch.is_ascii_alphabetic() && marker.len() <= 3 {
            saw_signal = true;
            continue;
        }
        if matches!(
            ch,
            '[' | ']'
                | '('
                | ')'
                | '{'
                | '}'
                | '<'
                | '>'
                | '.'
                | ','
                | ':'
                | ';'
                | '*'
                | '#'
                | '^'
                | '-'
                | '+'
                | '/'
                | '\\'
                | '|'
                | '~'
                | '†'
                | '‡'
        ) {
            continue;
        }
        return false;
    }

    saw_signal
}

fn expand_reference_wrapper(html: &str, start: usize, end: usize) -> Range<usize> {
    let before = &html[..start];
    let Some(sup_start) = before.rfind("<sup") else {
        return start..end;
    };
    let Some(rel_open_end) = html[sup_start..].find('>') else {
        return start..end;
    };
    let sup_open_end = sup_start + rel_open_end;
    if !html[sup_open_end + 1..start].trim().is_empty() {
        return start..end;
    }

    let after = &html[end..];
    let whitespace = after
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(after.len());
    let sup_close_start = end + whitespace;
    let Some(close) = html.get(sup_close_start..sup_close_start + "</sup>".len()) else {
        return start..end;
    };
    if !close.eq_ignore_ascii_case("</sup>") {
        return start..end;
    }

    sup_start..sup_close_start + "</sup>".len()
}

fn escape_html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn extract_first_tag<'a>(html: &'a str, tag_name: &str) -> Option<&'a str> {
    let html_lower = html.to_ascii_lowercase();
    let open_pat = format!("<{tag_name}");
    let start = html_lower.find(&open_pat)?;
    let open_end = start + html[start..].find('>')?;
    let close_pat = format!("</{tag_name}>");
    let close_start = html_lower[open_end + 1..].find(&close_pat)? + open_end + 1;
    html.get(start..close_start + close_pat.len())
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

fn section_label_map<I>(entries: I) -> HashMap<String, Vec<SectionLabel>>
where
    I: IntoIterator<Item = (String, String, String)>,
{
    let mut labels: HashMap<String, Vec<SectionLabel>> = HashMap::new();
    for (resource, fragment, title) in entries {
        if resource.is_empty() || fragment.is_empty() {
            continue;
        }

        let sections = labels.entry(resource).or_default();
        if sections.iter().any(|section| section.fragment == fragment) {
            continue;
        }
        sections.push(SectionLabel { fragment, title });
    }
    labels
}

fn build_chapters_from_spine<I>(
    entries: I,
    toc_titles: &HashMap<String, String>,
    section_labels: &HashMap<String, Vec<SectionLabel>>,
) -> Vec<Chapter>
where
    I: IntoIterator<Item = (String, bool)>,
{
    let mut chapters = Vec::new();

    for (resource_id, linear) in entries {
        if !linear || resource_id.is_empty() {
            continue;
        }

        if let Some(sections) = section_labels.get(resource_id.as_str()) {
            for section in sections {
                chapters.push(Chapter {
                    index: chapters.len(),
                    title: section.title.clone(),
                    resource_id: format!("{resource_id}#{}", section.fragment),
                });
            }
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

fn resource_fragment(resource_id: &str) -> Option<&str> {
    resource_id
        .split_once('#')
        .map(|(_, fragment)| fragment)
        .filter(|fragment| !fragment.is_empty())
}

fn slice_resource_html(
    html: &str,
    start_fragment: Option<&str>,
    end_fragment: Option<&str>,
) -> String {
    let html_lower = html.to_ascii_lowercase();
    let start = start_fragment
        .and_then(|fragment| find_section_anchor_start(html, &html_lower, fragment))
        .unwrap_or(0);
    let end = end_fragment
        .and_then(|fragment| find_section_anchor_start(html, &html_lower, fragment))
        .filter(|end| *end > start)
        .unwrap_or(html.len());
    html[start..end].to_string()
}

fn inject_section_sentinels(html: &str, section_labels: &[SectionLabel]) -> String {
    if section_labels.is_empty() {
        return html.to_string();
    }

    let html_lower = html.to_ascii_lowercase();
    let mut insertions = Vec::new();
    for (idx, section) in section_labels.iter().enumerate() {
        if let Some(position) = find_section_anchor_start(html, &html_lower, &section.fragment)
            && insertions.iter().all(|(existing, _)| *existing != position)
        {
            insertions.push((position, idx));
        }
    }

    if insertions.is_empty() {
        return html.to_string();
    }

    insertions.sort_by_key(|(position, _)| *position);

    let mut out = String::with_capacity(html.len() + insertions.len() * 32);
    let mut pos = 0;
    for (insert_at, idx) in insertions {
        out.push_str(&html[pos..insert_at]);
        out.push_str(&format!("</p><p>__INKSEC_{}__</p><p>", idx));
        pos = insert_at;
    }
    out.push_str(&html[pos..]);
    out
}

fn find_section_anchor_start(html: &str, html_lower: &str, fragment: &str) -> Option<usize> {
    let mut pos = 0;
    while pos < html.len() {
        let Some(rel_start) = html_lower[pos..].find('<') else {
            break;
        };
        let abs_start = pos + rel_start;
        let Some(rel_end) = html_lower[abs_start..].find('>') else {
            break;
        };
        let abs_end = abs_start + rel_end + 1;
        let tag = &html[abs_start..abs_end];

        if ["id", "xml:id", "name"]
            .iter()
            .filter_map(|attr| extract_attr(tag, attr))
            .any(|value| value.eq_ignore_ascii_case(fragment))
        {
            return Some(abs_start);
        }

        pos = abs_end;
    }
    None
}

/// Parse the `__INKIMG_N__` sentinel emitted by the html2text pass.
fn parse_img_sentinel(para: &str) -> Option<usize> {
    para.trim()
        .strip_prefix("__INKIMG_")
        .and_then(|s| s.strip_suffix("__"))
        .and_then(|n| n.parse::<usize>().ok())
}

fn parse_section_sentinel(para: &str) -> Option<usize> {
    para.trim()
        .strip_prefix("__INKSEC_")
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
        let section_labels = section_label_map(vec![
            (
                "Text/part0006.xhtml".to_string(),
                "preface".to_string(),
                "序章".to_string(),
            ),
            (
                "Text/part0006.xhtml".to_string(),
                "chapter-1".to_string(),
                "第一章 河南：对峙开始的地方".to_string(),
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
            &section_labels,
        );

        let resources: Vec<&str> = chapters.iter().map(|c| c.resource_id.as_str()).collect();
        assert_eq!(
            resources,
            vec![
                "Text/part0006.xhtml#preface",
                "Text/part0006.xhtml#chapter-1",
                "Text/Section0001.xhtml",
                "Text/part0007.xhtml",
            ]
        );
        assert_eq!(chapters[1].title, "第一章 河南：对峙开始的地方");
        assert_eq!(chapters[2].title, "第一节 河南节度使与张巡");
        assert_eq!(chapters[2].index, 2);
    }

    #[test]
    fn section_label_map_keeps_fragment_order_per_resource() {
        let labels = section_label_map(vec![
            (
                "Text/part0006.xhtml".to_string(),
                "preface".to_string(),
                "序章".to_string(),
            ),
            (
                "Text/part0006.xhtml".to_string(),
                "chapter-1".to_string(),
                "第一章".to_string(),
            ),
        ]);

        assert_eq!(
            labels.get("Text/part0006.xhtml"),
            Some(&vec![
                SectionLabel {
                    fragment: "preface".to_string(),
                    title: "序章".to_string(),
                },
                SectionLabel {
                    fragment: "chapter-1".to_string(),
                    title: "第一章".to_string(),
                },
            ])
        );
    }

    #[test]
    fn resource_path_strips_fragment_suffix() {
        assert_eq!(
            resource_path("Text/Section0001.xhtml#hh2-1"),
            "Text/Section0001.xhtml"
        );
    }

    #[test]
    fn slice_resource_html_stops_at_next_fragment_anchor() {
        let html =
            r#"<h1 id="preface">序章</h1><p>前言</p><h1 id="chapter-1">第一章</h1><p>正文</p>"#;
        let sliced = slice_resource_html(html, Some("preface"), Some("chapter-1"));

        assert!(sliced.contains("序章"));
        assert!(sliced.contains("前言"));
        assert!(!sliced.contains("第一章"));
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

    #[test]
    fn section_sentinel_survives_html2text_paragraph() {
        let html = inject_section_sentinels(
            r#"<html><body><p>序章</p><h2 id="chapter-1">第一章</h2><p>正文</p></body></html>"#,
            &[SectionLabel {
                fragment: "chapter-1".to_string(),
                title: "第一章".to_string(),
            }],
        );
        let text = html2text::from_read(html.as_bytes(), 80).unwrap();
        let paras: Vec<&str> = text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        assert!(
            paras
                .iter()
                .any(|para| parse_section_sentinel(para) == Some(0)),
            "section sentinel must survive html2text round-trip: {:?}",
            paras
        );
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

    // ── inline reference expansion ──

    #[test]
    fn inlines_same_document_reference_marker() {
        let html = r##"
            <html><body>
                <p>正文<sup><a href="#note-1">[4]</a></sup>继续。</p>
                <ol><li id="note-1"><p>《汉书》卷六十五。</p></li></ol>
            </body></html>
        "##;

        let rendered = inline_reference_links(html, "Text/ch01.xhtml", |_| None);

        assert!(rendered.contains(&format!(
            "正文{INLINE_REF_OPEN}《汉书》卷六十五。{INLINE_REF_CLOSE}继续。"
        )));
        assert!(!rendered.contains(r##"href="#note-1""##));
    }

    #[test]
    fn inlines_same_resource_reference_marker_when_target_is_outside_slice() {
        let full_html = r##"
            <html><body>
                <h2 id="chapter-1">第一章</h2>
                <p>正文<sup><a href="#note-1">[4]</a></sup>继续。</p>
                <h2 id="chapter-2">第二章</h2>
                <ol><li id="note-1"><p>《汉书》卷六十五。</p></li></ol>
            </body></html>
        "##;
        let sliced = slice_resource_html(full_html, Some("chapter-1"), Some("chapter-2"));
        let rendered = inline_reference_links(&sliced, "Text/ch01.xhtml", |path| {
            (path == "Text/ch01.xhtml").then(|| full_html.to_string())
        });

        assert!(rendered.contains(&format!(
            "正文{INLINE_REF_OPEN}《汉书》卷六十五。{INLINE_REF_CLOSE}继续。"
        )));
    }

    #[test]
    fn inlines_same_resource_reference_marker() {
        let html = r##"
            <html><body>
                <p>正文<a href="notes.xhtml#n2">[15]</a>继续。</p>
            </body></html>
        "##;

        let rendered = inline_reference_links(html, "Text/ch01.xhtml", |path| {
            (path == "Text/notes.xhtml").then(|| {
                r##"<html><body><aside id="n2"><p>《资治通鉴》卷二百一十一。</p></aside></body></html>"##
                    .to_string()
            })
        });

        assert!(rendered.contains(&format!(
            "正文{INLINE_REF_OPEN}《资治通鉴》卷二百一十一。{INLINE_REF_CLOSE}继续。"
        )));
    }

    #[test]
    fn leaves_normal_internal_links_untouched() {
        let html = r##"
            <html><body>
                <p>参见<a href="#sec-2">第二章</a>。</p>
                <h2 id="sec-2">第二章</h2>
            </body></html>
        "##;

        let rendered = inline_reference_links(html, "Text/ch01.xhtml", |_| None);

        assert!(rendered.contains(r##"href="#sec-2""##));
        assert!(!rendered.contains(&format!("{INLINE_REF_OPEN}第二章{INLINE_REF_CLOSE}")));
    }

    #[test]
    fn inlines_definition_list_footnotes() {
        let html = r##"
            <html><body>
                <p>各种吐槽齐国人打仗不行的段子<sup><a id="back_note_2" href="#note_2" class="noteref">2</a></sup>。</p>
                <dl id="note_2" class="footnote">
                    <dt>[<a href="#back_note_2">←2</a>]</dt>
                    <dd><p>参看《战国歧途》齐国军事的部分。</p></dd>
                </dl>
            </body></html>
        "##;

        let rendered = inline_reference_links(html, "Text/ch01.xhtml", |_| None);

        assert!(rendered.contains(&format!(
            "各种吐槽齐国人打仗不行的段子{INLINE_REF_OPEN}参看《战国歧途》齐国军事的部分。{INLINE_REF_CLOSE}。"
        )));
    }

    #[test]
    fn inlines_paragraph_footnotes_with_inline_anchor_target() {
        let html = r##"
            <html><body>
                <p>以及藩镇与州县的关系也成为近来学者关注的另一个重点。<a id="fn12" href="../Text/part0005.xhtml#ft12"><sup>[12]</sup></a></p>
                <p class="kindle-cn-footnote"><a id="ft12" href="../Text/part0005.xhtml#fn12">[12]</a>这一领域早期的重要研究有：张达志《唐代后期藩镇与州之关系研究》。</p>
            </body></html>
        "##;

        let rendered = inline_reference_links(html, "Text/part0005.xhtml", |_| None);

        assert!(rendered.contains(&format!(
            "以及藩镇与州县的关系也成为近来学者关注的另一个重点。{INLINE_REF_OPEN}这一领域早期的重要研究有：张达志《唐代后期藩镇与州之关系研究》。{INLINE_REF_CLOSE}"
        )));
    }

    #[test]
    fn inline_reference_sentinels_survive_html2text() {
        let html = format!(
            "<html><body><p>前文{INLINE_REF_OPEN}注释内容{INLINE_REF_CLOSE}后文</p></body></html>"
        );
        let text = html2text::from_read(html.as_bytes(), 80).unwrap();

        assert!(text.contains(INLINE_REF_OPEN));
        assert!(text.contains(INLINE_REF_CLOSE));
    }
}
