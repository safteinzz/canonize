//! The typed gate (red, no buttons, one field): in front of deleting what
//! exists nowhere else. Enter does nothing until the field holds the thing's
//! exact name, which the body shows so it is copied rather than guessed.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::widgets::*;
use crate::plan::Change;

pub(crate) struct Typed {
    pub(crate) title: String,
    pub(crate) message: String,
    /// What has to be typed.
    pub(crate) name: String,
    pub(crate) input: String,
    pub(crate) changes: Vec<Change>,
}

pub(crate) enum Typing {
    Pending,
    Cancelled,
    Confirmed,
}

impl Typed {
    pub(super) fn key(&mut self, key: KeyEvent) -> Typing {
        match key.code {
            KeyCode::Esc => return Typing::Cancelled,
            KeyCode::Enter if self.input == self.name => return Typing::Confirmed,
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(c)
            }
            _ => {}
        }
        Typing::Pending
    }
}

pub(super) fn render_typed(f: &mut Frame, area: Rect, t: &Typed) {
    let width = box_width(area.width);
    let inner = box_inner_width(width);
    let runs: Vec<String> = t.changes.iter().map(Change::command).collect();
    let field = format!("type {}:  {}█", t.name, t.input);
    // Every line is counted wrapped: an unwrapped count is what clips the
    // field and the key line off a gate and leaves it looking unanswerable.
    let rows = wrapped_rows(&t.message, inner)
        + 1
        + runs
            .iter()
            .map(|r| wrapped_line_count(&format!("runs  {r}"), inner) as u16)
            .sum::<u16>()
        + 1
        + wrapped_line_count(&field, inner) as u16
        + 2;
    let rect = box_area(area, width, box_height(rows, area.height));
    f.render_widget(Clear, rect);

    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines: Vec<Line> = t
        .message
        .lines()
        .map(|l| Line::raw(l.to_string()))
        .collect();
    lines.push(Line::raw(""));
    for (i, r) in runs.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(if i == 0 { "runs  " } else { "      " }, dim),
            Span::styled(r.clone(), Style::default().fg(Color::Red)),
        ]));
    }
    lines.push(Line::raw(""));
    let ok = t.input == t.name;
    lines.push(Line::from(vec![
        Span::raw(format!("type {}:  ", t.name)),
        Span::styled(
            t.input.clone(),
            Style::default().add_modifier(Modifier::BOLD).fg(if ok {
                Color::Red
            } else {
                Color::Reset
            }),
        ),
        Span::raw("█"),
    ]));
    lines.push(Line::raw(""));
    lines.push(box_hint(if ok {
        "enter delete · esc cancel"
    } else {
        "type the name to unlock enter · esc cancel"
    }));
    let para = Paragraph::new(lines)
        .block(box_block(Color::Red, &t.title))
        .wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}
