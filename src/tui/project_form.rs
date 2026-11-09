//! The projects tab's `a`: a form for the folder to add as a project, anywhere
//! on this machine, typed or picked with `ctrl-o` from the folders under the
//! scopes that are not one yet.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::line_edit;
use super::link_form::render_picker;
use super::widgets::*;

const FOLDER_HINT: &str = "ctrl-o to pick one under your scopes · or type a path anywhere";
const NOTE: &str = "canonize writes nothing in the folder: it is listed in canonize.toml, and gets a CANON.md only once you give it a convention.";

pub(crate) struct ProjectForm {
    folder: String,
    /// The cursor in `folder`, as characters after it (`line_edit::edit`).
    back: usize,
    /// The folders `ctrl-o` picks from, as `canon status` would name them;
    /// none is a project, so none is marked as one.
    choices: Vec<(String, bool)>,
    picking: Option<usize>,
    /// Why the last save was refused, shown in place of the note.
    pub(super) error: Option<String>,
}

pub(crate) enum Named {
    Pending,
    Cancelled,
    /// The folder as typed or picked.
    Done(String),
}

impl ProjectForm {
    pub(super) fn new(choices: Vec<String>) -> ProjectForm {
        ProjectForm {
            folder: String::new(),
            back: 0,
            choices: choices.into_iter().map(|c| (c, false)).collect(),
            picking: None,
            error: None,
        }
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Named {
        use KeyCode::*;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(at) = self.picking {
            let last = self.choices.len().saturating_sub(1);
            match key.code {
                Down | Char('j') => self.picking = Some((at + 1).min(last)),
                Up | Char('k') => self.picking = Some(at.saturating_sub(1)),
                Enter => {
                    if let Some((name, _)) = self.choices.get(at) {
                        self.folder = name.clone();
                        self.back = 0;
                    }
                    self.picking = None;
                }
                Esc => self.picking = None,
                _ => {}
            }
            return Named::Pending;
        }
        if ctrl && key.code == Char('o') {
            if self.choices.is_empty() {
                self.error = Some("nothing under your scopes is left to add: type a path".into());
            } else {
                let at = self.choices.iter().position(|c| c.0 == self.folder.trim());
                self.picking = Some(at.unwrap_or(0));
            }
            return Named::Pending;
        }
        match key.code {
            Esc => return Named::Cancelled,
            Enter => {
                let folder = self.folder.trim().to_string();
                if folder.is_empty() {
                    self.error = Some("folder is required".into());
                } else {
                    return Named::Done(folder);
                }
            }
            _ => {
                line_edit::edit(&mut self.folder, &mut self.back, key);
            }
        }
        Named::Pending
    }
}

/// The form over `area`, and the folder picker over it while it is open.
pub(super) fn render_project_form(f: &mut Frame, area: Rect, form: &ProjectForm) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let label = "folder";
    let mut spans = vec![
        Span::raw(label),
        Span::styled("*", Style::default().fg(Color::Red)),
        Span::raw(format!(
            "{:<1$}",
            ":",
            14usize.saturating_sub(label.chars().count() + 1)
        )),
    ];
    spans.extend(line_edit::with_cursor(&form.folder, form.back, bold));
    if form.folder.is_empty() {
        spans.push(Span::styled(FOLDER_HINT, dim));
    }
    let (note, style) = match &form.error {
        Some(e) => (e.clone(), Style::default().fg(Color::Yellow)),
        None => (NOTE.to_string(), dim),
    };
    let keys = [FORM_KEYS, &[REQUIRED]].concat();
    let lines = vec![
        Line::from(spans),
        Line::raw(""),
        Line::styled(note.clone(), style),
        Line::raw(""),
        box_hint(&keys),
    ];

    let w = box_width(area.width);
    let inner = box_inner_width(w);
    let shown = if form.folder.is_empty() {
        FOLDER_HINT
    } else {
        &form.folder
    };
    // The cursor takes a column of its own after the value.
    let body = wrapped_line_count(&format!("{:14}{shown} ", ""), inner) as u16
        + 1
        + wrapped_line_count(&note, inner) as u16
        + 1
        + wrapped_line_count(&keys.join(SEP), inner) as u16;
    let r = box_area(area, w, box_height(body, area.height));
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .block(box_block(Color::Cyan, "add project"))
            .wrap(Wrap { trim: false }),
        r,
    );
    if let Some(at) = form.picking {
        render_picker(f, area, "under your scopes", &form.choices, at);
    }
}
