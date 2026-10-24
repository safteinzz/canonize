//! The workspaces tab's keys: a list of folders linked to projects, and the
//! card of the one under the cursor, opened with Enter: a line per file
//! canonize writes there, its CONTEXT.md, a line per project, and per
//! convention and MCP server it can have.

use super::confirm::{Action, Confirm};
use super::link_form::LinkForm;
use super::{App, Edit, gate_title};
use crate::cli::house_label;
use crate::config::tilde;
use crate::plan::{self, CANON_FILE, Change, State};
use crate::projects;
use crate::workspaces::{self, CONTEXT_FILE, Entry, Reach};
use crossterm::event::{KeyCode, KeyEvent};
use std::path::{Path, PathBuf};

/// One line of a workspace's card.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SpaceLine {
    /// A file canonize writes at its top, by the first agent that reads it.
    File(usize),
    /// One of its projects, by index into `Entry::projects`.
    Project(usize),
    /// Its CONTEXT.md, the user's.
    Context,
    /// A convention of the canon, by column.
    Convention(usize),
    /// An MCP server of the canon, by index into `Mcps::servers`.
    Server(usize),
}

const NONE_YET: &str = "no workspaces yet: c links a folder to a project";

impl App {
    /// A key in the workspaces tab: `Some` when the tab took it.
    pub(super) fn space_key(&mut self, key: KeyEvent) -> Option<Option<Edit>> {
        use KeyCode::*;
        if self.in_space {
            return self.space_card_key(key);
        }
        let rows = self.space_rows().len();
        match key.code {
            Down | Char('j') => self.wrow = (self.wrow + 1).min(rows.saturating_sub(1)),
            Up | Char('k') => self.wrow = self.wrow.saturating_sub(1),
            Char('g') | Home => self.wrow = 0,
            Char('G') | End => self.wrow = rows.saturating_sub(1),
            Enter => match self.space() {
                Some(_) => {
                    self.in_space = true;
                    self.sline = 0;
                }
                None => self.set_status(NONE_YET),
            },
            Char('f') => self.fix_space(),
            Char('c') => self.link_form = Some(LinkForm::new(self.project_names(&[]))),
            Char('d') => self.unlink_space(),
            _ => return None,
        }
        Some(None)
    }

    /// A key in a workspace's card.
    fn space_card_key(&mut self, key: KeyEvent) -> Option<Option<Edit>> {
        use KeyCode::*;
        let lines = self.space_lines().len();
        match key.code {
            Down | Char('j') => self.sline = (self.sline + 1).min(lines.saturating_sub(1)),
            Up | Char('k') => self.sline = self.sline.saturating_sub(1),
            Char('g') | Home => self.sline = 0,
            Char('G') | End => self.sline = lines.saturating_sub(1),
            Esc => self.in_space = false,
            Enter | Char(' ') => return Some(self.space_enter()),
            Char('f') => self.space_fix_line(),
            Char('d') => self.space_delete_line(),
            Char('a') => self.add_project(),
            Char('o') => return Some(self.space_open()),
            _ => return None,
        }
        Some(None)
    }

    /// The workspace under the cursor, as an index into `Workspaces::list`.
    pub(super) fn space(&self) -> Option<usize> {
        self.space_rows().get(self.wrow).copied()
    }

    fn entry(&self) -> Option<(usize, &Entry)> {
        let i = self.space()?;
        Some((i, self.spaces.as_ref()?.list.get(i)?))
    }

    /// Where the agents in `e` get convention `c` from outside it.
    pub(super) fn convention_reach(&self, e: &Entry, c: usize) -> Option<Reach> {
        let (cfg, h) = (self.cfg.as_ref()?, self.houses.as_ref()?);
        Some(Reach::new(cfg, e.brought(c), h.cells.get(c)?))
    }

