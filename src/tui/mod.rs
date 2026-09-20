//! The dashboard: bare `canon`. One table, a row per rule file and skill and a
//! column per agent, with the selected cell explained beside it. Everything the
//! plain commands do is a key here, behind the house boxes.

mod alert;
mod confirm;
mod render;
mod scope;
mod typed;
mod widgets;
mod wizard;

use crate::check::{self, Report};
use crate::config::{self, Config, tilde};
use crate::plan::{Change, Plan, State};
use crate::projects::{self, Projects};
use crate::setup;
use alert::Note;
use anyhow::Result;
use confirm::{Action, Answer, Confirm};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use scope::{Picked, Scope};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use typed::{Typed, Typing};
use wizard::{Outcome, Wizard};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// One line of an agent's card: a plan row, or the summary of its skills.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CardLine {
    Row(usize),
    Skills,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Agents,
    Skills,
    Projects,
}

/// How long a status message stays on screen before the hints return.
const STATUS_TTL: Duration = Duration::from_secs(3);

/// How many commands a confirm box lists before it says how many more.
const LIST_MAX: usize = 12;

pub(super) struct App {
    /// Why the config did not load, shown in place of the table.
    pub(super) load_error: Option<String>,
    pub(super) cfg: Option<Config>,
    pub(super) plan: Option<Plan>,
    pub(super) projects: Option<Projects>,
    pub(super) view: View,
    /// The selected line of the agent's card in the agents tab.
    pub(super) aline: usize,
    /// Whether the agents tab's keys go to the card (after Enter) or the list.
    pub(super) in_card: bool,
    /// The selected skill in the skills tab, an index into `Plan::skill_rows`.
    pub(super) srow: usize,
    /// The selected project and house file in the projects view.
    pub(super) prow: usize,
    pub(super) pcol: usize,
    pub(super) report: Option<Report>,
    pub(super) col: usize,
    pub(super) confirm: Option<Confirm>,
    /// A delete waiting for its name to be typed.
    pub(super) typed: Option<Typed>,
    /// The add or remove scope being chosen in the projects tab.
    pub(super) scope: Option<Scope>,
    /// The setup questions, while they are being answered.
    pub(super) wizard: Option<Wizard>,
    /// A box that is only read. It owns every key until closed.
    pub(super) note: Option<Note>,
    pub(super) status: String,
    pub(super) status_failed: bool,
    pub(super) status_at: Option<Instant>,
    should_quit: bool,
}

/// A file to hand to `$EDITOR` with the terminal suspended.
struct Edit(PathBuf);

impl App {
    fn new() -> App {
        let mut app = App {
            load_error: None,
            cfg: None,
            plan: None,
            projects: None,
            view: View::Agents,
            aline: 0,
            in_card: false,
            srow: 0,
            prow: 0,
            pcol: 0,
            report: None,
            col: 0,
            confirm: None,
            typed: None,
            scope: None,
            wizard: None,
            note: None,
            status: String::new(),
            status_failed: false,
            status_at: None,
            should_quit: false,
        };
        app.reload();
        if app.load_error.is_some() {
            app.propose_setup();
        }
        app
    }

