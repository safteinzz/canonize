//! The MCP tab's `n` and `e`: a form for a new server, or for the one under
//! the cursor, saved into the canon's mcp.toml. Its token is typed here and
//! kept outside the canon, never shown again.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::line_edit;
use super::widgets::*;
use crate::mcp::{Bearer, Server, Spec};

const HINT: &str = "↵ next, saves on the last · esc cancel · * required";

/// A row of the form. Editing a server leaves its name out, since the row it
/// was opened from already says which server it is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Kind,
    Name,
    /// The URL of a remote server, the command of a local one.
    First,
    /// The token of a remote server, the arguments of a local one.
    Second,
}

pub(crate) struct ServerForm {
    remote: bool,
    focus: usize,
    name: String,
    url: String,
    token: String,
    command: String,
    args: String,
    /// Each text field's cursor, as characters after it (`line_edit::edit`),
    /// in the order `name`, `url`, `token`, `command`, `args`.
    backs: [usize; 5],
    /// The server being edited, when it is not a new one.
    pub(super) old: Option<Server>,
    /// Whether the server being edited has a kept token already.
    has_token: bool,
    /// Why the last save was refused, shown in place of the note.
    pub(super) error: Option<String>,
}

/// What the form describes once saved: the name, the server as it should be
/// (a token it reads from a variable kept), and a newly typed token, if any.
pub(crate) struct Saved {
    pub name: String,
    pub spec: Spec,
    pub token: Option<String>,
}

pub(crate) enum Filled {
    Pending,
    Cancelled,
    Done(Saved),
}

impl ServerForm {
    pub(super) fn new() -> ServerForm {
        ServerForm {
            remote: true,
            focus: 1,
            name: String::new(),
            url: String::new(),
            token: String::new(),
            command: String::new(),
            args: String::new(),
            backs: [0; 5],
            old: None,
            has_token: false,
            error: None,
        }
    }

    /// The form over `server`, its token left blank: typing one replaces it.
    pub(super) fn edit(server: &Server) -> ServerForm {
        let mut f = ServerForm::new();
        f.name = server.name.clone();
        match &server.spec {
            Spec::Http { url, .. } => f.url = url.clone(),
            Spec::Stdio { command, args, .. } => {
                f.remote = false;
                f.command = command.clone();
                f.args = join_args(args);
            }
        }
        f.has_token = server.kept().is_some_and(|p| p.is_file());
        f.old = Some(server.clone());
        f
    }

    fn rows(&self) -> Vec<Row> {
        if self.old.is_some() {
            vec![Row::Kind, Row::First, Row::Second]
        } else {
            vec![Row::Kind, Row::Name, Row::First, Row::Second]
        }
    }

    fn row(&self) -> Row {
        self.rows()[self.focus]
    }

    /// Which of the five text fields `row` is, for its cursor.
    fn slot(&self, row: Row) -> Option<usize> {
        match (row, self.remote) {
            (Row::Name, _) => Some(0),
            (Row::First, true) => Some(1),
            (Row::Second, true) => Some(2),
            (Row::First, false) => Some(3),
            (Row::Second, false) => Some(4),
            (Row::Kind, _) => None,
        }
    }

    fn field(&mut self) -> Option<(&mut String, &mut usize)> {
        let i = self.slot(self.row())?;
        let back = &mut self.backs[i];
        Some(match i {
            0 => (&mut self.name, back),
            1 => (&mut self.url, back),
            2 => (&mut self.token, back),
            3 => (&mut self.command, back),
            _ => (&mut self.args, back),
        })
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Filled {
        use KeyCode::*;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.rows().len();
        // The kind types nothing, so its horizontal keys point at a button and
        // stop at the edge; you leave it with Tab, Enter or the vertical keys.
        if self.row() == Row::Kind && !ctrl {
            match key.code {
                Left | Char('h') => {
                    self.remote = true;
                    return Filled::Pending;
                }
                Right | Char('l') => {
                    self.remote = false;
                    return Filled::Pending;
                }
                _ => {}
            }
        }
        // Plain j/k are typed text and ←/→ move the cursor in a field, so
        // fields move with Tab, ↑/↓ or Ctrl-j/k.
        let next = matches!(key.code, Tab | Down) || (ctrl && key.code == Char('j'));
        let prev = matches!(key.code, BackTab | Up) || (ctrl && key.code == Char('k'));
        if next {
            self.focus = (self.focus + 1) % n;
            return Filled::Pending;
        }
        if prev {
            self.focus = (self.focus + n - 1) % n;
            return Filled::Pending;
        }
        match key.code {
            Esc => return Filled::Cancelled,
            // Enter walks down the form and saves from its last field.
            Enter if self.focus + 1 < n => self.focus += 1,
            Enter => match self.saved() {
                Ok(saved) => return Filled::Done(saved),
                Err(why) => self.error = Some(why),
            },
            _ => {
                if let Some((text, back)) = self.field() {
                    line_edit::edit(text, back, key);
                }
            }
        }
        Filled::Pending
    }

    /// The server the form describes, or which starred field is still blank.
    /// What the form does not show of a server being edited (its headers, its
    /// env, a token it reads from a variable) stays as it was.
    fn saved(&self) -> Result<Saved, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("name is required".into());
        }
        let old = self.old.as_ref().map(|s| &s.spec);
        let spec = if self.remote {
            let url = self.url.trim();
            if url.is_empty() {
                return Err("url is required".into());
            }
            let (bearer, headers) = match old {
                Some(Spec::Http {
                    bearer, headers, ..
                }) => (bearer.clone(), headers.clone()),
                _ => (None, Default::default()),
            };
            Spec::Http {
                url: url.to_string(),
                bearer,
                headers,
            }
        } else {
            let command = self.command.trim();
            if command.is_empty() {
                return Err("command is required".into());
            }
            let env = match old {
                Some(Spec::Stdio { env, .. }) => env.clone(),
                _ => Default::default(),
            };
            Spec::Stdio {
                command: command.to_string(),
                args: split_args(&self.args),
                env,
            }
        };
        let token = self.token.trim();
        Ok(Saved {
            name: name.to_string(),
            spec,
            token: (self.remote && !token.is_empty()).then(|| token.to_string()),
        })
    }

    /// What the token field says while it is empty.
    fn token_placeholder(&self) -> String {
        match self.old.as_ref().map(|s| &s.spec) {
            _ if self.has_token => "blank keeps the current token".into(),
            Some(Spec::Http {
                bearer: Some(Bearer::Env(v)),
                ..
            }) => format!("blank keeps reading ${v}"),
            _ => "the API key, or blank for browser sign-in or none".into(),
        }
    }
}

