use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event};
use ratatui::layout::Size;

mod app;
mod book;
mod formats;
mod storage;
mod ui;

use app::App;
use storage::book_id;

#[derive(Parser)]
#[command(name = "ink-reader", about = "Terminal TUI e-book reader", version)]
struct Args {
    /// Path to the e-book file (epub, mobi, azw3, pdf, txt)
    file: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let canonical = args.file.canonicalize().unwrap_or_else(|_| args.file.clone());

    let reader = formats::load_reader(&canonical)?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, reader, &canonical);
    ratatui::restore();

    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    reader: Box<dyn book::BookReader>,
    book_path: &std::path::Path,
) -> Result<()> {
    let mut app = App::new(reader, book_id(book_path))?;

    // Initial render — we need the terminal size to paginate
    let size = terminal.size()?;
    let tui_size = Size::new(size.width, size.height);
    app.load_chapter(0, tui_size);

    loop {
        app.tick_anim();
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let poll_ms: u64 = if app.anim.is_some() { 15 } else { 200 };
        if !event::poll(Duration::from_millis(poll_ms))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                let sz = terminal.size()?;
                app.handle_key(key, Size::new(sz.width, sz.height));
            }
            Event::Resize(w, h) => {
                app.on_resize(Size::new(w, h));
            }
            _ => {}
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
