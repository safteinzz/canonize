//! The setup wizard: six questions, each pre-filled with what canonize found
//! and a note saying why, so nothing is chosen for the user unseen.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::widgets::*;
use crate::config::{self, tilde};
use crate::setup::{self, Choice, Found, Lost, Pick};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Folder,
    Rules,
    Schema,
    House,
    Skills,
    Projects,
}

const STEPS: [Step; 6] = [
    Step::Folder,
    Step::Rules,
    Step::Schema,
    Step::House,
    Step::Skills,
    Step::Projects,
];

/// One row of a pick step: what it shows, and what choosing it means.
struct Option_<T> {
    label: String,
    value: T,
}

pub(crate) struct Wizard {
    step: Step,
    found: Option<Found>,
    /// A rules import pointing at a file that is gone, when nothing was found.
    lost: Option<Lost>,
    folder: String,
    rules: Vec<Option_<Pick>>,
    schema: Vec<Option_<Pick>>,
    house: Vec<Option_<Option<String>>>,
    picked: [usize; 6],
    skills: String,
    projects: String,
}

pub(crate) enum Outcome {
    Pending,
    Cancelled,
    Done(Choice),
}

impl Wizard {
    pub(super) fn new() -> Wizard {
        let found = setup::detect_or_moved();
        // A moved canon: its rules file turned up nearby, so the note can say so.
        let lost = if setup::detect().is_none() {
            setup::lost()
        } else {
            None
        };
        let folder = found
            .as_ref()
            .map_or_else(|| tilde(&config::source_dir()), |f| tilde(&f.root));
        let mut w = Wizard {
            step: Step::Folder,
            found,
            lost,
            folder,
            rules: Vec::new(),
            schema: Vec::new(),
            house: Vec::new(),
            picked: [0; 6],
            skills: "skills".to_string(),
            projects: crate::projects::guess().join(", "),
        };
        w.fill();
        w
    }

    fn root(&self) -> std::path::PathBuf {
        config::expand(self.folder.trim())
    }

    /// The found file, when it is in the folder the user has now.
    fn found_here(&self) -> Option<&Found> {
        self.found.as_ref().filter(|f| f.root == self.root())
    }

