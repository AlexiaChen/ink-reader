use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MAX_FORMULA_CHARS: usize = 4096;
const MAX_FORMULA_NESTING: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerLineKind {
    Text,
    Heading,
    Code,
    Math,
    Dim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerLine {
    pub text: String,
    pub kind: AnswerLineKind,
}

#[derive(Clone)]
struct CachedFormula {
    rows: Vec<String>,
    width: usize,
    height: usize,
}

thread_local! {
    static FORMULA_CACHE: RefCell<HashMap<String, Option<CachedFormula>>> =
        RefCell::new(HashMap::new());
}

#[derive(Debug)]
struct ListState {
    next: Option<u64>,
}

/// Render Copilot Markdown into terminal-sized visual lines. Pulldown-cmark
/// recognizes completed `$...$` and `$$...$$` regions while leaving an
/// unfinished streaming delimiter as ordinary text. Multi-row inline formulae
/// become their own visual block so surrounding prose remains readable.
pub fn render_markdown(markdown: &str, width: usize) -> Vec<AnswerLine> {
    let width = width.max(1);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_MATH);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let mut lines = Vec::new();
    let mut pending = String::new();
    let mut pending_kind = AnswerLineKind::Text;
    let mut list_stack: Vec<ListState> = Vec::new();
    let mut in_code_block = false;

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { .. } => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    pending_kind = AnswerLineKind::Heading;
                }
                Tag::CodeBlock(_) => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    in_code_block = true;
                    pending_kind = AnswerLineKind::Code;
                }
                Tag::List(start) => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    list_stack.push(ListState { next: start });
                }
                Tag::Item => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    let indent = "  ".repeat(list_stack.len().saturating_sub(1));
                    let marker = if let Some(state) = list_stack.last_mut() {
                        match state.next.as_mut() {
                            Some(next) => {
                                let marker = format!("{next}. ");
                                *next += 1;
                                marker
                            }
                            None => "• ".to_string(),
                        }
                    } else {
                        "• ".to_string()
                    };
                    pending.push_str(&indent);
                    pending.push_str(&marker);
                }
                Tag::BlockQuote(_) => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    pending.push_str("│ ");
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Item => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    push_blank(&mut lines);
                }
                TagEnd::Heading(_) => {
                    flush_pending(&mut lines, &mut pending, width, AnswerLineKind::Heading);
                    pending_kind = AnswerLineKind::Text;
                    push_blank(&mut lines);
                }
                TagEnd::CodeBlock => {
                    flush_pending(&mut lines, &mut pending, width, AnswerLineKind::Code);
                    pending_kind = AnswerLineKind::Text;
                    in_code_block = false;
                    push_blank(&mut lines);
                }
                TagEnd::List(_) => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    list_stack.pop();
                }
                TagEnd::BlockQuote(_) => {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    push_blank(&mut lines);
                }
                _ => {}
            },
            Event::Text(text) => pending.push_str(&text),
            Event::Code(code) => {
                pending.push('`');
                pending.push_str(&code);
                pending.push('`');
            }
            Event::InlineMath(latex) => {
                if let Some(formula) = cached_formula(&latex)
                    && formula.height == 1
                    && formula.width <= width
                {
                    pending.push_str(formula.rows[0].trim_end());
                } else {
                    flush_pending(&mut lines, &mut pending, width, pending_kind);
                    push_formula(&mut lines, &latex, width);
                }
            }
            Event::DisplayMath(latex) => {
                flush_pending(&mut lines, &mut pending, width, pending_kind);
                push_formula(&mut lines, &latex, width);
                push_blank(&mut lines);
            }
            Event::SoftBreak => {
                if in_code_block {
                    pending.push('\n');
                } else if !pending.chars().last().is_some_and(char::is_whitespace) {
                    pending.push(' ');
                }
            }
            Event::HardBreak => {
                flush_pending(&mut lines, &mut pending, width, pending_kind);
            }
            Event::Rule => {
                flush_pending(&mut lines, &mut pending, width, pending_kind);
                lines.push(AnswerLine {
                    text: "─".repeat(width),
                    kind: AnswerLineKind::Dim,
                });
            }
            Event::TaskListMarker(done) => pending.push_str(if done { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(note) => {
                pending.push('[');
                pending.push_str(&note);
                pending.push(']');
            }
            Event::Html(html) | Event::InlineHtml(html) => pending.push_str(&html),
        }
    }

    flush_pending(&mut lines, &mut pending, width, pending_kind);
    while lines.last().is_some_and(|line| line.text.is_empty()) {
        lines.pop();
    }
    lines
}

