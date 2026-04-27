use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;

use crate::app::{AnimState, App};
use crate::book::{INLINE_REF_CLOSE, INLINE_REF_OPEN};

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    // Layout: status (1) | content (fill) | help (1)
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .split(area);

    render_status(frame, app, chunks[0]);
    render_content(frame, app, chunks[1]);
    render_help(frame, chunks[2]);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let meta = app.reader.meta();
    let chapters = &meta.chapters;

    let status = if app.showing_cover {
        format!(" {} │ Cover", meta.title)
    } else {
        let total_pages = app.pages.len().max(1);
        let current_page = app.current_page + 1;
        format!(
            " {} │ {} │ {}/{} pg  {}/{}  ch",
            meta.title,
            app.current_location_title(),
            current_page,
            total_pages,
            app.current_chapter + 1,
            chapters.len()
        )
    };

    let line = Line::from(Span::styled(
        status,
        Style::default().fg(Color::Black).bg(Color::Cyan),
    ));

    frame.render_widget(Paragraph::new(line), area);
}

fn render_content(frame: &mut Frame, app: &mut App, area: Rect) {
    // Render an image page (cover or embedded in-chapter image).
    if let Some(ref mut proto) = app.current_image {
        let caption: Vec<Line> = app
            .pages
            .get(app.current_page)
            .map(|page| stylize_inline_reference_lines(page.lines.iter().map(String::as_str)))
            .unwrap_or_default();

        let caption_height = (caption.len() as u16).min(area.height.saturating_sub(1));
        if caption_height == 0 {
            frame.render_stateful_widget(StatefulImage::<StatefulProtocol>::default(), area, proto);
            return;
        }

        let chunks =
            Layout::vertical([Constraint::Min(1), Constraint::Length(caption_height)]).split(area);

        frame.render_stateful_widget(
            StatefulImage::<StatefulProtocol>::default(),
            chunks[0],
            proto,
        );
        frame.render_widget(
            Paragraph::new(caption)
                .block(Block::default().borders(Borders::NONE))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
        return;
    }

    // Cover with no displayable image (e.g. terminal lacks graphics support).
    if app.showing_cover {
        let placeholder = Paragraph::new("[ Cover image — press → to start reading ]")
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(placeholder, area);
        return;
    }

    let text: Vec<Line> = if app.pages.is_empty() {
        vec![Line::from("Loading…")]
    } else if let Some(anim) = &app.anim {
        build_anim_frame(anim, app, area.height as usize)
    } else {
        app.pages
            .get(app.current_page)
            .map(|page| stylize_inline_reference_lines(page.lines.iter().map(String::as_str)))
            .unwrap_or_default()
    };

    let block = Block::default().borders(Borders::NONE);
    let para = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

/// Build a single animation frame: each line fans in from old → new content
/// at a staggered threshold so the page "rolls" top-down (forward) or
/// bottom-up (backward).
fn build_anim_frame<'a>(anim: &'a AnimState, app: &'a App, height: usize) -> Vec<Line<'a>> {
    let elapsed_ms = anim.start.elapsed().as_millis() as u64;
    let ratio = (elapsed_ms as f32 / anim.duration_ms as f32).clamp(0.0, 1.0);

    let new_lines = app
        .pages
        .get(app.current_page)
        .map(|p| p.lines.as_slice())
        .unwrap_or(&[]);

    let raw_lines: Vec<&str> = (0..height)
        .map(|i| {
            // Each line has a threshold in [0, 1) at which it switches to new content.
            // Forward: line 0 switches first (fan-down).
            // Backward: last line switches first (fan-up).
            let threshold = if height > 0 {
                let pos = if anim.forward { i } else { height - 1 - i };
                pos as f32 / height as f32
            } else {
                0.0
            };
            let s: &str = if ratio >= threshold {
                new_lines.get(i).map(|s| s.as_str()).unwrap_or("")
            } else {
                anim.old_lines.get(i).map(|s| s.as_str()).unwrap_or("")
            };
            s
        })
        .collect();

    stylize_inline_reference_lines(raw_lines)
}

fn stylize_inline_reference_lines<'a, I>(lines: I) -> Vec<Line<'a>>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut in_reference = false;
    lines
        .into_iter()
        .map(|line| stylize_inline_reference_line(line, &mut in_reference))
        .collect()
}

fn stylize_inline_reference_line<'a>(line: &'a str, in_reference: &mut bool) -> Line<'a> {
    let mut spans = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        if *in_reference {
            if let Some(end) = rest.find(INLINE_REF_CLOSE) {
                let note = rest[..end].trim_end();
                if !note.is_empty() {
                    spans.push(Span::styled(note, inline_reference_style()));
                }
                spans.push(Span::styled(")", inline_reference_bracket_style()));
                rest = rest[end + INLINE_REF_CLOSE.len_utf8()..].trim_start();
                *in_reference = false;
            } else {
                let note = rest.trim_end();
                if !note.is_empty() {
                    spans.push(Span::styled(note, inline_reference_style()));
                }
                rest = "";
            }
            continue;
        }

        let Some(start) = rest.find(INLINE_REF_OPEN) else {
            spans.push(Span::raw(rest));
            break;
        };
        let (before, after_start) = rest.split_at(start);
        if !before.is_empty() {
            spans.push(Span::raw(before));
        }
        spans.push(Span::styled("(", inline_reference_bracket_style()));
        rest = after_start[INLINE_REF_OPEN.len_utf8()..].trim_start();
        *in_reference = true;
    }

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

fn inline_reference_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::ITALIC)
}

fn inline_reference_bracket_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn render_help(frame: &mut Frame, area: Rect) {
    let help = " ↑ prev  ↓ next  n/p chapter  t ToC  b Bookmarks  s Save bookmark  q Quit";
    let line = Line::from(Span::styled(
        help,
        Style::default().fg(Color::DarkGray).bg(Color::Black),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_inline_references_with_parentheses() {
        let raw = format!("正文{INLINE_REF_OPEN}参看《战国歧途》{INLINE_REF_CLOSE}继续");
        let lines = stylize_inline_reference_lines([raw.as_str()]);
        let line = &lines[0];

        assert_eq!(line.spans.len(), 5);
        assert_eq!(line.spans[0].content.as_ref(), "正文");
        assert_eq!(line.spans[1].content.as_ref(), "(");
        assert_eq!(line.spans[2].content.as_ref(), "参看《战国歧途》");
        assert_eq!(line.spans[3].content.as_ref(), ")");
        assert_eq!(line.spans[4].content.as_ref(), "继续");
        assert!(line.spans[2].style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(line.spans[2].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn leaves_plain_text_lines_unchanged() {
        let lines = stylize_inline_reference_lines(["普通正文"]);
        let line = &lines[0];

        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content.as_ref(), "普通正文");
    }

    #[test]
    fn keeps_reference_state_across_wrapped_lines() {
        let first = format!("甲{INLINE_REF_OPEN}参看《战国");
        let second = format!("歧途》{INLINE_REF_CLOSE}乙");
        let lines = stylize_inline_reference_lines([first.as_str(), second.as_str()]);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "甲");
        assert_eq!(lines[0].spans[1].content.as_ref(), "(");
        assert_eq!(lines[0].spans[2].content.as_ref(), "参看《战国");
        assert_eq!(lines[1].spans[0].content.as_ref(), "歧途》");
        assert_eq!(lines[1].spans[1].content.as_ref(), ")");
        assert_eq!(lines[1].spans[2].content.as_ref(), "乙");
    }
}