    /// Which agents have server `k` everywhere, so in every workspace too.
    pub(super) fn server_reach(&self, k: usize) -> Option<Reach> {
        let (cfg, m) = (self.cfg.as_ref()?, self.mcps.as_ref()?);
        Some(Reach::new(cfg, Vec::new(), m.cells.get(k)?))
    }

    /// The lines of workspace `i`'s card: one per file, however many agents
    /// read it, then its CONTEXT.md, its projects, every convention and every
    /// MCP server of the canon.
    pub(super) fn space_lines_of(&self, i: usize) -> Vec<SpaceLine> {
        let (Some(s), Some(p), Some(m)) = (&self.spaces, &self.projects, &self.mcps) else {
            return Vec::new();
        };
        let Some(e) = s.list.get(i) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen: Vec<&Path> = Vec::new();
        for a in self.grid_agents() {
            let at = e.cells[a].at.as_path();
            if !seen.contains(&at) {
                seen.push(at);
                out.push(SpaceLine::File(a));
            }
        }
        out.push(SpaceLine::Context);
        out.extend((0..e.projects.len()).map(SpaceLine::Project));
        out.extend((0..p.house.len()).map(SpaceLine::Convention));
        out.extend((0..m.servers.len()).map(SpaceLine::Server));
        out
    }

    pub(super) fn space_lines(&self) -> Vec<SpaceLine> {
        self.space()
            .map_or_else(Vec::new, |i| self.space_lines_of(i))
    }

    pub(super) fn space_line(&self) -> Option<SpaceLine> {
        self.space_lines().get(self.sline).copied()
    }

    /// The projects canonize found, by the name `canon status` prints, but
    /// for the ones in `skip`.
    fn project_names(&self, skip: &[PathBuf]) -> Vec<String> {
        let (Some(cfg), Some(p)) = (&self.cfg, &self.projects) else {
            return Vec::new();
        };
        p.list
            .iter()
            .filter(|x| !skip.contains(&x.root))
            .map(|x| projects::short(cfg, &x.root))
            .collect()
    }

    /// Point the folder the form names at one more project, the setting first
    /// and the files after it, stopping at the first that fails. A refusal
    /// stays in the form.
    pub(super) fn save_link(&mut self, workspace: String, project: String) {
        let (Some(cfg), Some(p), Some(form)) = (&self.cfg, &self.projects, &mut self.link_form)
        else {
            return;
        };
        let planned = crate::cli::project_index(cfg, p, &project).and_then(|i| {
            let root = p.list[i].root.clone();
            crate::workspaces::link(cfg, p, &workspace, std::slice::from_ref(&root))
                .map(|l| (l, root))
        });
        let (l, root) = match planned {
            Ok(x) => x,
            Err(e) => {
                form.error = Some(crate::said(&e));
                return;
            }
        };
        let name = projects::short(cfg, &root);
        let all: Vec<String> = l
            .entry
            .projects
            .iter()
            .map(|x| projects::short(cfg, x))
            .collect();
        let key = l.key.clone();
        let foreign = l.entry.foreign();
        let changes: Vec<Change> = l
            .setting
            .into_iter()
            .chain(l.entry.file_fixes(None))
            .collect();
        self.link_form = None;
        if changes.is_empty() {
            self.set_status(format!("{key} already points at {name}"));
            return;
        }
        let what = format!("point {key} at {name}");
        self.run_changes("link", vec![Change::Batch { what, changes }]);
        if self.note.is_none() && !self.status_failed {
            match foreign.first() {
                Some(why) => self.set_failed(format!("{why}: move it aside, then f")),
                None => self.set_status(format!("{key} points at {}", all.join(", "))),
            }
        }
        if let Some(s) = &self.spaces
            && let Some(i) = s.list.iter().position(|e| e.key == key)
            && let Some(r) = self.space_rows().iter().position(|&x| x == i)
        {
            self.wrow = r;
        }
    }

