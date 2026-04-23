use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

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
    let chapter_title = chapters
        .get(app.current_chapter)
        .map(|c| c.title.as_str())
        .unwrap_or("");

    let total_pages = app.pages.len().max(1);
    let current_page = app.current_page + 1;

    let status = format!(
        " {} │ {} │ {}/{} pg  {}/{}  ch",
        meta.title,
        chapter_title,
        current_page,
        total_pages,
        app.current_chapter + 1,
        chapters.len()
    );

    let line = Line::from(Span::styled(
        status,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan),
    ));

    frame.render_widget(Paragraph::new(line), area);
}

fn render_content(frame: &mut Frame, app: &App, area: Rect) {
    let text: Vec<Line> = if app.pages.is_empty() {
        vec![Line::from("Loading…")]
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

    let block = Block::default()
        .borders(Borders::NONE);

    let para = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(para, area);
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