fn flush_pending(
    lines: &mut Vec<AnswerLine>,
    pending: &mut String,
    width: usize,
    kind: AnswerLineKind,
) {
    if pending.is_empty() {
        return;
    }

    for logical_line in pending.split('\n') {
        if logical_line.is_empty() {
            push_blank(lines);
            continue;
        }
        let options = textwrap::Options::new(width)
            .break_words(true)
            .word_separator(textwrap::WordSeparator::UnicodeBreakProperties);
        for wrapped in textwrap::wrap(logical_line, &options) {
            lines.push(AnswerLine {
                text: wrapped.into_owned(),
                kind,
            });
        }
    }
    pending.clear();
}

fn push_formula(lines: &mut Vec<AnswerLine>, latex: &str, width: usize) {
    let Some(formula) = cached_formula(latex) else {
        push_latex_fallback(lines, latex, width, "unsupported or unsafe formula");
        return;
    };

    if formula.width > width {
        for row in &formula.rows {
            lines.push(AnswerLine {
                text: truncate_width(row, width),
                kind: AnswerLineKind::Math,
            });
        }
        push_latex_fallback(lines, latex, width, "formula wider than panel");
        return;
    }

    let left_pad = (width - formula.width) / 2;
    for row in &formula.rows {
        lines.push(AnswerLine {
            text: format!("{}{row}", " ".repeat(left_pad)),
            kind: AnswerLineKind::Math,
        });
    }
}

fn push_latex_fallback(lines: &mut Vec<AnswerLine>, latex: &str, width: usize, reason: &str) {
    lines.push(AnswerLine {
        text: format!("LaTeX ({reason}):"),
        kind: AnswerLineKind::Dim,
    });
    let options = textwrap::Options::new(width)
        .break_words(true)
        .word_separator(textwrap::WordSeparator::UnicodeBreakProperties);
    for wrapped in textwrap::wrap(latex, &options) {
        lines.push(AnswerLine {
            text: wrapped.into_owned(),
            kind: AnswerLineKind::Code,
        });
    }
}

fn cached_formula(latex: &str) -> Option<CachedFormula> {
    FORMULA_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(latex) {
            return cached.clone();
        }

        let rendered = render_formula(latex);
        let mut cache = cache.borrow_mut();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(latex.to_string(), rendered.clone());
        rendered
    })
}

fn render_formula(latex: &str) -> Option<CachedFormula> {
    if !formula_is_reasonable(latex) {
        return None;
    }
    let block = catch_unwind(AssertUnwindSafe(|| term_maths::render(latex))).ok()?;
    if block.is_empty() {
        return None;
    }
    let rows = block
        .cells()
        .iter()
        .map(|row| row.concat())
        .collect::<Vec<_>>();
    Some(CachedFormula {
        rows,
        width: block.width(),
        height: block.height(),
    })
}

fn formula_is_reasonable(latex: &str) -> bool {
    if latex.trim().is_empty() || latex.chars().count() > MAX_FORMULA_CHARS {
        return false;
    }
    let mut depth = 0usize;
    for ch in latex.chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth > MAX_FORMULA_NESTING {
                    return false;
                }
            }
            '}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    depth == 0
}

fn truncate_width(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.trim_end().to_string();
    }
    if width == 0 {
        return String::new();
    }

    let target = width.saturating_sub(1);
    let mut used = 0usize;
    let mut output = String::new();
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > target {
            break;
        }
        output.push(ch);
        used += ch_width;
    }
    output.push('…');
    output
}

