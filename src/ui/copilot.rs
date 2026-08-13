use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::copilot::CopilotPhase;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let privacy = if app.copilot.config.is_local() {
        "LOCAL"
    } else {
        "REMOTE"
    };
    let title = format!(
        " Reading Copilot · {} · {} ",
        app.copilot.active_model, privacy
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.copilot.config.is_local() {
            Color::Cyan
        } else {
            Color::Yellow
        }))
        .title(title)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .split(inner);
    render_endpoint(frame, app, chunks[0]);
    match app.copilot.phase {
        CopilotPhase::Menu => render_menu(frame, chunks[1]),
        CopilotPhase::Input => render_input(frame, app, chunks[1]),
        CopilotPhase::Working | CopilotPhase::Answer => render_answer(frame, app, chunks[1]),
        CopilotPhase::Error => render_error(frame, app, chunks[1]),
    }
    render_footer(frame, app, chunks[2]);
}

fn render_endpoint(frame: &mut Frame, app: &App, area: Rect) {
    let privacy = if app.copilot.config.is_local() {
        app.copilot.config.endpoint_label()
    } else {
        format!(
            "{} · excerpt leaves this machine",
            app.copilot.config.endpoint_label()
        )
    };
    frame.render_widget(
        Paragraph::new(privacy).style(Style::default().fg(if app.copilot.config.is_local() {
            Color::DarkGray
        } else {
            Color::Yellow
        })),
        area,
    );
}

fn render_menu(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "Ask about the page on the left",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        shortcut("e", "Explain concepts, argument, and assumptions"),
        shortcut("t", "Translate accurately into Simplified Chinese"),
        shortcut("s", "Summarize for efficient study"),
        shortcut("r", "Deep mathematical / logical analysis"),
        shortcut("a", "Ask your own question"),
        Line::from(""),
        Line::from(Span::styled(
            "Only the visible page text is sent. Images and the rest of the book stay out of context.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn shortcut<'a>(key: &'a str, description: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  [{key}] "),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(description),
    ])
}

fn render_input(frame: &mut Frame, app: &App, area: Rect) {
    let prompt = Paragraph::new(app.copilot.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Question (Enter=send) "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(prompt, area);

    if area.width > 2 && area.height > 2 {
        let available = area.width.saturating_sub(3) as usize;
        let input_width = UnicodeWidthStr::width(app.copilot.input.as_str());
        let x = area.x + 1 + (input_width.min(available) as u16);
        let y = area.y + 1 + (input_width / available.max(1)) as u16;
        frame.set_cursor_position((
            x.min(area.right().saturating_sub(2)),
            y.min(area.bottom() - 2),
        ));
    }
}

fn render_answer(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.copilot.answer.is_empty() {
        "Waiting for the model…"
    } else {
        app.copilot.answer.as_str()
    };
    let answer = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((app.copilot.scroll, 0));
    frame.render_widget(answer, area);
}

fn render_error(frame: &mut Frame, app: &App, area: Rect) {
    let error = Paragraph::new(app.copilot.error.as_str())
        .style(Style::default().fg(Color::LightRed))
        .wrap(Wrap { trim: false });
    frame.render_widget(error, area);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let help = match app.copilot.phase {
        CopilotPhase::Menu => " e/t/s/r action · a ask · Esc close",
        CopilotPhase::Input => " Enter send · Backspace edit · Esc close",
        CopilotPhase::Working => " x cancel · j/k scroll · Esc close",
        CopilotPhase::Answer => " j/k scroll · a follow-up · r retry · Esc close",
        CopilotPhase::Error => " r retry · m menu · Esc close",
    };
    let status = if app.copilot.status.is_empty() {
        help.to_string()
    } else {
        format!(" {}  │{}", app.copilot.status, help)
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