/// `args` as one line, an argument holding a space or a quote written in
/// double quotes, so `split_args` gives the same list back.
fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() || a.chars().any(|c| c.is_whitespace() || c == '"') {
                format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The arguments on `line`: split on spaces, except inside double quotes,
/// where a backslash keeps the next character as it is.
fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(word);
    }
    out
}

/// The form over `area`; `file` is where the server is written.
pub(super) fn render_server_form(f: &mut Frame, area: Rect, form: &ServerForm, file: &str) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let rows = form.rows();
    let focused = |row: Row| rows.get(form.focus) == Some(&row);
    let label_style = |row: Row| {
        if focused(row) { Style::default() } else { dim }
    };
    let line = |row: Row, label: &str, required: bool, value: &str, placeholder: &str| {
        let back = form.slot(row).map_or(0, |i| form.backs[i]);
        let mut spans = vec![Span::styled(label.to_string(), label_style(row))];
        if required {
            spans.push(Span::styled("*", Style::default().fg(Color::Red)));
        }
        let used = label.chars().count() + usize::from(required);
        spans.push(Span::styled(
            format!("{:<1$}", ":", 14usize.saturating_sub(used)),
            label_style(row),
        ));
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
    let button = |text: &str, picked: bool| {
        Span::styled(
            format!(" {text} "),
            if picked {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                dim
            },
        )
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:<14}", "kind:"), label_style(Row::Kind)),
        button("remote", form.remote),
        Span::raw("  "),
        button("local", !form.remote),
    ])];
    if form.old.is_none() {
        let example = if form.remote { "docs" } else { "files" };
        lines.push(line(
            Row::Name,
            "name",
            true,
            &form.name,
            &format!("{example}, what every agent calls it"),
        ));
    }
    if form.remote {
        lines.push(line(
            Row::First,
            "url",
            true,
            &form.url,
            "https://example.com/mcp",
        ));
        // Never shown back, not even to the one typing it.
        let masked = "•".repeat(form.token.chars().count());
        lines.push(line(
            Row::Second,
            "token",
            false,
            &masked,
            &form.token_placeholder(),
        ));
    } else {
        lines.push(line(Row::First, "command", true, &form.command, "npx"));
        lines.push(line(
            Row::Second,
            "args",
            false,
            &form.args,
            "-y @modelcontextprotocol/server-filesystem . (\"quote one with spaces\")",
        ));
    }
    lines.push(Line::raw(""));
    // A refused save takes the note's row, so the box keeps its size.
    let (note, style) = match &form.error {
        Some(e) => (e.clone(), Style::default().fg(Color::Yellow)),
        None => (
            format!("Goes into {file}. A token is kept outside your canon, readable by you alone."),
            dim,
        ),
    };
    lines.push(Line::styled(note.clone(), style));
    lines.push(Line::raw(""));
    lines.push(box_hint(HINT));

    let w = box_width(area.width);
    let inner = box_inner_width(w);
    let body = rows.len() as u16
        + 1
        + wrapped_line_count(&note, inner) as u16
        + 1
        + wrapped_line_count(HINT, inner) as u16;
    let r = box_area(area, w, box_height(body, area.height));
    let title = match &form.old {
        Some(s) => format!("edit {}", s.name),
        None => "new MCP server".to_string(),
    };
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .block(box_block(Color::Cyan, &title))
            .wrap(Wrap { trim: false }),
        r,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_come_back_from_the_form_as_they_went_in() {
        let args: Vec<String> = ["--root", "/My Drive", "say \"hi\"", "", r"C:\tmp", "-y"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            split_args(&join_args(&args)),
            args,
            "saving a server unchanged must not split or alter its arguments"
        );
    }
}