    /// `f` in the list: write what is missing or stale in this workspace, its
    /// files, its convention imports and its MCP servers.
    fn fix_space(&mut self) {
        let (Some((i, e)), Some(m)) = (self.entry(), &self.mcps) else {
            self.set_status(NONE_YET);
            return;
        };
        let root = tilde(&e.root);
        let changes = plan::dedup(e.fixes(None).into_iter().chain(m.space_fixes(i)));
        if changes.is_empty() {
            let why = if let Some(gone) = e.gone.first() {
                Some(format!(
                    "the project at {} is not there: in its card, a adds where it went and d on it drops it",
                    tilde(gone)
                ))
            } else {
                e.foreign()
                    .first()
                    .map(|why| format!("{why}: move it aside, then f"))
            };
            match why {
                Some(why) => self.set_failed(why),
                None if !e.here => self.set_status(format!("{root} is not on this machine")),
                None => self.set_status(format!("{root} has nothing to fix")),
            }
            return;
        }
        let runs = Self::runs(&changes);
        self.confirm = Some(
            Confirm::offer("fix", format!("Fix {root}?"), Action::Changes(changes)).runs(runs),
        );
    }

    /// `d` in the list: take back everything canonize made in this workspace
    /// and forget it, behind the red gate.
    fn unlink_space(&mut self) {
        let (Some((i, e)), Some(cfg), Some(m)) = (self.entry(), &self.cfg, &self.mcps) else {
            self.set_status(NONE_YET);
            return;
        };
        let servers = m.spaces.get(i).map_or(&[][..], Vec::as_slice);
        let changes = workspaces::unlinking(cfg, e, servers);
        let message = unlink_message(&e.root);
        let runs = Self::runs(&changes);
        self.confirm = Some(Confirm::gate("unlink", message, Action::Changes(changes)).runs(runs));
    }

    /// Enter in the card: the line's own verb.
    fn space_enter(&mut self) -> Option<Edit> {
        match self.space_line()? {
            SpaceLine::File(a) => self.space_file(a, true),
            SpaceLine::Project(_) => self.set_status("a adds a project · d drops this one"),
            SpaceLine::Context => return self.space_open(),
            SpaceLine::Convention(c) => self.space_convention(c, None),
            SpaceLine::Server(k) => self.space_server(k, None),
        }
        None
    }

    /// `f` in the card: write, add or repoint what the line names.
    fn space_fix_line(&mut self) {
        match self.space_line() {
            Some(SpaceLine::File(a)) => self.space_file(a, true),
            Some(SpaceLine::Convention(c)) => self.space_convention(c, Some(true)),
            Some(SpaceLine::Server(k)) => self.space_server(k, Some(true)),
            Some(_) => self.set_status("nothing to fix here"),
            None => {}
        }
    }

    /// `d` in the card: take back what the line names, or drop the project.
    fn space_delete_line(&mut self) {
        match self.space_line() {
            Some(SpaceLine::File(a)) => self.space_file(a, false),
            Some(SpaceLine::Project(k)) => self.drop_project(k),
            Some(SpaceLine::Context) => self.set_status(format!(
                "{CONTEXT_FILE} is yours: canonize never deletes it"
            )),
            Some(SpaceLine::Convention(c)) => self.space_convention(c, Some(false)),
            Some(SpaceLine::Server(k)) => self.space_server(k, Some(false)),
            None => {}
        }
    }