    /// List what the chosen folder offers for every later step, with what was
    /// found preselected.
    fn fill(&mut self) {
        let root = self.root();
        let found_rules = self
            .found_here()
            .and_then(|f| f.rules.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let by = self.found_here().map(|f| tilde(&f.by));

        self.rules = setup::rule_files(&root)
            .into_iter()
            .map(|n| Option_ {
                label: match (&found_rules, &by) {
                    (Some(f), Some(by)) if *f == n && self.lost.is_some() => {
                        format!("{n}   (what {by} imported before it moved)")
                    }
                    (Some(f), Some(by)) if *f == n => format!("{n}   (what {by} reads)"),
                    _ => n.clone(),
                },
                value: Pick::Existing(n),
            })
            .collect();
        self.rules.push(Option_ {
            label: "a new rules.yaml from the starter".into(),
            value: Pick::New,
        });
        self.rules.push(Option_ {
            label: "no rules file: my rules live in each agent's own file".into(),
            value: Pick::None,
        });
        self.picked[1] = found_rules
            .and_then(|f| {
                self.rules
                    .iter()
                    .position(|o| o.value == Pick::Existing(f.clone()))
            })
            .unwrap_or(0);

        self.schema = setup::schema_files(&root)
            .into_iter()
            .map(|n| Option_ {
                label: n.clone(),
                value: Pick::Existing(n),
            })
            .collect();
        let has_schema = !self.schema.is_empty();
        self.schema.push(Option_ {
            label: "a new rules.schema.json from the starter".into(),
            value: Pick::New,
        });
        self.schema.push(Option_ {
            label: "no schema".into(),
            value: Pick::None,
        });
        self.picked[2] = if has_schema { 0 } else { self.schema.len() - 2 };

        self.house = setup::house_patterns(&root)
            .into_iter()
            .map(|(p, n)| Option_ {
                label: format!("{p}   ({n} file{})", if n == 1 { "" } else { "s" }),
                value: Some(p),
            })
            .collect();
        if !self
            .house
            .iter()
            .any(|o| o.value.as_deref() == Some("house/*.md"))
        {
            self.house.push(Option_ {
                label: "house/*.md   (a folder for them, empty for now)".into(),
                value: Some("house/*.md".into()),
            });
        }
        self.house.push(Option_ {
            label: "no house files".into(),
            value: None,
        });
        self.picked[3] = 0;
    }

    /// The steps this run actually asks: with no rules file there is nothing
    /// for a schema to check, so that question goes.
    fn steps(&self) -> Vec<Step> {
        let no_rules = self
            .rules
            .get(self.picked[1])
            .is_some_and(|o| o.value == Pick::None);
        STEPS
            .iter()
            .copied()
            .filter(|s| !(no_rules && *s == Step::Schema))
            .collect()
    }

    fn index(&self) -> usize {
        self.steps()
            .iter()
            .position(|s| *s == self.step)
            .unwrap_or(0)
    }

    fn len(&self) -> usize {
        match self.step {
            Step::Rules => self.rules.len(),
            Step::Schema => self.schema.len(),
            Step::House => self.house.len(),
            _ => 0,
        }
    }

    fn choice(&self) -> Choice {
        let rules = self.rules[self.picked[1]].value.clone();
        Choice {
            root: self.root(),
            schema: if rules == Pick::None {
                Pick::None
            } else {
                self.schema[self.picked[2]].value.clone()
            },
            rules,
            house: self.house[self.picked[3]].value.clone(),
            skills: self.skills.trim().trim_end_matches('/').to_string(),
            projects: self
                .projects
                .split(',')
                .map(|p| p.trim().trim_end_matches('/').to_string())
                .filter(|p| !p.is_empty())
                .collect(),
        }
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Outcome {
        use KeyCode::*;
        let i = self.index();
        let typing = matches!(self.step, Step::Folder | Step::Skills | Step::Projects);
        match key.code {
            Esc if i == 0 => return Outcome::Cancelled,
            Esc => self.step = self.steps()[i - 1],
            Enter => {
                if self.step == Step::Folder {
                    if self.folder.trim().is_empty() {
                        return Outcome::Pending;
                    }
                    self.fill();
                }
                if self.step == Step::Skills && self.skills.trim().is_empty() {
                    return Outcome::Pending;
                }
                match self.steps().get(i + 1) {
                    Some(next) => self.step = *next,
                    None => return Outcome::Done(self.choice()),
                }
            }
            Backspace if typing => {
                let field = self.field();
                field.pop();
            }
            Char(c) if typing && !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.field().push(c)
            }
            Down | Char('j') if !typing => {
                self.picked[i] = (self.picked[i] + 1).min(self.len().saturating_sub(1))
            }
            Up | Char('k') if !typing => self.picked[i] = self.picked[i].saturating_sub(1),
            _ => {}
        }
        Outcome::Pending
    }

    fn field(&mut self) -> &mut String {
        match self.step {
            Step::Skills => &mut self.skills,
            Step::Projects => &mut self.projects,
            _ => &mut self.folder,
        }
    }
}

pub(super) fn render_wizard(f: &mut Frame, area: Rect, w: &Wizard) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let i = w.index();
    let mut lines: Vec<Line> = Vec::new();
    let mut note: Vec<String> = Vec::new();

    let (question, label) = match w.step {
        Step::Folder => (
            "Welcome. Where should your canon live? It is the one folder that holds your rules, house files and skills, and every agent reads from it.",
            "folder",
        ),
        Step::Rules => ("Which file in your canon is your rules?", ""),
        Step::Schema => ("Which schema should `canon validate` hold them to?", ""),
        Step::House => ("Which are your house files?", ""),
        Step::Skills => ("Where in your canon should your skills live?", "skills"),
        Step::Projects => (
            "Where are your projects? canonize shows which house files each one imports.",
            "projects",
        ),
    };
    lines.push(Line::raw(question));
    lines.push(Line::raw(""));