    /// Read the config and the disk again. Called after every change, so the
    /// table never shows a state it has not just looked at.
    fn reload(&mut self) {
        // The skill under the cursor, so a change that reorders the rows (an
        // adopt, say) leaves the cursor on the same skill.
        let skill = self.plan.as_ref().and_then(|p| {
            let r = *p.skill_rows().get(self.srow)?;
            Some(p.rows[r].label())
        });
        match config::load() {
            Ok(cfg) => {
                self.plan = Some(Plan::build(&cfg));
                self.projects = Some(Projects::build(&cfg));
                self.report = Some(check::run(&cfg.source));
                self.cfg = Some(cfg);
                self.load_error = None;
            }
            Err(e) => {
                self.cfg = None;
                self.plan = None;
                self.projects = None;
                self.report = None;
                self.load_error = Some(format!("{e:#}"));
            }
        }
        let (_, cols) = self.size();
        self.col = self.col.min(cols.saturating_sub(1));
        self.aline = self.aline.min(self.card_lines().len().saturating_sub(1));
        let skills = self.plan.as_ref().map_or(0, |p| p.skill_rows().len());
        self.srow = self.srow.min(skills.saturating_sub(1));
        if let (Some(name), Some(p)) = (skill, &self.plan)
            && let Some(n) = p
                .skill_rows()
                .iter()
                .position(|r| p.rows[*r].label() == name)
        {
            self.srow = n;
        }
        if let Some(p) = &self.projects {
            self.prow = self.prow.min(p.list.len().saturating_sub(1));
            self.pcol = self.pcol.min(p.house.len().saturating_sub(1));
        }
    }

    /// Rows and columns (agents) on screen.
    pub(super) fn size(&self) -> (usize, usize) {
        match (&self.plan, &self.cfg) {
            (Some(p), Some(c)) => (p.rows.len(), c.agents.len()),
            _ => (0, 0),
        }
    }

