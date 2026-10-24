//! The workspaces tab's `c`, and `a` in a workspace's card: a form for a
//! folder to link to a project, or for one more project the workspace is for.
//! The project is typed, or picked with `ctrl-o` from the projects canonize
//! found.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::line_edit;
use super::widgets::*;

/// What each field says while it is empty.
const FOLDER_HINT: &str = "~/issues/app, a folder outside the project";
const PROJECT_HINT: &str = "ctrl-o to pick · or type a project or its path";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Workspace,
    Project,
}

pub(crate) struct LinkForm {
    /// The workspace getting one more project, by its key, when opened with
    /// `a` in its card.
    pub(super) fixed: Option<String>,
    focus: usize,
    workspace: String,
    project: String,
    /// Each field's cursor, as characters after it (`line_edit::edit`), in
    /// the order `workspace`, `project`.
    backs: [usize; 2],
    /// The projects `ctrl-o` picks from, as `canon status` names them.
    choices: Vec<String>,
    /// The picked choice while the picker is open.
    picking: Option<usize>,
    /// Why the last save was refused, shown in place of the note.
    pub(super) error: Option<String>,
}

pub(crate) enum Linked {
    Pending,
    Cancelled,
    /// The folder as typed (or the key, for `a`) and the project as typed.
    Done(String, String),
}

impl LinkForm {
    pub(super) fn new(choices: Vec<String>) -> LinkForm {
        LinkForm {
            fixed: None,
            focus: 0,
            workspace: String::new(),
            project: String::new(),
            backs: [0; 2],
            choices,
            picking: None,
            error: None,
        }
    }

    /// The form for one more project of the workspace `key`.
    pub(super) fn adding(key: &str, choices: Vec<String>) -> LinkForm {
        let mut f = LinkForm::new(choices);
        f.fixed = Some(key.to_string());
        f
    }

    fn rows(&self) -> Vec<Row> {
        if self.fixed.is_some() {
            vec![Row::Project]
        } else {
            vec![Row::Workspace, Row::Project]
        }
    }

    fn row(&self) -> Row {
        self.rows()[self.focus]
    }

