//! The projects tab's keys: every folder under the scopes, and every project
//! wherever it is, one row each, where Enter makes a folder a project or
//! forgets one the user added.

use super::confirm::{Action, Confirm};
use super::project_form::ProjectForm;
use super::workspaces::file_name;
use super::{App, Edit};
use crate::config::tilde;
use crate::plan::Change;
use crate::projects;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::{Path, PathBuf};

/// One row of the projects tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjRow {
    /// A project, by index into `Projects::list`.
    Project(usize),
    /// An added project whose folder is not here, by index into `Projects::gone`.
    Gone(usize),
    /// A folder under a scope that is not a project, by index into
    /// `Projects::candidates`.
    Folder(usize),
}

impl App {
    /// A key in the projects tab: `Some` when the tab took it.
    pub(super) fn project_key(&mut self, key: KeyEvent) -> Option<Option<Edit>> {
        use KeyCode::*;
        let rows = self.proj_rows().len();
        if let Some(to) = self.motion(key, self.jrow, rows) {
            self.jrow = to;
            return Some(None);
        }
        match key.code {
            Enter | Char(' ') => self.project_toggle(),
            Char('a') => self.open_project_form(),
            Char('d') => self.project_forget(),
            Char('o') => return Some(self.project_file().map(Edit)),
            _ => return None,
        }
        Some(None)
    }

