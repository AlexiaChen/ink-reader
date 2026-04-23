use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;

use crate::app::{AnimState, App};

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
        let chapter_title = chapters
            .get(app.current_chapter)
            .map(|c| c.title.as_str())
            .unwrap_or("");
        let total_pages = app.pages.len().max(1);
        let current_page = app.current_page + 1;
        format!(
            " {} │ {} │ {}/{} pg  {}/{}  ch",
            meta.title,
            chapter_title,
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
        frame.render_stateful_widget(StatefulImage::<StatefulProtocol>::default(), area, proto);
        return;
    }

    // Cover with no displayable image (e.g. terminal lacks graphics support).
    if app.showing_cover {
        let placeholder =
            Paragraph::new("[ Cover image — press → to start reading ]")
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
            .map(|page| {
                page.lines
                    .iter()
                    .map(|l| Line::from(l.as_str()))
                    .collect()
            })
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

    let new_lines = app.pages
        .get(app.current_page)
        .map(|p| p.lines.as_slice())
        .unwrap_or(&[]);

    (0..height)
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
            Line::from(s)
        })
        .collect()
}

fn render_help(frame: &mut Frame, area: Rect) {
    let help = " ↑ prev  ↓ next  n/p chapter  t ToC  b Bookmarks  a Add bookmark  q Quit";
    let line = Line::from(Span::styled(
        help,
        Style::default()
            .fg(Color::DarkGray)
            .bg(Color::Black),
    ));
    frame.render_widget(Paragraph::new(line), area);
}
