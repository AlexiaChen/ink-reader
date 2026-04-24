use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem},
};

use crate::app::App;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let bmarks = app.bookmarks_for_current_book();

    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = if bmarks.is_empty() {
        vec![ListItem::new(
            "  (no bookmarks — press 's' in reading mode)",
        )]
    } else {
        bmarks
            .iter()
            .map(|bm| {
                let label = format!(" Ch.{} block {}", bm.chapter + 1, bm.block_index);
                ListItem::new(label)
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Bookmarks  (Enter=jump, d=delete, Esc=close) "),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, popup, &mut app.bookmark_state);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[0]);
    horizontal[0]
}