    /// Write (`fix`) or delete the file agent `a` reads at the workspace's top.
    fn space_file(&mut self, a: usize, fix: bool) {
        let Some((_, e)) = self.entry() else { return };
        let c = &e.cells[a];
        let name = file_name(&c.at);
        let root = tilde(&e.root);
        let change = if fix { &c.change } else { &c.undo };
        let Some(change) = change.clone() else {
            let (failed, msg) = match (&c.state, fix) {
                (_, _) if !e.here => (false, format!("{root} is not on this machine")),
                (State::Foreign(why), _) => (true, format!("{why}: move it aside, then f")),
                (State::Broken(why), true) => (
                    true,
                    format!("{why}: a adds where it went, d on it drops it"),
                ),
                (_, true) => (false, format!("{name} has nothing to fix")),
                (_, false) => (false, format!("canonize wrote no {name} in {root}")),
            };
            if failed {
                self.set_failed(msg);
            } else {
                self.set_status(msg);
            }
            return;
        };
        let runs = vec![change.command()];
        self.confirm = Some(
            if fix {
                Confirm::offer(
                    "fix",
                    format!("Fix {name} in {root}?"),
                    Action::Changes(vec![change]),
                )
            } else {
                Confirm::gate(
                    "delete file",
                    format!("Delete the {name} canonize wrote in {root}? f writes it again."),
                    Action::Changes(vec![change]),
                )
            }
            .runs(runs),
        );
    }

    /// `d` on a project line: drop it from the workspace, or unlink the
    /// workspace when it is the last one.
    fn drop_project(&mut self, k: usize) {
        let (Some((i, e)), Some(cfg), Some(p), Some(m)) =
            (self.entry(), &self.cfg, &self.projects, &self.mcps)
        else {
            return;
        };
        let Some(project) = e.projects.get(k) else {
            return;
        };
        let servers = m.spaces.get(i).map_or(&[][..], Vec::as_slice);
        let changes = workspaces::dropping(cfg, p, e, servers, std::slice::from_ref(project));
        let (title, message) = if e.projects.len() == 1 {
            ("unlink", unlink_message(&e.root))
        } else {
            (
                "drop project",
                format!(
                    "Drop {} from {}? Its files then list the other projects.",
                    projects::short(cfg, project),
                    tilde(&e.root)
                ),
            )
        };
        let runs = Self::runs(&changes);
        self.confirm = Some(Confirm::gate(title, message, Action::Changes(changes)).runs(runs));
    }