    pub(super) fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_at = Some(Instant::now());
        self.status_failed = false;
    }

    pub(super) fn set_failed(&mut self, msg: impl Into<String>) {
        self.set_status(msg);
        self.status_failed = true;
    }

    pub(super) fn live_status(&self) -> Option<&str> {
        self.status_at
            .filter(|t| t.elapsed() < STATUS_TTL)
            .map(|_| self.status.as_str())
    }

    fn on_key(&mut self, key: KeyEvent) -> Option<Edit> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return None;
        }
        if let Some(w) = &mut self.wizard {
            match w.key(key) {
                Outcome::Pending => {}
                Outcome::Cancelled => {
                    self.wizard = None;
                    self.set_status("set up cancelled: s starts it again");
                }
                Outcome::Done(choice) => {
                    self.wizard = None;
                    let message = choice.summary().join("\n");
                    self.confirm = Some(Confirm::offer("set up", message, Action::Setup(choice)));
                }
            }
            return None;
        }
        if let Some(n) = &mut self.note {
            if n.key(key) {
                self.note = None;
            }
            return None;
        }
        if let Some(c) = &mut self.confirm {
            match c.key(key) {
                Answer::Pending => {}
                Answer::No => {
                    self.confirm = None;
                    self.set_status("cancelled");
                }
                Answer::Yes => {
                    let c = self.confirm.take()?;
                    self.run_action(&c.title, c.action);
                }
            }
            return None;
        }
        if let Some(t) = &mut self.typed {
            match t.key(key) {
                Typing::Pending => {}
                Typing::Cancelled => {
                    self.typed = None;
                    self.set_status("cancelled");
                }
                Typing::Confirmed => {
                    let t = self.typed.take()?;
                    self.run_changes(&t.title, t.changes);
                }
            }
            return None;
        }
        if let Some(sc) = &mut self.scope {
            let remove = sc.remove;
            let verb = sc.verb;
            match sc.key(key) {
                Picked::Pending => {}
                Picked::Cancelled => self.scope = None,
                Picked::Chosen(label, changes) => {
                    self.scope = None;
                    self.confirm_scope(remove, verb, &label, changes);
                }
            }
            return None;
        }

        use KeyCode::*;
        if key.code == Tab || key.code == BackTab {
            self.in_card = false;
            self.view = match self.view {
                View::Agents => View::Skills,
                View::Skills => View::Projects,
                View::Projects => View::Agents,
            };
            return None;
        }
        if self.view == View::Projects {
            let (rows, cols) = self
                .projects
                .as_ref()
                .map_or((0, 0), |p| (p.list.len(), p.house.len()));
            match key.code {
                Down | Char('j') => {
                    self.prow = (self.prow + 1).min(rows.saturating_sub(1));
                    return None;
                }
                Up | Char('k') => {
                    self.prow = self.prow.saturating_sub(1);
                    return None;
                }
                Right | Char('l') => {
                    self.pcol = (self.pcol + 1).min(cols.saturating_sub(1));
                    return None;
                }
                Left | Char('h') => {
                    self.pcol = self.pcol.saturating_sub(1);
                    return None;
                }
                Char('g') | Home => {
                    self.prow = 0;
                    return None;
                }
                Char('G') | End => {
                    self.prow = rows.saturating_sub(1);
                    return None;
                }
                Char('f') => {
                    self.project_cell(true);
                    return None;
                }
                // A grid of imports reads like checkboxes, so Enter toggles.
                Enter | Char(' ') => {
                    let imported = self
                        .projects
                        .as_ref()
                        .and_then(|p| p.list.get(self.prow))
                        .and_then(|x| x.cells.get(self.pcol))
                        .is_some_and(|c| c.state == State::Linked);
                    self.project_cell(!imported);
                    return None;
                }
                Char('a') => {
                    self.open_scope(false);
                    return None;
                }
                Char('d') => {
                    self.open_scope(true);
                    return None;
                }
                Char('D') => {
                    let changes: Vec<Change> = self
                        .projects
                        .iter()
                        .flat_map(|p| p.list.iter())
                        .flat_map(|x| x.cells.iter())
                        .filter_map(|c| c.undo.clone())
                        .collect();
                    self.confirm_scope(
                        true,
                        "Delete",
                        "every house file from every project",
                        changes,
                    );
                    return None;
                }
                Char('o') => {
                    return self
                        .projects
                        .as_ref()
                        .and_then(|p| p.list.get(self.prow))
                        .map(|x| Edit(x.host.clone()));
                }
                _ => {}
            }
        }
        if self.view == View::Skills {
            match key.code {
                Enter | Char(' ') => {
                    self.skill_toggle();
                    return None;
                }
                Char('a') => {
                    self.open_skill_scope(false);
                    return None;
                }
                Char('d') => {
                    self.open_skill_scope(true);
                    return None;
                }
                Char('f') => return None,
                _ => {}
            }
        }
        // Keys only the agents tab has must not reach it from the projects tab.
        if self.view == View::Projects && matches!(key.code, Char('f' | 'd' | 'D') | Enter) {
            return None;
        }
        let (_, agents) = self.size();
        let lines = self.card_lines().len();
        let skills = self.plan.as_ref().map_or(0, |p| p.skill_rows().len());
        // Agents: j/k walks the list, or the card once Enter has opened it.
        // Skills: j/k picks the skill, h/l the agent.
        if self.view == View::Agents {
            if !self.in_card && key.code == Enter {
                self.in_card = true;
                return None;
            }
            if self.in_card && key.code == Esc {
                self.in_card = false;
                return None;
            }
            // f and d act on a card line, so the card has to be open.
            if !self.in_card && matches!(key.code, Char('f' | 'd')) {
                self.set_status("↵ opens the agent's card, where f and d act on a line");
                return None;
            }
        }
        let mut none = 0usize;
        let (vert, vert_max, horiz, horiz_max) = match self.view {
            View::Skills => (&mut self.srow, skills, &mut self.col, agents),
            _ if self.in_card => (&mut self.aline, lines, &mut none, 0),
            _ => (&mut self.col, agents, &mut none, 0),
        };
        match key.code {
            Down | Char('j') => {
                *vert = (*vert + 1).min(vert_max.saturating_sub(1));
                return None;
            }
            Up | Char('k') => {
                *vert = vert.saturating_sub(1);
                return None;
            }
            Right | Char('l') => {
                *horiz = (*horiz + 1).min(horiz_max.saturating_sub(1));
                return None;
            }
            Left | Char('h') => {
                *horiz = horiz.saturating_sub(1);
                return None;
            }
            Char('g') | Home => {
                *vert = 0;
                return None;
            }
            Char('G') | End => {
                *vert = vert_max.saturating_sub(1);
                return None;
            }
            _ => {}
        }
        match key.code {
            Char('q') | Esc => self.should_quit = true,
            Char('?') => self.note = Some(Note::reader("help", render::HELP.to_string())),
            Char('r') => {
                self.reload();
                self.set_status("reloaded");
            }
            Char('f') | Enter => self.agent_cell(true),
            Char('F') => self.offer_fix_all(),
            Char('d') => self.agent_cell(false),
            Char('D') if self.view == View::Skills => self.remove_skills_all(),
            Char('D') => self.remove_agents_all(),
            Char('v') => self.show_check(),
            Char('s') if self.load_error.is_some() => self.propose_setup(),
            Char('e') => {
                return Some(Edit(match &self.cfg {
                    Some(c) => c.path.clone(),
                    None => config::source_dir().join(config::CONFIG_FILE),
                }));
            }
            _ => {}
        }
        None
    }

    /// The commands a list of changes amounts to, cut off with a count.
    fn runs(changes: &[Change]) -> Vec<String> {
        let mut out: Vec<String> = changes.iter().take(LIST_MAX).map(Change::command).collect();
        if changes.len() > LIST_MAX {
            out.push(format!("# and {} more", changes.len() - LIST_MAX));
        }
        out
    }

    /// Everything broken or missing in the current tab, behind one offer.
    fn offer_fix_all(&mut self) {
        let Some(plan) = &self.plan else { return };
        let not_adopt = |c: &Change| !matches!(c, Change::Adopt { .. });
        let rows_changes = |rows: Vec<usize>| -> Vec<Change> {
            let mut out: Vec<Change> = Vec::new();
            for r in rows {
                for c in plan.cells[r].iter().filter_map(|c| c.change.clone()) {
                    if not_adopt(&c) && !out.contains(&c) {
                        out.push(c);
                    }
                }
            }
            out
        };
        // F fixes what the tab shows, nothing beyond it.
        let (changes, what) = match self.view {
            View::Agents => (rows_changes(plan.setup_rows()), "every agent's setup"),
            View::Skills => {
                let mut c = rows_changes(plan.skill_rows());
                c.extend(plan.stale.iter().cloned());
                (c, "every missing or broken skill link")
            }
            View::Projects => (
                self.projects
                    .as_ref()
                    .map(|p| p.fixes())
                    .unwrap_or_default(),
                "every project's broken imports and CANON.md wiring",
            ),
        };
        if changes.is_empty() {
            self.set_status(format!("nothing to fix: {what} is up to date"));
            return;
        }
        let message = format!(
            "Fix {what}? {} change{}.",
            changes.len(),
            if changes.len() == 1 { "" } else { "s" },
        );
        let runs = Self::runs(&changes);
        self.confirm =
            Some(Confirm::offer("fix all", message, Action::Changes(changes)).runs(runs));
    }

    /// Every agent's setup canonize made (rules, CANON.md loader), behind one red gate.
    fn remove_agents_all(&mut self) {
        let Some(plan) = &self.plan else { return };
        let mut changes: Vec<Change> = Vec::new();
        for r in plan.setup_rows() {
            for u in plan.cells[r].iter().filter_map(|c| c.undo.clone()) {
                if !changes.contains(&u) {
                    changes.push(u);
                }
            }
        }
        if changes.is_empty() {
            self.set_status("canonize has wired no agent's setup");
            return;
        }
        let message = format!(
            "Delete every agent's setup canonize made ({} change{})? Skills stay, and so does your canon.",
            changes.len(),
            if changes.len() == 1 { "" } else { "s" },
        );
        let runs = Self::runs(&changes);
        self.confirm =
            Some(Confirm::gate("delete all", message, Action::Changes(changes)).runs(runs));
    }

    /// The add or remove choices for the selected cell, nearest first: this
    /// cell, every house file in this project, then its house file in every
    /// project, the widest and least likely.
    fn open_scope(&mut self, remove: bool) {
        let (Some(p), Some(cfg)) = (&self.projects, &self.cfg) else {
            return;
        };
        let (Some(x), Some(h)) = (p.list.get(self.prow), p.house.get(self.pcol)) else {
            return;
        };
        let pick = |c: &crate::projects::Cell| {
            if remove {
                c.undo.clone()
            } else if c.state == State::Linked {
                None
            } else {
                c.change.clone()
            }
        };
        let file = h
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = projects::short(cfg, &x.root);
        let to = if remove { "from" } else { "to" };
        // Adding to a project brings its CANON.md wiring along.
        let wired = |y: &projects::Project, changes: Vec<Change>| {
            let mut out = changes;
            if !remove && !out.is_empty() {
                out.extend(y.wiring_changes());
            }
            out
        };
        let mut items = vec![
            (
                format!("{file} {to} {name}"),
                wired(x, pick(&x.cells[self.pcol]).into_iter().collect()),
            ),
            (
                format!("every house file {to} {name}"),
                wired(x, x.cells.iter().filter_map(pick).collect()),
            ),
            (
                format!("{file} {to} every project"),
                p.list
                    .iter()
                    .flat_map(|y| wired(y, pick(&y.cells[self.pcol]).into_iter().collect()))
                    .collect(),
            ),
        ];
        // A choice that would do nothing is noise.
        items.retain(|(_, changes): &(String, Vec<Change>)| !changes.is_empty());
        if items.is_empty() {
            self.set_status(format!(
                "nothing to {} here",
                if remove { "delete" } else { "add" }
            ));
            return;
        }
        self.scope = Some(Scope {
            remove,
            verb: if remove { "Delete" } else { "Add" },
            items,
            picked: 0,
        });
    }

    fn confirm_scope(&mut self, remove: bool, verb: &str, label: &str, changes: Vec<Change>) {
        if changes.is_empty() {
            self.set_status(format!("nothing to {}: {label}", verb.to_lowercase()));
            return;
        }
        // Deleting a skill that exists nowhere else takes its typed name.
        if let [Change::DeleteDir { at }] = changes.as_slice() {
            let name = at
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.typed = Some(Typed {
                title: "delete skill".into(),
                message: format!(
                    "Delete {name}? It is a real folder that exists nowhere else, scripts and all, so it cannot be brought back."
                ),
                name,
                input: String::new(),
                changes,
            });
            return;
        }
        let runs = Self::runs(&changes);
        let title = verb.to_lowercase();
        self.confirm = Some(
            if remove {
                Confirm::gate(&title, format!("{verb} {label}?"), Action::Changes(changes))
            } else {
                Confirm::offer(&title, format!("{verb} {label}?"), Action::Changes(changes))
            }
            .runs(runs),
        );
    }

    /// The skills tab's `a` and `d`: this skill for this agent, every skill
    /// for this agent, or this skill for every agent; and adopting or deleting
    /// a skill an agent keeps itself, the delete taking its typed name.
    fn open_skill_scope(&mut self, remove: bool) {
        let (Some(plan), Some(cfg)) = (&self.plan, &self.cfg) else {
            return;
        };
        let rows = plan.skill_rows();
        let Some(&r) = rows.get(self.srow) else {
            return;
        };
        let agent = cfg.agents[self.col].name.clone();
        let label = plan.rows[r].label();
        let skill = label.strip_prefix("skill ").unwrap_or(&label).to_string();
        let pick = |c: &crate::plan::Cell| -> Option<Change> {
            let ch = if remove {
                c.undo.clone()
            } else {
                c.change.clone()
            };
            ch.filter(|c| !matches!(c, Change::Adopt { .. } | Change::DeleteDir { .. }))
        };
        let (to, verb) = if remove {
            ("from", "Delete")
        } else {
            ("to", "Link")
        };
        let mut items = vec![
            (
                format!("{skill}'s link {to} {agent}"),
                pick(&plan.cells[r][self.col]).into_iter().collect(),
            ),
            (
                format!("every skill link {to} {agent}"),
                rows.iter()
                    .filter_map(|x| pick(&plan.cells[*x][self.col]))
                    .collect(),
            ),
            (
                format!("{skill}'s link {to} every agent"),
                plan.cells[r].iter().filter_map(pick).collect::<Vec<_>>(),
            ),
        ];
        // A choice that would do nothing is noise.
        items.retain(|(_, changes): &(String, Vec<Change>)| !changes.is_empty());
        let cell = &plan.cells[r][self.col];
        if cell.state == State::Own {
            let (label, change) = if remove {
                (
                    format!("delete {skill} itself (type its name)"),
                    cell.undo.clone(),
                )
            } else {
                (
                    format!("adopt {skill} into your canon"),
                    cell.change.clone(),
                )
            };
            // A delete goes last, so the cursor never opens on it.
            if let Some(change) = change {
                items.push((label, vec![change]));
            }
        }
        if items.is_empty() {
            self.set_status(format!("nothing to {} for {skill}", verb.to_lowercase()));
            return;
        }
        self.scope = Some(Scope {
            remove,
            verb,
            items,
            picked: 0,
        });
    }

    /// Enter in the skills tab: the obvious thing for the cell.
    fn skill_toggle(&mut self) {
        let Some(r) = self.selected_row() else { return };
        let Some(plan) = &self.plan else { return };
        match plan.cells[r][self.col].state {
            State::Linked => self.agent_cell(false),
            State::Missing | State::Broken(_) | State::Own => self.agent_cell(true),
            _ => self.set_status("nothing to do here"),
        }
    }

    /// The lines of an agent's card: its setup rows, then one skills summary.
    pub(super) fn card_lines(&self) -> Vec<CardLine> {
        let Some(plan) = &self.plan else {
            return Vec::new();
        };
        let mut out: Vec<CardLine> = plan.setup_rows().into_iter().map(CardLine::Row).collect();
        out.push(CardLine::Skills);
        out
    }

    /// The plan row the selection stands on, or `None` on the skills summary.
    pub(super) fn selected_row(&self) -> Option<usize> {
        let plan = self.plan.as_ref()?;
        match self.view {
            View::Skills => plan.skill_rows().get(self.srow).copied(),
            _ => match self.card_lines().get(self.aline)? {
                CardLine::Row(r) => Some(*r),
                CardLine::Skills => None,
            },
        }
    }

    /// Every skill change for the selected agent, from its card's skills line.
    fn skills_bulk(&mut self, fix: bool) {
        let (Some(plan), Some(cfg)) = (&self.plan, &self.cfg) else {
            return;
        };
        let name = cfg.agents[self.col].name.clone();
        let changes: Vec<Change> = plan
            .skill_rows()
            .into_iter()
            .filter_map(|r| {
                let c = &plan.cells[r][self.col];
                if fix {
                    c.change.clone()
                } else {
                    c.undo.clone()
                }
            })
            .filter(|c| !matches!(c, Change::Adopt { .. } | Change::DeleteDir { .. }))
            .collect();
        if changes.is_empty() {
            self.set_status(if fix {
                format!("{name}'s skills have nothing to link (own ones are adopted one by one in the skills tab)")
            } else {
                format!("canonize made no skill links for {name}")
            });
            return;
        }
        let runs = Self::runs(&changes);
        self.confirm = Some(
            if fix {
                Confirm::offer(
                    "fix",
                    format!("Link {name}'s missing skills?"),
                    Action::Changes(changes),
                )
            } else {
                Confirm::gate(
                    "delete",
                    format!("Delete every skill link canonize made for {name}? Your canon stays."),
                    Action::Changes(changes),
                )
            }
            .runs(runs),
        );
    }

    /// Every skill link canonize made, for every agent, behind one red gate.
    fn remove_skills_all(&mut self) {
        let Some(plan) = &self.plan else { return };
        let mut changes: Vec<Change> = Vec::new();
        for r in plan.skill_rows() {
            for c in &plan.cells[r] {
                if let Some(u) = &c.undo
                    && !matches!(u, Change::DeleteDir { .. })
                    && !changes.contains(u)
                {
                    changes.push(u.clone());
                }
            }
        }
        if changes.is_empty() {
            self.set_status("canonize made no skill links");
            return;
        }
        let runs = Self::runs(&changes);
        self.confirm = Some(
            Confirm::gate(
                "delete all",
                "Delete every skill link canonize made, for every agent? Your canon stays."
                    .to_string(),
                Action::Changes(changes),
            )
            .runs(runs),
        );
    }

    /// Fix (`fix`) or take back (`!fix`) the selected cell.
    fn agent_cell(&mut self, fix: bool) {
        let Some(r) = self.selected_row() else {
            self.skills_bulk(fix);
            return;
        };
        let (Some(plan), Some(cfg)) = (&self.plan, &self.cfg) else {
            return;
        };
        let Some(row) = plan.rows.get(r) else {
            return;
        };
        let cell = &plan.cells[r][self.col];
        let what = format!("{}'s {}", cfg.agents[self.col].name, row.label());
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            self.set_status(if fix {
                format!("{what} has nothing to fix")
            } else {
                format!("{what} has nothing canonize made")
            });
            return;
        };
        if let Change::DeleteDir { at } = &change {
            let name = at
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.typed = Some(Typed {
                title: "delete skill".into(),
                message: format!(
                    "Delete {what}? It is a real folder that exists nowhere else, scripts and all, so it cannot be brought back. f would adopt it into your canon instead."
                ),
                name,
                input: String::new(),
                changes: vec![change],
            });
            return;
        }
        let adopt = matches!(change, Change::Adopt { .. });
        self.confirm = Some(
            if fix && adopt {
                Confirm::offer(
                    "adopt",
                    format!("Move {what} into your canon, scripts and all, and leave a link in its place?"),
                    Action::Changes(vec![change.clone()]),
                )
            } else if fix {
                Confirm::offer(
                    "fix",
                    format!("Fix {what}?"),
                    Action::Changes(vec![change.clone()]),
                )
            } else {
                Confirm::gate(
                    "delete",
                    format!("Delete {what}? Your files in the canon stay."),
                    Action::Changes(vec![change.clone()]),
                )
            }
            .runs(vec![change.command()]),
        );
    }

    /// Add or repoint (`fix`), or remove (`!fix`), the selected house import.
    fn project_cell(&mut self, fix: bool) {
        let (Some(p), Some(cfg)) = (&self.projects, &self.cfg) else {
            return;
        };
        let (Some(project), Some(house)) = (p.list.get(self.prow), p.house.get(self.pcol)) else {
            return;
        };
        let cell = &project.cells[self.pcol];
        let name = projects::short(cfg, &project.root);
        let file = house
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            self.set_status(if fix {
                format!("{name} already imports {file}")
            } else {
                format!("{name} does not import {file}")
            });
            return;
        };
        // Adding brings what makes the agents read CANON.md, the first time.
        let mut changes = vec![change.clone()];
        if fix {
            changes.extend(project.wiring_changes());
        }
        let runs = Self::runs(&changes);
        self.confirm = Some(
            if fix {
                let verb = if matches!(cell.state, State::Broken(_)) {
                    "Repoint"
                } else {
                    "Add"
                };
                Confirm::offer(
                    &verb.to_lowercase(),
                    format!("{verb} {name}'s import of {file}?"),
                    Action::Changes(changes),
                )
            } else {
                Confirm::gate(
                    "delete",
                    format!("Stop {name} importing {file}?"),
                    Action::Changes(changes),
                )
            }
            .runs(runs),
        );
    }

    /// Start the setup questions.
    fn propose_setup(&mut self) {
        self.wizard = Some(Wizard::new());
    }

    fn run_action(&mut self, what: &str, action: Action) {
        match action {
            Action::Changes(changes) => self.run_changes(what, changes),
            Action::Setup(choice) => match setup::apply(&choice) {
                Ok(()) => {
                    self.reload();
                    self.set_status(format!("set up in {}", tilde(&choice.root)));
                }
                Err(e) => self.note = Some(Note::alert("set up failed", format!("{e:#}"))),
            },
        }
    }

    fn run_changes(&mut self, what: &str, changes: Vec<Change>) {
        let mut failed = Vec::new();
        let total = changes.len();
        for c in changes {
            if let Err(e) = c.run() {
                failed.push(format!("{}: {e:#}", c.describe()));
            }
        }
        self.reload();
        // One short failure fits the status line; anything longer needs a box.
        if failed.is_empty() {
            self.set_status(format!("{what}: {total} done"));
        } else if failed.len() == 1 && failed[0].chars().count() <= 80 {
            self.set_failed(format!("{what} failed: {}", failed[0]));
        } else {
            self.note = Some(Note::alert(
                format!("{what} failed"),
                format!(
                    "{} of {total} could not be done:\n\n{}",
                    failed.len(),
                    failed.join("\n")
                ),
            ));
        }
    }

    fn show_check(&mut self) {
        let Some(report) = &self.report else { return };
        if report.problems.is_empty() {
            self.set_status(format!("canon is valid: {}", check::counts(report)));
        } else {
            self.note = Some(Note::reader(
                "validate",
                format!(
                    "{} problem{} in the source:\n\n{}",
                    report.problems.len(),
                    if report.problems.len() == 1 { "" } else { "s" },
                    report.problems.join("\n")
                ),
            ));
        }
    }
}

