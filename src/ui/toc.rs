use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem},
};

use crate::app::App;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let popup = centered_rect(60, 80, area);
    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = app
        .reader
        .meta()
        .chapters
        .iter()
        .map(|c| ListItem::new(c.title.as_str()))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Table of Contents  (Enter=jump, Esc=close) "),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, popup, &mut app.toc_state);
}

/// Returns a Rect centred in `area` with given percentage width/height.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[0]);
    horizontal[0]
}
