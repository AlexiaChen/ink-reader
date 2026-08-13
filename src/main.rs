use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, Event};
use ratatui::layout::Size;

mod app;
mod book;
mod copilot;
mod formats;
mod math_render;
mod storage;
mod ui;

use app::App;
use storage::book_id;

#[derive(Parser)]
#[command(name = "ink-reader", about = "Terminal TUI e-book reader", version)]
struct Args {
    /// Path to the e-book file (epub, pdf, txt)
    file: PathBuf,

    /// Ollama-compatible API base URL (env: INK_READER_OLLAMA_URL)
    #[arg(long)]
    ollama_url: Option<String>,

    /// Model for explain, translate, summarize, and questions
    #[arg(long)]
    copilot_model: Option<String>,

    /// Optional separate model for deep reasoning (defaults to copilot-model)
    #[arg(long)]
    copilot_reasoning_model: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let canonical = args
        .file
        .canonicalize()
        .unwrap_or_else(|_| args.file.clone());

    let reader = formats::load_reader(&canonical)?;

    let mut terminal = ratatui::init();
    let copilot_config = copilot::CopilotConfig::from_overrides(
        args.ollama_url,
        args.copilot_model,
        args.copilot_reasoning_model,
    );
    let result = run(&mut terminal, reader, &canonical, copilot_config);
    ratatui::restore();

    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    reader: Box<dyn book::BookReader>,
    book_path: &std::path::Path,
    copilot_config: copilot::CopilotConfig,
) -> Result<()> {
    let mut app = App::new_with_copilot(reader, book_id(book_path), copilot_config)?;

    // Initial render — we need the terminal size to paginate
    let size = terminal.size()?;
    let tui_size = Size::new(size.width, size.height);
    app.load_chapter(0, tui_size);

    // The app is keyboard-only. Explicitly disable mouse capture so inherited
    // terminal mouse-reporting sequences cannot leak through as fake keypresses.
    let mut stdout = std::io::stdout();
    disable_mouse_capture(&mut stdout)?;

    loop {
        app.tick();
        if app.take_terminal_clear_request() {
            terminal.clear()?;
        }
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let poll_ms: u64 = if app.anim.is_some() || app.copilot.is_working() {
            15
        } else {
            200
        };
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
            Event::Mouse(_) => {}
            _ => {}
        }

        if let Some(err) = app.take_pending_error() {
            return Err(err);
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn disable_mouse_capture<W: std::io::Write>(writer: &mut W) -> std::io::Result<()> {
    crossterm::execute!(writer, DisableMouseCapture)
}

#[cfg(test)]
mod tests {
    use super::disable_mouse_capture;

    #[test]
    fn disable_mouse_capture_emits_disable_sequences() {
        let mut out = Vec::new();
        disable_mouse_capture(&mut out).unwrap();
        let ansi = String::from_utf8(out).unwrap();

        assert!(ansi.contains("\u{1b}[?1000l"));
        assert!(ansi.contains("\u{1b}[?1006l"));
    }
}
