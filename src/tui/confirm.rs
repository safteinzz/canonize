//! Yes/No boxes: the gate in front of removing links (red, starts on No), and
//! the offers (cyan, start on Yes) to apply, set up or adopt.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::widgets::*;
use crate::plan::Change;
use crate::setup::Choice;

/// What a Yes runs.
pub(crate) enum Action {
    Changes(Vec<Change>),
    Setup(Choice),
}

pub(crate) struct Confirm {
    pub(crate) title: String,
    pub(crate) message: String,
    /// The shell commands a Yes amounts to, drawn in green under the message.
    pub(crate) runs: Vec<String>,
    pub(crate) action: Action,
    /// A gate rather than an offer, which decides the colour and the default.
    pub(crate) danger: bool,
    pub(crate) yes: bool,
}

pub(crate) enum Answer {
    Yes,
    No,
    Pending,
}

impl Confirm {
    pub(super) fn gate(title: &str, message: String, action: Action) -> Confirm {
        Confirm {
            title: title.to_string(),
            message,
            runs: Vec::new(),
            action,
            danger: true,
            yes: false,
        }
    }

    pub(super) fn offer(title: &str, message: String, action: Action) -> Confirm {
        Confirm {
            title: title.to_string(),
            message,
            runs: Vec::new(),
            action,
            danger: false,
            yes: true,
        }
    }

    /// `y` proceeds and `n`/Esc cancels outright, or move between the buttons
    /// and press Enter. Any other key is swallowed.
    pub(super) fn runs(mut self, runs: Vec<String>) -> Confirm {
        self.runs = runs;
        self
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Answer {
        use KeyCode::*;
        match key.code {
            Left | Right | Char('h' | 'l') | Tab | BackTab => {
                self.yes = !self.yes;
                Answer::Pending
            }
            Char('n' | 'N') | Esc => Answer::No,
            Char('y' | 'Y') => Answer::Yes,
            Enter if self.yes => Answer::Yes,
            Enter => Answer::No,
            _ => Answer::Pending,
        }
    }
}

pub(super) fn render_confirm(f: &mut Frame, area: Rect, c: &Confirm) {
    let accent = if c.danger { Color::Red } else { Color::Cyan };
    let width = box_width(area.width);
    let inner = box_inner_width(width);
    // Each command gets its own lines, broken between arguments with a `\`
    // and indented, so a long path never wraps into the next command.
    let run_lines: Vec<String> = c
        .runs
        .iter()
        .flat_map(|r| r.lines())
        .flat_map(|cmd| shell_lines(cmd, inner.saturating_sub(6)))
        .enumerate()
        .map(|(i, r)| format!("{}{r}", if i == 0 { "runs  " } else { "      " }))
        .collect();
    let mut rows = wrapped_rows(&c.message, inner);
    if !run_lines.is_empty() {
        rows += 1 + run_lines
            .iter()
            .map(|l| wrapped_line_count(l, inner) as u16)
            .sum::<u16>();
    }
    // The message, the commands, a blank, the buttons, a blank, the keys.
    let rect = box_area(area, width, box_height(rows + 4, area.height));
    f.render_widget(Clear, rect);

    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines: Vec<Line> = c
        .message
        .lines()
        .map(|l| Line::raw(l.to_string()))
        .collect();
    if !run_lines.is_empty() {
        lines.push(Line::raw(""));
        for l in &run_lines {
            let (head, cmd) = l.split_at(6);
            lines.push(Line::from(vec![
                Span::styled(head.to_string(), dim),
                Span::styled(cmd.to_string(), Style::default().fg(Color::Green)),
            ]));
        }
    }
    lines.extend([
        Line::raw(""),
        box_buttons(accent, c.yes),
        Line::raw(""),
        box_hint("h/l ←/→ move · enter select · y/n"),
    ]);
    let para = Paragraph::new(lines)
        .block(box_block(accent, &c.title))
        .wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

/// Split `cmd` into words, keeping a single-quoted argument whole.
fn words(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in cmd.chars() {
        match ch {
            '\'' => {
                quoted = !quoted;
                cur.push(ch);
            }
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `cmd` laid out in lines of at most `width` columns, broken between words
/// with a trailing `\` and the continuation indented, the way a person would
/// write it in a shell.
fn shell_lines(cmd: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in words(cmd) {
        let indent = if lines.is_empty() { "" } else { "    " };
        let need = if cur.is_empty() {
            indent.len() + w.chars().count()
        } else {
            cur.chars().count() + 1 + w.chars().count()
        };
        // Two columns are kept free for the ` \` a broken line ends with.
        if !cur.is_empty() && need + 2 > width {
            lines.push(format!("{cur} \\"));
            cur = format!("    {w}");
        } else if cur.is_empty() {
            cur = format!("{indent}{w}");
        } else {
            cur.push(' ');
            cur.push_str(&w);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}
