//! Boxes that are only read: the alert, for a failure that already happened
//! (yellow), and the reader, for a body worth scrolling with nothing at stake
//! (cyan). Both scroll and own every key until dismissed.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::widgets::*;

pub(crate) struct Note {
    title: String,
    body: String,
    colour: Color,
    scroll: u16,
}

impl Note {
    pub(super) fn alert(title: impl Into<String>, body: impl Into<String>) -> Note {
        Note {
            title: title.into(),
            body: body.into(),
            colour: Color::Yellow,
            scroll: 0,
        }
    }

    pub(super) fn reader(title: impl Into<String>, body: impl Into<String>) -> Note {
        Note {
            colour: Color::Cyan,
            ..Note::alert(title, body)
        }
    }

    /// Whether the key closed it. Movement scrolls; anything else is swallowed,
    /// so a stray keypress cannot close a message before it has been read.
    pub(super) fn key(&mut self, key: KeyEvent) -> bool {
        use KeyCode::*;
        match key.code {
            // Stop at the last line: scrolling past it shows an empty box.
            Down | Char('j') | PageDown => {
                let last = self.body.lines().count().saturating_sub(1) as u16;
                self.scroll = self.scroll.saturating_add(1).min(last);
            }
            Up | Char('k') | PageUp => self.scroll = self.scroll.saturating_sub(1),
            Esc | Enter | Char('q' | ' ') => return true,
            _ => {}
        }
        false
    }
}

pub(super) fn render_note(f: &mut Frame, area: Rect, n: &Note) {
    let width = box_width(area.width);
    let rows = wrapped_rows(&n.body, box_inner_width(width));
    // The body, a blank, the keys.
    let rect = box_area(area, width, box_height(rows + 2, area.height));
    f.render_widget(Clear, rect);

    let mut lines: Vec<Line> = n.body.lines().map(|l| Line::raw(l.to_string())).collect();
    lines.push(Line::raw(""));
    lines.push(box_hint("j/k ↑↓ scroll · esc close"));

    let para = Paragraph::new(lines)
        .block(box_block(n.colour, &n.title))
        .wrap(Wrap { trim: false })
        .scroll((n.scroll, 0));
    f.render_widget(para, rect);
}
