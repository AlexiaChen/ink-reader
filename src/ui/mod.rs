use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
};

use crate::app::{App, Mode};

mod bookmarks;
mod copilot;
mod reader;
mod toc;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    match app.mode {
        Mode::CopilotPanel => {
            let layout = copilot_layout(area);
            if layout.side_by_side {
                reader::render(frame, app, layout.reader, true);
            }
            copilot::render(frame, app, layout.panel);
        }
        Mode::TocOverlay => {
            reader::render(frame, app, area, false);
            toc::render(frame, app, area);
        }
        Mode::BookmarkOverlay => {
            reader::render(frame, app, area, false);
            bookmarks::render(frame, app, area);
        }
        Mode::Reading => reader::render(frame, app, area, true),
    }
}

const MIN_SIDE_BY_SIDE_WIDTH: u16 = 90;
const MIN_PANEL_WIDTH: u16 = 40;
const MAX_PANEL_WIDTH: u16 = 64;

pub(crate) struct CopilotLayout {
    pub reader: Rect,
    pub panel: Rect,
    pub side_by_side: bool,
}

/// Keep the page visible beside Copilot whenever both panes remain readable.
/// On a narrow terminal the panel takes the full screen instead of producing
/// two unusably thin columns.
pub(crate) fn copilot_layout(area: Rect) -> CopilotLayout {
    if area.width < MIN_SIDE_BY_SIDE_WIDTH {
        return CopilotLayout {
            reader: area,
            panel: area,
            side_by_side: false,
        };
    }

    let panel_width = (area.width * 42 / 100).clamp(MIN_PANEL_WIDTH, MAX_PANEL_WIDTH);
    let columns = Layout::horizontal([
        Constraint::Length(area.width - panel_width),
        Constraint::Length(panel_width),
    ])
    .split(area);
    CopilotLayout {
        reader: columns[0],
        panel: columns[1],
        side_by_side: true,
    }
}

pub(crate) fn reader_size(size: Size, mode: Mode) -> Size {
    if mode != Mode::CopilotPanel {
        return size;
    }
    let layout = copilot_layout(Rect::new(0, 0, size.width, size.height));
    if layout.side_by_side {
        Size::new(layout.reader.width, layout.reader.height)
    } else {
        size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_terminal_uses_bounded_right_panel() {
        let layout = copilot_layout(Rect::new(0, 0, 160, 40));
        assert!(layout.side_by_side);
        assert_eq!(layout.reader.width + layout.panel.width, 160);
        assert_eq!(layout.panel.width, 64);
    }

    #[test]
    fn narrow_terminal_falls_back_to_full_panel() {
        let layout = copilot_layout(Rect::new(0, 0, 89, 30));
        assert!(!layout.side_by_side);
        assert_eq!(layout.panel, Rect::new(0, 0, 89, 30));
    }
}