    /// Every row before the filter, grouped by scope (the ones added from
    /// elsewhere last) and by path inside a group, so a folder keeps its
    /// place when it becomes a project.
    pub(super) fn all_proj_rows(&self) -> Vec<ProjRow> {
        let Some(p) = &self.projects else {
            return Vec::new();
        };
        let mut rows: Vec<(&Path, ProjRow)> = p
            .list
            .iter()
            .enumerate()
            .map(|(i, x)| (x.root.as_path(), ProjRow::Project(i)))
            .chain(
                p.gone
                    .iter()
                    .enumerate()
                    .map(|(i, g)| (g.as_path(), ProjRow::Gone(i))),
            )
            .chain(
                p.candidates
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (d.as_path(), ProjRow::Folder(i))),
            )
            .collect();
        rows.sort_by(|a, b| (self.group_of(a.0), a.0).cmp(&(self.group_of(b.0), b.0)));
        rows.into_iter().map(|(_, r)| r).collect()
    }

    /// The group a row sits in: the index of its scope in config order, or
    /// one past the last for a project added from elsewhere.
    pub(super) fn proj_group(&self, row: ProjRow) -> usize {
        self.proj_path(row).map_or(0, |p| self.group_of(p))
    }

    fn group_of(&self, path: &Path) -> usize {
        let Some(cfg) = &self.cfg else { return 0 };
        cfg.scopes
            .iter()
            .position(|s| path.strip_prefix(s).is_ok())
            .unwrap_or(cfg.scopes.len())
    }

    /// The rows the tab shows once filtered; `jrow` indexes this.
    pub(super) fn proj_rows(&self) -> Vec<ProjRow> {
        self.kept(self.all_proj_rows(), |r| self.proj_name(r))
    }

    pub(super) fn proj_row(&self) -> Option<ProjRow> {
        self.proj_rows().get(self.jrow).copied()
    }

    /// The folder a row is about.
    pub(super) fn proj_path(&self, row: ProjRow) -> Option<&Path> {
        let p = self.projects.as_ref()?;
        Some(match row {
            ProjRow::Project(i) => p.list.get(i)?.root.as_path(),
            ProjRow::Gone(i) => p.gone.get(i)?.as_path(),
            ProjRow::Folder(i) => p.candidates.get(i)?.as_path(),
        })
    }

    /// The name a row goes by, which is what `/` matches: as `canon status`
    /// names a project, and a folder under its scope.
    pub(super) fn proj_name(&self, row: ProjRow) -> String {
        match (&self.cfg, self.proj_path(row)) {
            (Some(cfg), Some(path)) => projects::short(cfg, path),
            _ => String::new(),
        }
    }

    /// Enter: offer to make the folder a project, or gate forgetting one the
    /// user added; a project found by its own file stays one.
    fn project_toggle(&mut self) {
        match self.proj_row() {
            Some(ProjRow::Folder(_)) => {
                if let Some(path) = self.proj_path_owned() {
                    self.offer_project(&tilde(&path));
                }
            }
            Some(ProjRow::Project(_) | ProjRow::Gone(_)) => self.project_forget(),
            None => self.set_status(NOTHING),
        }
    }

    /// `d`: forget a project the user added, behind the red gate.
    fn project_forget(&mut self) {
        let (Some(cfg), Some(p), Some(row)) = (&self.cfg, &self.projects, self.proj_row()) else {
            self.set_status(NOTHING);
            return;
        };
        let path = match row {
            ProjRow::Folder(_) => {
                self.set_status("not a project: ↵ adds it");
                return;
            }
            ProjRow::Gone(i) => p.gone[i].clone(),
            ProjRow::Project(i) if p.list[i].listed => p.list[i].root.clone(),
            ProjRow::Project(i) => {
                let marker = file_name(&p.list[i].host);
                self.set_status(format!(
                    "it has its own {marker}, so it is a project while that file is there"
                ));
                return;
            }
        };
        let change = Change::Project {
            config: cfg.path.clone(),
            path: tilde(&path),
            keep: false,
        };
        let message = format!(
            "Forget {}? Nothing in it changes: its CANON.md, CLAUDE.local.md and MCP entries stay, and ↵ adds it again.",
            tilde(&path)
        );
        let runs = Self::runs(std::slice::from_ref(&change));
        self.confirm = Some(
            Confirm::gate("forget project", message, Action::Changes(vec![change])).runs(runs),
        );
    }

    /// The footer's word for Enter on the row under the cursor: it adds a
    /// folder and forgets an added project, and a found one has nothing.
    pub(super) fn proj_enter(&self) -> Option<&'static str> {
        let p = self.projects.as_ref()?;
        match self.proj_row()? {
            ProjRow::Folder(_) => Some("↵ add"),
            ProjRow::Gone(_) => Some("↵ forget"),
            ProjRow::Project(i) => p.list.get(i)?.listed.then_some("↵ forget"),
        }
    }

    fn proj_path_owned(&self) -> Option<PathBuf> {
        self.proj_path(self.proj_row()?).map(Path::to_path_buf)
    }

    /// `o`: the project's own instructions file, when it has one.
    pub(super) fn project_file(&self) -> Option<PathBuf> {
        match self.proj_row()? {
            ProjRow::Project(i) => {
                let host = &self.projects.as_ref()?.list.get(i)?.host;
                host.is_file().then(|| host.clone())
            }
            _ => None,
        }
    }

    /// The tab's `a`: the form for a folder anywhere, its picker offering
    /// what the scopes hold that is not a project yet.
    pub(super) fn open_project_form(&mut self) {
        let (Some(cfg), Some(p)) = (&self.cfg, &self.projects) else {
            return;
        };
        let choices = p
            .candidates
            .iter()
            .map(|dir| projects::short(cfg, dir))
            .collect();
        self.project_form = Some(ProjectForm::new(choices));
    }

    /// The form's Enter: offer to make `folder` a project, or say in the form
    /// why it cannot be one.
    pub(super) fn save_project(&mut self, folder: &str) {
        let typed = self.under_scope(folder);
        let refused = match (&self.cfg, &self.projects) {
            (Some(cfg), Some(p)) => projects::adding(cfg, p, &typed).err(),
            _ => return,
        };
        if let Some(e) = refused {
            if let Some(form) = &mut self.project_form {
                form.error = Some(crate::said(&e));
            }
            return;
        }
        self.project_form = None;
        self.offer_project(&typed);
    }

    /// A folder as the user typed or picked it: a path from `~` or `/` as it
    /// is, else the folder of that name under a scope.
    pub(super) fn under_scope(&self, folder: &str) -> String {
        if folder.starts_with('~') || folder.starts_with('/') {
            return folder.to_string();
        }
        self.cfg
            .as_ref()
            .and_then(|cfg| {
                cfg.scopes
                    .iter()
                    .map(|s| s.join(folder))
                    .find(|at| at.is_dir())
            })
            .map_or_else(|| folder.to_string(), |at| tilde(&at))
    }

    /// The cyan offer making the folder `typed` a project.
    fn offer_project(&mut self, typed: &str) {
        let (Some(cfg), Some(p)) = (&self.cfg, &self.projects) else {
            return;
        };
        let change = match projects::adding(cfg, p, typed) {
            Ok(c) => c,
            Err(e) => {
                self.report_failure("add project", crate::said(&e));
                return;
            }
        };
        let at = crate::config::expand(typed);
        let root = std::fs::canonicalize(&at).unwrap_or(at);
        let message = format!(
            "Add {} as a project? canonize writes nothing in it: it is listed in canonize.toml, gets a row in Conventions and MCPs, and a workspace can be linked to it.",
            tilde(&root)
        );
        self.added = Some(root);
        let runs = Self::runs(std::slice::from_ref(&change));
        self.confirm =
            Some(Confirm::offer("add project", message, Action::Changes(vec![change])).runs(runs));
    }

    /// Put the projects tab's cursor on the folder at `root`.
    pub(super) fn select_project(&mut self, root: &Path) {
        let rows = self.proj_rows();
        if let Some(r) = rows.iter().position(|&r| {
            self.proj_path(r)
                .is_some_and(|p| crate::workspaces::same(p, root))
        }) {
            self.jrow = r;
        }
    }
}

const NOTHING: &str = "no folder here: a adds one, or set `scopes` in canonize.toml (E)";