pub fn run() -> Result<()> {
    install_panic_hook();
    let mut terminal = setup()?;
    let mut app = App::new();
    let res = event_loop(&mut terminal, &mut app);
    teardown(&mut terminal)?;
    res
}

fn event_loop(terminal: &mut Term, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|f| render::ui(f, app))?;
        // Wake up only while a status is fading, so an idle dashboard costs nothing.
        let timeout = if app.live_status().is_some() {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(3600)
        };
        if !event::poll(timeout)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(Edit(path)) = app.on_key(key) {
            let editor = std::env::var("VISUAL")
                .or_else(|_| std::env::var("EDITOR"))
                .unwrap_or_else(|_| "vi".to_string());
            match run_suspended(terminal, &editor, &path)? {
                Some(_) => {
                    app.reload();
                    app.set_status(format!("reloaded after editing {}", tilde(&path)));
                }
                None => {
                    app.note = Some(Note::alert(
                        "could not open the editor",
                        format!(
                            "`{editor}` could not be started, so {} was not opened.\n\nSet `$EDITOR` to an editor on your PATH.",
                            tilde(&path)
                        ),
                    ))
                }
            }
        }
    }
    Ok(())
}

/// Leave the TUI, run the editor with the real terminal, then come back.
fn run_suspended(
    terminal: &mut Term,
    editor: &str,
    path: &PathBuf,
) -> Result<Option<std::process::ExitStatus>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    // `$EDITOR` may carry its own flags (`code -w`), so it is split on spaces.
    let mut parts = editor.split_whitespace();
    let status = parts
        .next()
        .and_then(|bin| Command::new(bin).args(parts).arg(path).status().ok());
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    Ok(status)
}

fn setup() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn teardown(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Make sure a panic does not leave the terminal in raw or alternate-screen mode.
fn install_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        hook(info);
    }));
}