    match w.step {
        Step::Folder | Step::Skills | Step::Projects => {
            let value = match w.step {
                Step::Folder => &w.folder,
                Step::Skills => &w.skills,
                _ => &w.projects,
            };
            // Both typed fields refuse a blank, so both carry the red star.
            lines.push(Line::from(vec![
                Span::raw(label.to_string()),
                Span::styled("*", Style::default().fg(Color::Red)),
                Span::raw(format!("{:<1$}", ":", 14 - label.chars().count())),
                Span::styled(value.clone(), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw("█"),
            ]));
            if w.step == Step::Folder {
                match (&w.found, &w.lost) {
                    (Some(fd), Some(l)) if fd.root == w.root() => note.push(format!(
                        "{} imports {}, which is gone. A file by that name is here now, so your canon most likely moved here.",
                        tilde(&l.by),
                        tilde(&l.path)
                    )),
                    (Some(fd), None) if fd.root == w.root() => note.push(format!(
                        "Found because {} reads {} from here.",
                        tilde(&fd.by),
                        tilde(&fd.rules)
                    )),
                    (None, Some(l)) => note.push(format!(
                        "{} imports {}, which is gone, and no folder next to it has a file by that name. Type where your canon is now.",
                        tilde(&l.by),
                        tilde(&l.path)
                    )),
                    (Some(_), _) => note.push("A folder of your choosing.".into()),
                    (None, None) => note.push(
                        "None of your agents reads a rules file yet, so your canon starts here, fresh."
                            .into(),
                    ),
                }
                if !w.root().exists() {
                    note.push("It does not exist yet; setup creates it.".into());
                }
            } else if w.step == Step::Projects {
                note.push("Folders, comma separated. canonize looks up to three levels down for a CLAUDE.md or AGENTS.md; leave it empty to skip projects.".into());
                if !crate::projects::guess().is_empty() {
                    note.push("Pre-filled with the usual project folders that exist here.".into());
                }
            } else {
                let dir = w.root().join(w.skills.trim());
                note.push(format!("A folder inside your canon: {}/.", tilde(&dir)));
                if !dir.exists() {
                    note.push(
                        "It starts empty; adopt fills it with the skills your agents already have."
                            .into(),
                    );
                }
            }
        }
        Step::Rules | Step::Schema | Step::House => {
            let labels: Vec<&String> = match w.step {
                Step::Rules => w.rules.iter().map(|o| &o.label).collect(),
                Step::Schema => w.schema.iter().map(|o| &o.label).collect(),
                _ => w.house.iter().map(|o| &o.label).collect(),
            };
            for (n, l) in labels.iter().enumerate() {
                let style = if n == w.picked[i] {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(Line::styled(format!(" {l} "), style));
            }
            note.push(match w.step {
                Step::Rules => "The file every agent will follow. Listed: the YAML and Markdown files in your folder. `canon validate` holds a YAML one to the schema; any other format only has to be there.".into(),
                Step::Schema => "`canon validate` validates your rules against it. Listed: the *.schema.json files in your folder.".into(),
                _ => "Shared rules your projects import one by one. Listed: the patterns that match files in your folder.".into(),
            });
        }
    }

    lines.push(Line::raw(""));
    for n in &note {
        lines.push(Line::styled(n.clone(), dim));
    }
    lines.push(Line::raw(""));
    let hint = match w.step {
        Step::Folder => "type to change · enter next · esc cancel · * required",
        Step::Skills => "type to change · enter next · esc back · * required",
        Step::Projects => "type to change · enter review · esc back",
        _ => "j/k ↑↓ choose · enter next · esc back",
    };
    lines.push(box_hint(hint));

    let width = box_width(area.width);
    let inner = box_inner_width(width);
    let rows: u16 = lines
        .iter()
        .map(|l| {
            let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            wrapped_line_count(&text, inner) as u16
        })
        .sum();
    let rect = box_area(area, width, box_height(rows, area.height));
    f.render_widget(Clear, rect);
    let para = Paragraph::new(lines)
        .block(box_block(
            Color::Cyan,
            &format!("set up {}/{}", i + 1, w.steps().len()),
        ))
        .wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}