    /// Add or repoint (`Some(true)`), take back (`Some(false)`) or toggle
    /// (`None`) the workspace's import of convention `c`. Adding brings its
    /// files along, since they are what point its agents at its CANON.md.
    fn space_convention(&mut self, c: usize, fix: Option<bool>) {
        let (Some((_, e)), Some(p)) = (self.entry(), &self.projects) else {
            return;
        };
        let root = tilde(&e.root);
        if !e.here {
            self.set_status(format!("{root} is not on this machine"));
            return;
        }
        let (Some(cell), Some(house)) = (e.own.cells.get(c), p.house.get(c)) else {
            return;
        };
        let label = house_label(house);
        let fix = fix.unwrap_or(cell.state != State::Linked);
        let reach = self.convention_reach(e, c);
        let from = reach.as_ref().and_then(|r| r.note(("reads", "read")));
        if cell.state == State::Missing && reach.as_ref().is_some_and(Reach::covered) && fix {
            let from = from.unwrap_or_default();
            self.set_status(format!("{root} already reads {label}: {from}"));
            return;
        }
        if cell.undo.is_none()
            && !fix
            && let Some(from) = from
        {
            self.set_status(format!("{root} does not import {label} itself: {from}"));
            return;
        }
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            self.set_status(if fix {
                format!("{root} already imports {label}")
            } else {
                format!("{root} does not import {label}")
            });
            return;
        };
        let mut changes = vec![change];
        if fix {
            changes.extend(e.own.wiring_changes());
            changes.extend(e.file_fixes(None));
        }
        let changes = plan::dedup(changes.into_iter());
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
                    format!("{verb} {label} for {root}?"),
                    Action::Changes(changes),
                )
            } else {
                Confirm::gate(
                    &gate_title(&changes),
                    format!("Stop {root} importing {label}?"),
                    Action::Changes(changes),
                )
            }
            .runs(runs),
        );
    }

    /// Add or rewrite (`Some(true)`), take back (`Some(false)`) or toggle
    /// (`None`) MCP server `k` for every agent in the workspace.
    fn space_server(&mut self, k: usize, fix: Option<bool>) {
        let (Some((i, e)), Some(m)) = (self.entry(), &self.mcps) else {
            return;
        };
        let root = tilde(&e.root);
        let (Some(server), Some(cell)) = (m.servers.get(k), m.spaces.get(i).and_then(|r| r.get(k)))
        else {
            self.set_status(format!("{root} is not on this machine"));
            return;
        };
        let name = server.name.clone();
        let fix = fix.unwrap_or(cell.state != State::Linked);
        let reach = self.server_reach(k);
        let from = reach.as_ref().and_then(|r| r.note(("has", "have")));
        if cell.state == State::Missing && reach.as_ref().is_some_and(Reach::covered) && fix {
            let from = from.unwrap_or_default();
            self.set_status(format!("{root} already has {name}: {from}"));
            return;
        }
        if cell.undo.is_none()
            && !fix
            && let Some(from) = from
        {
            self.set_status(format!("{root} does not have {name} itself: {from}"));
            return;
        }
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            let (failed, msg) = match &cell.state {
                State::Foreign(why) => (true, why.clone()),
                State::Na => (false, format!("no agent here can take {name}")),
                _ if fix => (false, format!("{root} already has {name}")),
                _ => (false, format!("{root} does not have {name}")),
            };
            if failed {
                self.set_failed(msg);
            } else {
                self.set_status(msg);
            }
            return;
        };
        let runs = Self::runs(std::slice::from_ref(&change));
        self.confirm = Some(
            if fix {
                Confirm::offer(
                    "add",
                    format!("Add {name} to {root}, for every agent there?"),
                    Action::Changes(vec![change]),
                )
            } else {
                Confirm::gate(
                    &gate_title(std::slice::from_ref(&change)),
                    format!("Delete {name} from {root}?"),
                    Action::Changes(vec![change]),
                )
            }
            .runs(runs),
        );
    }

    /// `a` in the card: the form for one more project.
    fn add_project(&mut self) {
        let Some((_, e)) = self.entry() else { return };
        let (key, skip) = (e.key.clone(), e.projects.clone());
        let names = self.project_names(&skip);
        self.link_form = Some(LinkForm::adding(&key, names));
    }

    /// `o` in the card: the file the line is about, when there is one.
    fn space_open(&mut self) -> Option<Edit> {
        let file = self.space_file_at();
        if file.is_none() {
            self.set_status("nothing to open on this line");
        }
        file.map(Edit)
    }

    /// The file `o` opens on the card's line: the workspace's file, the
    /// project's own, CONTEXT.md (there or about to be), or the CANON.md a
    /// convention import sits in.
    pub(super) fn space_file_at(&self) -> Option<PathBuf> {
        let (_, e) = self.entry()?;
        if !e.here {
            return None;
        }
        let file = match self.space_line()? {
            SpaceLine::File(a) => e.cells[a].at.clone(),
            SpaceLine::Project(k) => {
                let root = e.projects.get(k)?;
                self.projects
                    .as_ref()?
                    .list
                    .iter()
                    .find(|x| x.root == *root)?
                    .host
                    .clone()
            }
            SpaceLine::Context => return Some(e.root.join(CONTEXT_FILE)),
            SpaceLine::Convention(_) => e.root.join(CANON_FILE),
            SpaceLine::Server(_) => return None,
        };
        file.is_file().then_some(file)
    }
}

/// What the gate in front of `canon unlink` says.
fn unlink_message(root: &Path) -> String {
    format!(
        "Unlink {}? canonize deletes what it made there (its CLAUDE.md and AGENTS.md, convention imports and MCP servers) and forgets it, leaving everything else, {CONTEXT_FILE} included, as it is.",
        tilde(root)
    )
}

pub(super) fn file_name(at: &Path) -> String {
    at.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
