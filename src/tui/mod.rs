//! The dashboard: bare `canon`. One table, a row per rule file and skill and a
//! column per agent, with the selected cell explained beside it. Everything the
//! plain commands do is a key here, behind the house boxes.

mod alert;
mod confirm;
mod line_edit;
mod mcp;
mod render;
mod scope;
mod server_form;
mod typed;
mod widgets;
mod wizard;

use crate::check::{self, Report};
use crate::config::{self, Config, tilde};
use crate::houses::Houses;
use crate::mcp::Mcps;
use crate::plan::{self, Change, Plan, State};
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
pub(crate) use server_form::Saved;
use server_form::{Filled, ServerForm};
use std::cell::Cell;
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

/// A row of the houses tab: an agent, for every project, or one project.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HouseRow {
    Agent(usize),
    Project(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Agents,
    Skills,
    Houses,
    Mcp,
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
    pub(super) houses: Option<Houses>,
    pub(super) mcps: Option<Mcps>,
    pub(super) view: View,
    /// The selected line of the agent's card in the agents tab.
    pub(super) aline: usize,
    /// Whether the agents tab's keys go to the card (after Enter) or the list.
    pub(super) in_card: bool,
    /// The selected skill in the skills tab, an index into `Plan::skill_rows`.
    pub(super) srow: usize,
    /// The selected row and house file in the houses tab: agents first, each
    /// for every project, then the projects (`house_row`).
    pub(super) prow: usize,
    pub(super) pcol: usize,
    /// The MCP tab's selection, over the same rows as the houses tab.
    pub(super) mrow: usize,
    pub(super) mcol: usize,
    /// The first row each grid shows, kept between frames so moving back up
    /// scrolls only once the selection reaches the top.
    pub(super) stop: Cell<usize>,
    pub(super) ptop: Cell<usize>,
    pub(super) mtop: Cell<usize>,
    pub(super) report: Option<Report>,
    pub(super) col: usize,
    pub(super) confirm: Option<Confirm>,
    /// A delete waiting for its name to be typed.
    pub(super) typed: Option<Typed>,
    /// The add or remove scope being chosen in the skills or houses tab.
    pub(super) scope: Option<Scope>,
    /// The setup questions, while they are being answered.
    pub(super) wizard: Option<Wizard>,
    /// A new MCP server being described, from the MCP tab's `c`.
    pub(super) server_form: Option<ServerForm>,
    /// A box that is only read. It owns every key until closed.
    pub(super) note: Option<Note>,
    pub(super) show_help: bool,
    /// The first help row on screen; `render_help` clamps it to the end.
    pub(super) help_scroll: Cell<usize>,
    /// The `/` filter over the current tab's rows, dropped when the tab changes.
    pub(super) query: String,
    /// The cursor in `query`, as characters after it (`line_edit::edit`).
    pub(super) query_back: usize,
    /// Whether keys are going into `query` rather than to the tab.
    pub(super) searching: bool,
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
            houses: None,
            mcps: None,
            view: View::Agents,
            aline: 0,
            in_card: false,
            srow: 0,
            prow: 0,
            stop: Cell::new(0),
            ptop: Cell::new(0),
            pcol: 0,
            mrow: 0,
            mcol: 0,
            mtop: Cell::new(0),
            report: None,
            col: 0,
            confirm: None,
            typed: None,
            scope: None,
            wizard: None,
            server_form: None,
            note: None,
            show_help: false,
            help_scroll: Cell::new(0),
            query: String::new(),
            query_back: 0,
            searching: false,
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
            let r = *self.skill_list().get(self.srow)?;
            Some(p.rows[r].label())
        });
        match config::load() {
            Ok(cfg) => {
                self.plan = Some(Plan::build(&cfg));
                let projects = Projects::build(&cfg);
                self.mcps = Some(Mcps::build(&cfg, &projects));
                self.projects = Some(projects);
                self.houses = Some(Houses::build(&cfg));
                self.report = Some(check::run(&cfg.source));
                self.cfg = Some(cfg);
                self.load_error = None;
            }
            Err(e) => {
                self.cfg = None;
                self.plan = None;
                self.projects = None;
                self.houses = None;
                self.mcps = None;
                self.report = None;
                self.load_error = Some(format!("{e:#}"));
            }
        }
        let (_, cols) = self.size();
        self.col = self.col.min(cols.saturating_sub(1));
        self.aline = self.aline.min(self.card_lines().len().saturating_sub(1));
        let skills = self.skill_list();
        self.srow = self.srow.min(skills.len().saturating_sub(1));
        if let (Some(name), Some(p)) = (skill, &self.plan)
            && let Some(n) = skills.iter().position(|r| p.rows[*r].label() == name)
        {
            self.srow = n;
        }
        self.prow = self.prow.min(self.house_rows().saturating_sub(1));
        if let Some(p) = &self.projects {
            self.pcol = self.pcol.min(p.house.len().saturating_sub(1));
        }
        self.mrow = self.mrow.min(self.house_rows().saturating_sub(1));
        if let Some(m) = &self.mcps {
            self.mcol = self.mcol.min(m.servers.len().saturating_sub(1));
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

    /// Whether a box or a form is up, which owns every key while it is.
    fn boxed(&self) -> bool {
        self.show_help
            || self.wizard.is_some()
            || self.note.is_some()
            || self.server_form.is_some()
            || self.confirm.is_some()
            || self.typed.is_some()
            || self.scope.is_some()
    }

    fn on_key(&mut self, key: KeyEvent) -> Option<Edit> {
        let mut key = key;
        // Ctrl-C quits from a view and is Esc anywhere else, so a reflex one
        // steps out one level at a time.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if !self.boxed() && !self.searching {
                self.should_quit = true;
                return None;
            }
            key = KeyEvent::from(KeyCode::Esc);
        }
        if self.show_help {
            self.help_key(key);
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
        if let Some(form) = &mut self.server_form {
            match form.key(key) {
                Filled::Pending => {}
                Filled::Cancelled => {
                    self.server_form = None;
                    self.set_status("cancelled");
                }
                Filled::Done(saved) => self.save_server(saved),
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

        if self.searching {
            self.search_key(key);
            return None;
        }

        use KeyCode::*;
        if matches!(key.code, Tab | BackTab) {
            const VIEWS: [View; 4] = [View::Agents, View::Skills, View::Houses, View::Mcp];
            let i = VIEWS.iter().position(|v| *v == self.view).unwrap_or(0);
            let step = if key.code == Tab { 1 } else { VIEWS.len() - 1 };
            self.in_card = false;
            self.view = VIEWS[(i + step) % VIEWS.len()];
            // A filter belongs to the list it was typed over; carried into the
            // next tab it would hide rows nobody searched for.
            self.query.clear();
            self.query_back = 0;
            return None;
        }
        // The agents list filters, the card opened from it does not.
        let listing = self.load_error.is_none() && !(self.view == View::Agents && self.in_card);
        if listing && key.code == Char('/') {
            self.query.clear();
            self.query_back = 0;
            self.searching = true;
            self.requery();
            return None;
        }
        if listing && key.code == Esc && !self.query.is_empty() {
            self.query.clear();
            self.query_back = 0;
            self.requery();
            self.set_status("filter cleared");
            return None;
        }
        if self.view == View::Mcp
            && let Some(edit) = self.mcp_key(key)
        {
            return edit;
        }
        if self.view == View::Houses {
            let rows = self.house_rows();
            let cols = self.projects.as_ref().map_or(0, |p| p.house.len());
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
                    self.house_toggle(Some(true));
                    return None;
                }
                // A grid of imports reads like checkboxes, so Enter toggles.
                Enter | Char(' ') => {
                    self.house_toggle(None);
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
                Char('o') => {
                    return match self.house_row()? {
                        HouseRow::Project(i) => self
                            .projects
                            .as_ref()?
                            .list
                            .get(i)
                            .map(|x| Edit(x.host.clone())),
                        HouseRow::Agent(a) => {
                            let at = &self.houses.as_ref()?.cells.get(self.pcol)?.get(a)?.at;
                            at.is_file().then(|| Edit(at.clone()))
                        }
                    };
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
        // Keys only the agents tab has must not reach it from the houses tab.
        if matches!(self.view, View::Houses | View::Mcp)
            && matches!(key.code, Char('f' | 'd') | Enter)
        {
            return None;
        }
        let (_, agents) = self.size();
        let lines = self.card_lines().len();
        let skills = self.skill_list().len();
        // Agents: j/k walks the list, or the card once Enter has opened it.
        // Skills: j/k picks the skill, h/l the agent.
        if self.view == View::Agents {
            let shown = self.agent_rows();
            let at = shown.iter().position(|&a| a == self.col);
            if !self.in_card {
                let to = match key.code {
                    Down | Char('j') => at.map_or(0, |i| i + 1).min(shown.len().saturating_sub(1)),
                    Up | Char('k') => at.map_or(0, |i| i.saturating_sub(1)),
                    Char('g') | Home => 0,
                    Char('G') | End => shown.len().saturating_sub(1),
                    Enter => {
                        self.in_card = at.is_some();
                        return None;
                    }
                    _ => usize::MAX,
                };
                if to != usize::MAX {
                    if let Some(&a) = shown.get(to) {
                        self.col = a;
                    }
                    return None;
                }
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
            _ => (&mut self.aline, lines, &mut none, 0),
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
        self.anywhere(key)
    }

    /// Keys while the help is up: it scrolls, and closes on esc, `q` or `?`.
    fn help_key(&mut self, key: KeyEvent) {
        use KeyCode::*;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half = 10;
        let top = self.help_scroll.get();
        match key.code {
            Char('?' | 'q') | Esc => self.show_help = false,
            Char('d') if ctrl => self.help_scroll.set(top.saturating_add(half)),
            Char('u') if ctrl => self.help_scroll.set(top.saturating_sub(half)),
            PageDown => self.help_scroll.set(top.saturating_add(half)),
            PageUp => self.help_scroll.set(top.saturating_sub(half)),
            Char('j') | Down => self.help_scroll.set(top.saturating_add(1)),
            Char('k') | Up => self.help_scroll.set(top.saturating_sub(1)),
            Char('g') | Home => self.help_scroll.set(0),
            // `render_help` clamps this to the last screenful.
            Char('G') | End => self.help_scroll.set(usize::MAX),
            _ => {}
        }
    }

    /// The keys every tab answers to.
    fn anywhere(&mut self, key: KeyEvent) -> Option<Edit> {
        use KeyCode::*;
        match key.code {
            Char('q') => self.should_quit = true,
            Char('?') => {
                self.show_help = true;
                self.help_scroll.set(0);
            }
            Char('r') => {
                self.reload();
                self.set_status("refreshed");
            }
            Char('f') | Enter => self.agent_cell(true),
            Char('F') => self.offer_fix_all(),
            Char('d') => self.agent_cell(false),
            Char('v') => self.show_check(),
            Char('s') if self.load_error.is_some() => self.propose_setup(),
            Char('E') => {
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
            plan::dedup(
                rows.into_iter()
                    .flat_map(|r| plan.cells[r].iter().filter_map(|c| c.change.clone()))
                    .filter(not_adopt),
            )
        };
        // F fixes what the tab shows, nothing beyond it.
        let (changes, what) = match self.view {
            View::Agents => (rows_changes(plan.setup_rows()), "every agent's setup"),
            View::Skills => {
                let mut c = rows_changes(plan.skill_rows());
                c.extend(plan.stale_for(None).into_iter().cloned());
                (c, "every missing or broken skill link")
            }
            View::Houses => {
                let mut c = self
                    .houses
                    .as_ref()
                    .map(|h| h.fixes(None))
                    .unwrap_or_default();
                c.extend(
                    self.projects
                        .as_ref()
                        .map(|p| p.fixes())
                        .unwrap_or_default(),
                );
                (
                    plan::dedup(c.into_iter()),
                    "every broken convention import and CANON.md wiring",
                )
            }
            View::Mcp => (
                self.mcps
                    .as_ref()
                    .map(|m| m.fixes(None))
                    .unwrap_or_default(),
                "every MCP server that differs from your canon",
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

    /// The add or remove choices for the selected cell, nearest first: this
    /// cell, every house file in this project, then its house file in every
    /// project, the widest and least likely.
    fn open_scope(&mut self, remove: bool) {
        let prow = match self.house_row() {
            Some(HouseRow::Project(i)) => i,
            Some(HouseRow::Agent(a)) => return self.open_agent_scope(a, remove),
            None => return,
        };
        let (Some(p), Some(cfg)) = (&self.projects, &self.cfg) else {
            return;
        };
        let (Some(x), Some(h)) = (p.list.get(prow), p.house.get(self.pcol)) else {
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
                plan::dedup(wired(x, pick(&x.cells[self.pcol]).into_iter().collect()).into_iter()),
            ),
            (
                format!("every convention {to} {name}"),
                plan::dedup(wired(x, x.cells.iter().filter_map(pick).collect()).into_iter()),
            ),
            (
                format!("{file} {to} every project"),
                plan::dedup(
                    p.list
                        .iter()
                        .flat_map(|y| wired(y, pick(&y.cells[self.pcol]).into_iter().collect())),
                ),
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
        // Deleting a server from the canon takes its typed name, since its
        // kept token exists nowhere else.
        if let Some((name, token)) = undefined_server(&changes) {
            let token = if token.is_some_and(|t| t.is_file()) {
                " Its kept token is deleted too, and it exists nowhere else."
            } else {
                ""
            };
            self.typed = Some(Typed {
                title: "delete server".into(),
                message: format!(
                    "Delete {name} from your canon? It comes out of every agent and project that has it, and out of mcp.toml.{token}"
                ),
                name: name.to_string(),
                input: String::new(),
                back: 0,
                changes,
            });
            return;
        }
        // Deleting a real folder takes its typed name.
        if let Some(at) = deleted_dir(&changes) {
            let name = at
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let in_canon = self
                .cfg
                .as_ref()
                .is_some_and(|cfg| at.starts_with(&cfg.source.skills));
            let message = if in_canon {
                format!(
                    "Delete {name} from your canon? The folder goes, scripts and all, and so does every agent's link to it."
                )
            } else {
                format!(
                    "Delete {name}? It is a real folder that exists nowhere else, scripts and all, so it cannot be brought back."
                )
            };
            self.typed = Some(Typed {
                title: "delete skill".into(),
                message,
                name,
                input: String::new(),
                back: 0,
                changes,
            });
            return;
        }
        let runs = Self::runs(&changes);
        self.confirm = Some(
            if remove {
                Confirm::gate(
                    &gate_title(&changes),
                    format!("{verb} {label}?"),
                    Action::Changes(changes),
                )
            } else {
                Confirm::offer(
                    &verb.to_lowercase(),
                    format!("{verb} {label}?"),
                    Action::Changes(changes),
                )
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
        let Some(&r) = self.skill_list().get(self.srow) else {
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
                plan::dedup(pick(&plan.cells[r][self.col]).into_iter()),
            ),
            (
                format!("every skill link {to} {agent}"),
                plan::dedup(rows.iter().filter_map(|x| pick(&plan.cells[*x][self.col]))),
            ),
            (
                format!("{skill}'s link {to} every agent"),
                plan::dedup(plan.cells[r].iter().filter_map(pick)),
            ),
        ];
        // A choice that would do nothing is noise.
        items.retain(|(_, changes): &(String, Vec<Change>)| !changes.is_empty());
        // The canon's own copy: deleting it takes every link with it, since a
        // link left behind points at nothing.
        if remove
            && rows
                .iter()
                .position(|&x| x == r)
                .is_some_and(|i| i < plan.canon_skills)
        {
            let at = cfg.source.skills.join(&skill);
            let mut changes: Vec<Change> = Vec::new();
            for c in &plan.cells[r] {
                // A folder-mode agent's cell is its whole skills folder, which
                // this delete must not take.
                if let Some(Change::Unlink { at }) = &c.undo
                    && at.file_name().is_some_and(|n| n == skill.as_str())
                    && !changes
                        .iter()
                        .any(|x| matches!(x, Change::Unlink { at: a } if a == at))
                {
                    changes.push(Change::Unlink { at: at.clone() });
                }
            }
            changes.push(Change::DeleteDir { at });
            items.push((
                format!("delete {skill} from your canon (type its name)"),
                vec![Change::Batch {
                    what: format!("delete {skill} from your canon and every link to it"),
                    changes,
                }],
            ));
        }
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
        match self.view {
            View::Skills => self.skill_list().get(self.srow).copied(),
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
                    &gate_title(&changes),
                    format!("Delete every skill link canonize made for {name}? Your canon stays."),
                    Action::Changes(changes),
                )
            }
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
                back: 0,
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
                    &gate_title(std::slice::from_ref(&change)),
                    format!("Delete {what}? Your files in the canon stay."),
                    Action::Changes(vec![change.clone()]),
                )
            }
            .runs(vec![change.command()]),
        );
    }

    /// The houses tab's `a` and `d` on an agent's row: this cell, every house
    /// file for this agent, or this house file for every agent.
    fn open_agent_scope(&mut self, a: usize, remove: bool) {
        let (Some(h), Some(cfg)) = (&self.houses, &self.cfg) else {
            return;
        };
        let (Some(house), Some(row)) = (h.house.get(self.pcol), h.cells.get(self.pcol)) else {
            return;
        };
        let pick = |c: &crate::plan::Cell| {
            if remove {
                c.undo.clone()
            } else if c.state == State::Linked {
                None
            } else {
                c.change.clone()
            }
        };
        let file = house
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let agent = &cfg.agents[a].name;
        let to = if remove { "from" } else { "to" };
        let mut items = vec![
            (
                format!("{file} {to} {agent}, in every project"),
                plan::dedup(pick(&row[a]).into_iter()),
            ),
            (
                format!("every convention {to} {agent}"),
                plan::dedup(h.cells.iter().filter_map(|r| pick(&r[a]))),
            ),
            (
                format!("{file} {to} every agent"),
                plan::dedup(row.iter().filter_map(pick)),
            ),
        ];
        items.retain(|(_, changes): &(String, Vec<Change>)| !changes.is_empty());
        if items.is_empty() {
            self.set_status(match (row[a].state.why(), remove) {
                (Some(why), false) => format!("{agent}: {why}"),
                (None, false) if row[a].state == State::Na => {
                    format!("{agent} has no way to read another file")
                }
                _ => format!("nothing to {} here", if remove { "delete" } else { "add" }),
            });
            return;
        }
        self.scope = Some(Scope {
            remove,
            verb: if remove { "Delete" } else { "Add" },
            items,
            picked: 0,
        });
    }

    /// What the `/` filter keeps of a row named `name`: all of them with no
    /// query, else the ones holding it, case aside, or holding its characters
    /// in order, so `cl` finds `dev/crates/cli`.
    fn kept(&self, name: &str) -> bool {
        let q = self.query.trim().to_lowercase();
        let name = name.to_lowercase();
        let mut chars = name.chars();
        q.is_empty() || name.contains(&q) || q.chars().all(|c| chars.any(|h| h == c))
    }

    /// The agents the agents tab lists, by index into `cfg.agents`.
    pub(super) fn agent_rows(&self) -> Vec<usize> {
        self.cfg.as_ref().map_or(Vec::new(), |c| {
            (0..c.agents.len())
                .filter(|&a| self.kept(&c.agents[a].name))
                .collect()
        })
    }

    /// The skills tab's rows, as plan rows; `srow` indexes this.
    pub(super) fn skill_list(&self) -> Vec<usize> {
        self.plan.as_ref().map_or(Vec::new(), |p| {
            p.skill_rows()
                .into_iter()
                .filter(|&r| self.kept(&p.rows[r].label()))
                .collect()
        })
    }

    /// Rows in the conventions and MCP tabs: every agent, then every project,
    /// before the filter, so a detail pane sized from them keeps its height.
    pub(super) fn all_places(&self) -> Vec<HouseRow> {
        let projects = self.projects.as_ref().map_or(0, |p| p.list.len());
        self.grid_agents()
            .into_iter()
            .map(HouseRow::Agent)
            .chain((0..projects).map(HouseRow::Project))
            .collect()
    }

    /// The name a conventions or MCP row goes by, which is what `/` matches.
    pub(super) fn place_name(&self, row: HouseRow) -> String {
        match (row, &self.cfg, &self.projects) {
            (HouseRow::Agent(a), Some(c), _) => c.agents[a].name.clone(),
            (HouseRow::Project(i), Some(c), Some(p)) => projects::short(c, &p.list[i].root),
            _ => String::new(),
        }
    }

    /// The conventions and MCP tabs' rows; `prow` and `mrow` index this.
    pub(super) fn place_rows(&self) -> Vec<HouseRow> {
        self.all_places()
            .into_iter()
            .filter(|&r| self.kept(&self.place_name(r)))
            .collect()
    }

    /// Rows in the houses tab: every agent, then every project.
    pub(super) fn house_rows(&self) -> usize {
        self.place_rows().len()
    }

    /// Rows the current tab shows once filtered.
    pub(super) fn row_count(&self) -> usize {
        match self.view {
            View::Agents => self.agent_rows().len(),
            View::Skills => self.skill_list().len(),
            View::Houses | View::Mcp => self.house_rows(),
        }
    }

    /// Keep the selection on a row the filter still shows.
    fn requery(&mut self) {
        let shown = self.agent_rows();
        if !shown.contains(&self.col)
            && self.view == View::Agents
            && let Some(&a) = shown.first()
        {
            self.col = a;
        }
        self.srow = self.srow.min(self.skill_list().len().saturating_sub(1));
        let places = self.house_rows().saturating_sub(1);
        self.prow = self.prow.min(places);
        self.mrow = self.mrow.min(places);
    }

    /// A key while `/` is being typed: every letter goes into the query and the
    /// list narrows under it. Enter keeps the filter and hands the keys back
    /// to the tab; Esc drops it.
    fn search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.query.clear();
                self.query_back = 0;
                self.searching = false;
            }
            KeyCode::Enter => self.searching = false,
            KeyCode::Down | KeyCode::Up => {
                let down = key.code == KeyCode::Down;
                let n = self.row_count().saturating_sub(1);
                let step = |at: usize| {
                    if down {
                        (at + 1).min(n)
                    } else {
                        at.saturating_sub(1)
                    }
                };
                match self.view {
                    View::Agents => {
                        let shown = self.agent_rows();
                        let at = shown.iter().position(|&a| a == self.col).unwrap_or(0);
                        if let Some(&a) = shown.get(step(at)) {
                            self.col = a;
                        }
                    }
                    View::Skills => self.srow = step(self.srow),
                    View::Houses => self.prow = step(self.prow),
                    View::Mcp => self.mrow = step(self.mrow),
                }
                return;
            }
            _ => {
                line_edit::edit(&mut self.query, &mut self.query_back, key);
            }
        }
        self.requery();
    }

    /// The agents the conventions and MCP tabs have a row for: the ones
    /// installed and enabled, since a row for any other could do nothing.
    pub(super) fn grid_agents(&self) -> Vec<usize> {
        self.cfg.as_ref().map_or(Vec::new(), |c| {
            (0..c.agents.len())
                .filter(|&a| c.agents[a].active())
                .collect()
        })
    }

    /// What the houses tab's row `r` is, counted over the filtered rows.
    pub(super) fn house_row_at(&self, r: usize) -> Option<HouseRow> {
        self.place_rows().get(r).copied()
    }

    pub(super) fn house_row(&self) -> Option<HouseRow> {
        self.house_row_at(self.prow)
    }

    /// Enter (`None`, a toggle) or `f` (`Some(true)`) in the houses tab.
    fn house_toggle(&mut self, fix: Option<bool>) {
        let imported = |c: Option<&State>| c.is_some_and(|s| *s == State::Linked);
        match self.house_row() {
            Some(HouseRow::Agent(a)) => {
                let now = imported(
                    self.houses
                        .as_ref()
                        .and_then(|h| h.cells.get(self.pcol)?.get(a))
                        .map(|c| &c.state),
                );
                self.house_cell(a, fix.unwrap_or(!now));
            }
            Some(HouseRow::Project(i)) => {
                let now = imported(
                    self.projects
                        .as_ref()
                        .and_then(|p| p.list.get(i)?.cells.get(self.pcol))
                        .map(|c| &c.state),
                );
                self.project_cell(i, fix.unwrap_or(!now));
            }
            None => {}
        }
    }

    /// Add or repoint (`fix`), or remove (`!fix`), project `prow`'s import of
    /// the selected house file.
    fn project_cell(&mut self, prow: usize, fix: bool) {
        let (Some(p), Some(cfg)) = (&self.projects, &self.cfg) else {
            return;
        };
        let (Some(project), Some(house)) = (p.list.get(prow), p.house.get(self.pcol)) else {
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
                    &gate_title(&changes),
                    format!("Stop {name} importing {file}?"),
                    Action::Changes(changes),
                )
            }
            .runs(runs),
        );
    }

    /// Add or repoint (`fix`), or take back (`!fix`), agent `a`'s import of
    /// the selected house file.
    fn house_cell(&mut self, a: usize, fix: bool) {
        let (Some(h), Some(cfg)) = (&self.houses, &self.cfg) else {
            return;
        };
        let (Some(house), Some(cell)) = (
            h.house.get(self.pcol),
            h.cells.get(self.pcol).and_then(|r| r.get(a)),
        ) else {
            return;
        };
        let agent = &cfg.agents[a].name;
        let file = house
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            self.set_status(match (&cell.state, fix) {
                (State::Linked, true) => format!("{agent} already reads {file}"),
                (_, false) => format!("{agent} does not read {file}"),
                (s, true) => match s.why() {
                    Some(why) => format!("{agent}: {why}"),
                    None => format!("{agent} has no way to read another file"),
                },
            });
            return;
        };
        let runs = Self::runs(std::slice::from_ref(&change));
        self.confirm = Some(
            if fix {
                let verb = if matches!(cell.state, State::Broken(_)) {
                    "Repoint"
                } else {
                    "Add"
                };
                Confirm::offer(
                    &verb.to_lowercase(),
                    format!("{verb} {file} for {agent}, in every project?"),
                    Action::Changes(vec![change]),
                )
            } else {
                Confirm::gate(
                    &gate_title(std::slice::from_ref(&change)),
                    format!("Stop {agent} reading {file}?"),
                    Action::Changes(vec![change]),
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
    // A Ctrl-C meant for the editor reaches canonize too, and would end it.
    let caught = swallow_interrupts();
    // `$EDITOR` may carry its own flags (`code -w`), so it is split on spaces.
    let mut parts = editor.split_whitespace();
    let status = parts
        .next()
        .and_then(|bin| Command::new(bin).args(parts).arg(path).status().ok());
    for id in caught {
        signal_hook::low_level::unregister(id);
    }
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    Ok(status)
}

/// Catch Ctrl-C and Ctrl-\ and do nothing with them, while a child that owns
/// the terminal acts on them. A caught signal is reset to the default in an
/// exec'd child, so this never reaches it, unlike `SIG_IGN`, which it would
/// inherit. Hand the ids to `signal_hook::low_level::unregister` to stop.
fn swallow_interrupts() -> Vec<signal_hook::SigId> {
    use signal_hook::consts::{SIGINT, SIGQUIT};
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    [SIGINT, SIGQUIT]
        .into_iter()
        .filter_map(|signal| signal_hook::flag::register(signal, seen.clone()).ok())
        .collect()
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

/// The real folder a delete would remove, inside a batch as well, so the typed
/// gate is never skipped by wrapping the delete in one.
fn deleted_dir(changes: &[Change]) -> Option<&PathBuf> {
    changes.iter().find_map(|c| match c {
        Change::DeleteDir { at } => Some(at),
        Change::Batch { changes, .. } => deleted_dir(changes),
        _ => None,
    })
}

/// The server a delete would take out of the canon, and its kept token,
/// inside a batch as well.
fn undefined_server(changes: &[Change]) -> Option<(&str, Option<&PathBuf>)> {
    changes.iter().find_map(|c| match c {
        Change::McpUndefine { name, token, .. } => Some((name.as_str(), token.as_ref())),
        Change::Batch { changes, .. } => undefined_server(changes),
        _ => None,
    })
}

/// A gate's title, naming what it is in front of by the first change it runs.
fn gate_title(changes: &[Change]) -> String {
    let many = changes.len() > 1;
    let noun = |one: &str, more: &str| format!("delete {}", if many { more } else { one });
    match changes.first() {
        Some(Change::Batch { changes, .. }) => gate_title(changes),
        Some(Change::Unlink { .. }) => noun("link", "links"),
        Some(Change::RemoveImport { .. }) => noun("import", "imports"),
        Some(Change::McpRemove { .. }) => noun("server entry", "server entries"),
        Some(c) => c.verb().to_string(),
        None => "delete".to_string(),
    }
}
