use ratatui::Frame;

use crate::app::{App, Mode};

mod bookmarks;
mod reader;
mod toc;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    reader::render(frame, app, area);

    match app.mode {
        Mode::TocOverlay => toc::render(frame, app, area),
        Mode::BookmarkOverlay => bookmarks::render(frame, app, area),
        Mode::Reading => {}
    }
}
