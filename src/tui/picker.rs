//! The picker a form opens with `ctrl-o` (cyan): the choices the app can list
//! for one field, narrowed with `/` as any list is.

use std::cell::Cell;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph};

use super::line_edit;
use super::widgets::*;

/// What marks a choice that is a folder and not a project yet.
const NOT_YET: &str = " not a project yet";

/// Columns `h/l` and the arrows pan a choice too wide for the box.
const PAN: usize = 8;

pub(super) struct Picker {
    /// What there is to pick, each with whether it is a folder that is not a
    /// project yet.
    choices: Vec<(String, bool)>,
    /// The cursor, as an index into `rows`.
    at: usize,
    /// The first column shown, once a choice is wider than the box.
    left: usize,
    query: String,
    /// The cursor in `query`, as characters after it (`line_edit::edit`).
    back: usize,
    /// Whether keys are going into `query` rather than to the list.
    typing: bool,
    /// The rows the list showed last frame, which `ctrl-d/u` page by.
    page: Cell<usize>,
    /// The columns the list showed last frame, which panning stops at.
    view: Cell<usize>,
}

pub(super) enum Pick {
    Pending,
    Cancelled,
    Chose(String),
}

impl Picker {
    /// The picker over `choices`, its cursor on the one named `current` when
    /// there is one.
    pub(super) fn new(choices: Vec<(String, bool)>, current: &str) -> Picker {
        let at = choices.iter().position(|c| c.0 == current).unwrap_or(0);
        Picker {
            choices,
            at,
            left: 0,
            query: String::new(),
            back: 0,
            typing: false,
            page: Cell::new(0),
            view: Cell::new(0),
        }
    }

    /// The choices the query keeps, by index into `choices`.
    fn rows(&self) -> Vec<usize> {
        super::kept(&self.query, (0..self.choices.len()).collect(), |i| {
            self.choices[i].0.clone()
        })
    }

    /// The columns the widest choice takes, mark included.
    fn widest(&self) -> usize {
        self.choices
            .iter()
            .map(|(name, folder)| line(name, *folder).chars().count())
            .max()
            .unwrap_or(0)
    }

    /// Change the query through `change`, leaving the cursor on its choice
    /// while the query still keeps it, else on the first row.
    fn requery(&mut self, change: impl FnOnce(&mut Picker)) {
        let on = self.rows().get(self.at).copied();
        change(self);
        let rows = self.rows();
        self.at = on
            .and_then(|c| rows.iter().position(|&r| r == c))
            .unwrap_or(0);
    }

    fn step(&mut self, down: bool) {
        let last = self.rows().len().saturating_sub(1);
        self.at = if down {
            (self.at + 1).min(last)
        } else {
            self.at.saturating_sub(1)
        };
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Pick {
        use KeyCode::*;
        let clear = |p: &mut Picker| {
            p.query.clear();
            p.back = 0;
        };
        if self.typing {
            match key.code {
                Esc => {
                    self.requery(clear);
                    self.typing = false;
                }
                Enter => self.typing = false,
                Down | Up => self.step(key.code == Down),
                _ => self.requery(|p| {
                    line_edit::edit(&mut p.query, &mut p.back, key);
                }),
            }
            return Pick::Pending;
        }
        if let Some(to) = super::motion(key, self.at, self.rows().len(), self.page.get()) {
            self.at = to;
            return Pick::Pending;
        }
        let most = self.widest().saturating_sub(self.view.get());
        match key.code {
            Left | Char('h') => self.left = self.left.saturating_sub(PAN),
            Right | Char('l') => self.left = (self.left + PAN).min(most),
            Char('/') => {
                self.requery(clear);
                self.typing = true;
            }
            Enter => {
                if let Some(&i) = self.rows().get(self.at) {
                    return Pick::Chose(self.choices[i].0.clone());
                }
            }
            Esc if !self.query.is_empty() => self.requery(clear),
            Esc => return Pick::Cancelled,
            _ => {}
        }
        Pick::Pending
    }
}

/// One choice as the list draws it.
fn line(name: &str, folder: bool) -> String {
    format!(" {name} {}", if folder { NOT_YET } else { "" })
}

/// `picker` under `title`, scrolled so its cursor is on screen, each choice
/// marked when it is a folder that is not a project yet. The box is sized from
/// every choice, so typing a query never resizes it: as wide as the widest up
/// to the whole screen, panned sideways past that.
pub(super) fn render_picker(f: &mut Frame, area: Rect, title: &str, picker: &Picker) {
    let widest = picker.widest();
    let width = box_width(area.width)
        .max(u16::try_from(widest + usize::from(BOX_CHROME_W)).unwrap_or(u16::MAX))
        .min(area.width);
    // The choices, then a blank and the key row.
    let rect = box_area(
        area,
        width,
        box_height(picker.choices.len() as u16 + 2, area.height),
    );
    f.render_widget(Clear, rect);
    let block = box_block(Color::Cyan, title);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let rows = picker.rows();
    let shown = (inner.height as usize).saturating_sub(2).max(1);
    picker.page.set(shown);
    picker.view.set(inner.width as usize);
    let left = picker.left.min(widest.saturating_sub(inner.width as usize));
    let top = picker.at.saturating_sub(shown - 1);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(top)
        .take(shown)
        .map(|(row, &i)| {
            let (name, folder) = &picker.choices[i];
            let style = if row == picker.at {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut spans = vec![Span::styled(format!(" {name} "), style)];
            if *folder {
                spans.push(Span::styled(
                    NOT_YET,
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).scroll((0, u16::try_from(left).unwrap_or(u16::MAX))),
        Rect {
            height: shown as u16,
            ..inner
        },
    );
    let keys = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(key_row(picker, rows.len())), keys);
    vscrollbar(f, rect, rows.len(), top, shown);
    hscrollbar(f, rect, widest, left, inner.width as usize);
}

/// The picker's last row: the query while it is typed, with how many of the
/// choices it keeps (`kept`), else the keys, led by the query while one holds.
fn key_row(picker: &Picker, kept: usize) -> Line<'static> {
    if picker.typing {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let mut spans = vec![Span::styled("/", bold.fg(Color::Cyan))];
        spans.extend(line_edit::with_cursor(&picker.query, picker.back, bold));
        spans.extend(box_hint(&[&format!("   {kept} match   {KEEP}"), BACK]).spans);
        return Line::from(spans);
    }
    if picker.query.is_empty() {
        return box_hint(FINDER_KEYS);
    }
    box_hint(&[&format!("/{}", picker.query), BACK, PICK, FIND])
}