    fn slot(row: Row) -> usize {
        match row {
            Row::Workspace => 0,
            Row::Project => 1,
        }
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Linked {
        use KeyCode::*;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(at) = self.picking {
            let last = self.choices.len().saturating_sub(1);
            match key.code {
                Down | Char('j') => self.picking = Some((at + 1).min(last)),
                Up | Char('k') => self.picking = Some(at.saturating_sub(1)),
                Enter => {
                    if let Some(name) = self.choices.get(at) {
                        self.project = name.clone();
                        self.backs[1] = 0;
                    }
                    self.picking = None;
                }
                Esc => self.picking = None,
                _ => {}
            }
            return Linked::Pending;
        }
        if ctrl && key.code == Char('o') && self.row() == Row::Project {
            if self.choices.is_empty() {
                self.error = Some("canonize found no project to pick: type its path".into());
            } else {
                let at = self.choices.iter().position(|c| *c == self.project.trim());
                self.picking = Some(at.unwrap_or(0));
            }
            return Linked::Pending;
        }
        let n = self.rows().len();
        // Plain j/k are typed text, so fields move with Tab, ↑/↓ or Ctrl-j/k.
        let next = matches!(key.code, Tab | Down) || (ctrl && key.code == Char('j'));
        let prev = matches!(key.code, BackTab | Up) || (ctrl && key.code == Char('k'));
        if next {
            self.focus = (self.focus + 1) % n;
            return Linked::Pending;
        }
        if prev {
            self.focus = (self.focus + n - 1) % n;
            return Linked::Pending;
        }
        match key.code {
            Esc => return Linked::Cancelled,
            Enter if self.focus + 1 < n => self.focus += 1,
            Enter => {
                let workspace = self
                    .fixed
                    .clone()
                    .unwrap_or_else(|| self.workspace.trim().to_string());
                let project = self.project.trim().to_string();
                if workspace.is_empty() {
                    self.error = Some("folder is required".into());
                } else if project.is_empty() {
                    self.error = Some("project is required".into());
                } else {
                    return Linked::Done(workspace, project);
                }
            }
            _ => {
                let row = self.row();
                let back = &mut self.backs[Self::slot(row)];
                let text = match row {
                    Row::Workspace => &mut self.workspace,
                    Row::Project => &mut self.project,
                };
                line_edit::edit(text, back, key);
            }
        }
        Linked::Pending
    }
}

/// The form over `area`, and the project picker over it while it is open.
pub(super) fn render_link_form(f: &mut Frame, area: Rect, form: &LinkForm) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let rows = form.rows();
    let focused = |row: Row| rows.get(form.focus) == Some(&row);
    let label_style = |row: Row| {
        if focused(row) { Style::default() } else { dim }
    };
    let line = |row: Row, label: &str, value: &str, placeholder: &str| {
        let back = form.backs[LinkForm::slot(row)];
        let mut spans = vec![
            Span::styled(label.to_string(), label_style(row)),
            Span::styled("*", Style::default().fg(Color::Red)),
            Span::styled(
                format!(
                    "{:<1$}",
                    ":",
                    14usize.saturating_sub(label.chars().count() + 1)
                ),
                label_style(row),
            ),
        ];
        let bold = Style::default().add_modifier(Modifier::BOLD);
        if focused(row) {
            spans.extend(line_edit::with_cursor(value, back, bold));
        } else if !value.is_empty() {
            spans.push(Span::styled(value.to_string(), bold));
        }
        if value.is_empty() {
            spans.push(Span::styled(placeholder.to_string(), dim));
        }
        Line::from(spans)
    };
    let mut lines = Vec::new();
    if form.fixed.is_none() {
        lines.push(line(Row::Workspace, "folder", &form.workspace, FOLDER_HINT));
    }
    lines.push(line(Row::Project, "project", &form.project, PROJECT_HINT));
    lines.push(Line::raw(""));
    let (note, style) = match (&form.error, &form.fixed) {
        (Some(e), _) => (e.clone(), Style::default().fg(Color::Yellow)),
        (None, Some(_)) => (
            "The files canonize writes at the folder's top then list this project as well.".to_string(),
            dim,
        ),
        (None, None) => (
            "canonize writes a CLAUDE.md and an AGENTS.md at the folder's top, so an agent started in any folder under it reads the project's files.".to_string(),
            dim,
        ),
    };
    lines.push(Line::styled(note.clone(), style));
    lines.push(Line::raw(""));
    let keys = [FORM_KEYS, &[REQUIRED]].concat();
    lines.push(box_hint(&keys));

    let w = box_width(area.width);
    let inner = box_inner_width(w);
    // Each field as it wraps: its label column, then its value or placeholder.
    let field = |value: &str, placeholder: &str| {
        let shown = if value.is_empty() { placeholder } else { value };
        // The cursor takes a column of its own after the value.
        wrapped_line_count(&format!("{:14}{shown} ", ""), inner) as u16
    };
    let mut fields = field(&form.project, PROJECT_HINT);
    if form.fixed.is_none() {
        fields += field(&form.workspace, FOLDER_HINT);
    }
    let body = fields
        + 1
        + wrapped_line_count(&note, inner) as u16
        + 1
        + wrapped_line_count(&keys.join(SEP), inner) as u16;
    let r = box_area(area, w, box_height(body, area.height));
    let title = match &form.fixed {
        Some(key) => format!("add a project to {key}"),
        None => "create workspace".to_string(),
    };
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .block(box_block(Color::Cyan, &title))
            .wrap(Wrap { trim: false }),
        r,
    );
    if let Some(at) = form.picking {
        render_picker(f, area, &form.choices, at);
    }
}

/// The projects to pick from, scrolled so `at` is on screen.
fn render_picker(f: &mut Frame, area: Rect, choices: &[String], at: usize) {
    let width = box_width(area.width);
    // The choices, then a blank and the key row.
    let rect = box_area(
        area,
        width,
        box_height(choices.len() as u16 + 2, area.height),
    );
    f.render_widget(Clear, rect);
    let block = box_block(Color::Cyan, "projects");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let shown = (inner.height as usize).saturating_sub(2).max(1);
    let top = at.saturating_sub(shown - 1);
    let lines: Vec<Line> = choices
        .iter()
        .enumerate()
        .skip(top)
        .take(shown)
        .map(|(i, name)| {
            let style = if i == at {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::styled(format!(" {name} "), style)
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines),
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
    f.render_widget(Paragraph::new(box_hint(PICKER_KEYS)), keys);
    vscrollbar(f, rect, choices.len(), top, shown);
}