fn push_blank(lines: &mut Vec<AnswerLine>) {
    if !lines.last().is_some_and(|line| line.text.is_empty()) {
        lines.push(AnswerLine {
            text: String::new(),
            kind: AnswerLineKind::Text,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(markdown: &str, width: usize) -> Vec<String> {
        render_markdown(markdown, width)
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    #[test]
    fn renders_single_row_inline_math_without_dollar_markers() {
        let lines = texts("The identity is $x^2 + y^2 = z^2$.", 80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("The identity is "));
        assert!(!lines[0].contains('$'));
        assert!(lines[0].contains('²'));
    }

    #[test]
    fn inserts_multirow_inline_formula_between_surrounding_text() {
        let lines = texts("Before $\\frac{a}{b}$ after.", 30);
        let before = lines
            .iter()
            .position(|line| line.trim() == "Before")
            .unwrap();
        let fraction = lines.iter().position(|line| line.contains('─')).unwrap();
        let after = lines
            .iter()
            .position(|line| line.trim() == "after.")
            .unwrap();
        assert!(before < fraction && fraction < after);
    }

    #[test]
    fn centers_display_formula_and_keeps_order() {
        let rendered = render_markdown("First.\n\n$$\\frac{1}{2}$$\n\nLast.", 20);
        let math = rendered
            .iter()
            .find(|line| line.kind == AnswerLineKind::Math && line.text.contains('─'))
            .unwrap();
        assert!(math.text.starts_with(' '));
        assert_eq!(rendered.first().unwrap().text, "First.");
        assert_eq!(rendered.last().unwrap().text, "Last.");
    }

    #[test]
    fn unfinished_streaming_delimiter_stays_visible_text() {
        let lines = texts("The next expression is $\\frac{a}", 80);
        assert!(lines.join("\n").contains("$\\frac{a}"));
    }

    #[test]
    fn code_dollar_markers_are_not_rendered_as_math() {
        let lines = texts("Use `$x$` literally.", 80);
        assert!(lines.join("\n").contains("`$x$`"));
    }

    #[test]
    fn overwide_formula_keeps_latex_source_as_fallback() {
        let lines = texts("$$a+b+c+d+e+f+g+h+i+j+k$$", 8);
        assert!(lines.iter().any(|line| line.starts_with("LaTeX")));
        assert!(lines.join("").contains("a+b+c"));
    }

    #[test]
    fn renders_common_reasoning_notation_and_matrices() {
        for latex in [
            r"\sum_{i=1}^{n} x_i",
            r"\int_{0}^{1} x^2 dx",
            r"\sqrt{b^2 - 4ac}",
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        ] {
            let rendered = render_formula(latex).expect("common formula should render");
            assert!(!rendered.rows.is_empty());
            assert!(rendered.width > 0);
        }
    }

    #[test]
    fn renders_the_real_qwen_math_response_shape() {
        let answer = "对于一元二次方程 $ax^2+bx+c=0\\;(a \\neq 0)$，其求根公式为：\n$$x = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}$$";
        let lines = render_markdown(answer, 50);
        assert!(lines.iter().all(|line| !line.text.contains('$')));
        assert!(
            lines
                .iter()
                .any(|line| line.kind == AnswerLineKind::Math && line.text.contains('─'))
        );
        assert!(lines.iter().any(|line| line.text.contains('²')));
    }

    #[test]
    fn unbalanced_braces_use_safe_fallback() {
        for latex in [r"\frac{a}{b", r"a}"] {
            assert!(!formula_is_reasonable(latex));
        }
    }

    #[test]
    fn excessive_nesting_uses_safe_fallback() {
        let formula = format!("{}x{}", "{".repeat(65), "}".repeat(65));
        let markdown = format!("$${formula}$$");
        let lines = texts(&markdown, 40);
        assert!(lines[0].contains("unsafe"));
    }
}
