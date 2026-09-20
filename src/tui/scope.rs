//! The scope picker behind `a` and `d` in the skills and projects tabs (cyan):
//! add or remove for this cell, its row or its column, each choice named by the
//! real skill, file, agent or project and carrying the changes it would make.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::widgets::*;
use crate::plan::Change;

pub(crate) struct Scope {
    pub(crate) remove: bool,
    /// The word the box opens with: `Add`, `Link` or `Remove`.
    pub(crate) verb: &'static str,
    pub(crate) items: Vec<(String, Vec<Change>)>,
    pub(crate) picked: usize,
}

pub(crate) enum Picked {
    Pending,
    Cancelled,
    Chosen(String, Vec<Change>),
}

impl Scope {
    pub(super) fn key(&mut self, key: KeyEvent) -> Picked {
        use KeyCode::*;
        match key.code {
            Down | Char('j') => {
                self.picked = (self.picked + 1).min(self.items.len().saturating_sub(1))
            }
            Up | Char('k') => self.picked = self.picked.saturating_sub(1),
            Enter => {
                let (label, changes) = self.items[self.picked].clone();
                return Picked::Chosen(label, changes);
            }
            Esc | Char('q') => return Picked::Cancelled,
            _ => {}
        }
        Picked::Pending
    }
}

pub(super) fn render_scope(f: &mut Frame, area: Rect, s: &Scope) {
    let width = box_width(area.width);
    let inner = box_inner_width(width);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines = vec![Line::raw(format!("{}…", s.verb)), Line::raw("")];
    let mut rows = 2u16;
    for (i, (label, changes)) in s.items.iter().enumerate() {
        let count = match changes.len() {
            0 => "  (nothing to do)".to_string(),
            1 => "  (1 change)".to_string(),
            n => format!("  ({n} changes)"),
        };
        let style = if i == s.picked {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        rows += wrapped_line_count(&format!(" {label}{count}"), inner) as u16;
        lines.push(Line::from(vec![
            Span::styled(format!(" {label} "), style),
            Span::styled(count, dim),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(box_hint("j/k ↑↓ move · enter pick · esc cancel"));
    let rect = box_area(area, width, box_height(rows + 2, area.height));
    f.render_widget(Clear, rect);
    let para = Paragraph::new(lines)
        .block(box_block(Color::Cyan, &s.verb.to_lowercase()))
        .wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}
