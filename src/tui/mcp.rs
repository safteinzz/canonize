//! The MCP tab's keys: a column per server in the canon, and the houses tab's
//! rows, every agent (for every project) above every project.

use super::Saved;
use super::confirm::{Action, Confirm};
use super::scope::Scope;
use super::server_form::ServerForm;
use super::typed::Typed;
use super::{App, Edit, HouseRow};
use crate::mcp::{Bearer, Spec};
use crate::plan::{self, Change, State};
use crate::projects;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::Path;

impl App {
    /// A key in the MCP tab: `Some` when the tab took it, with what to edit.
    pub(super) fn mcp_key(&mut self, key: KeyEvent) -> Option<Option<Edit>> {
        use KeyCode::*;
        let rows = self.house_rows();
        let cols = self.mcps.as_ref().map_or(0, |m| m.servers.len());
        match key.code {
            Down | Char('j') => self.mrow = (self.mrow + 1).min(rows.saturating_sub(1)),
            Up | Char('k') => self.mrow = self.mrow.saturating_sub(1),
            Right | Char('l') => self.mcol = (self.mcol + 1).min(cols.saturating_sub(1)),
            Left | Char('h') => self.mcol = self.mcol.saturating_sub(1),
            Char('g') | Home => self.mrow = 0,
            Char('G') | End => self.mrow = rows.saturating_sub(1),
            Char('f') => self.mcp_toggle(Some(true)),
            Enter | Char(' ') => self.mcp_toggle(None),
            Char('n') => self.server_form = Some(ServerForm::new()),
            Char('e') => match self.mcps.as_ref().and_then(|m| m.servers.get(self.mcol)) {
                Some(s) => self.server_form = Some(ServerForm::edit(s)),
                None => self.set_status("no MCP servers yet: n describes one"),
            },
            Char('a') => self.open_mcp_scope(false),
            Char('d') => self.open_mcp_scope(true),
            Char('D') => self.delete_server(),
            Char('o') => {
                let HouseRow::Agent(a) = self.house_row_at(self.mrow)? else {
                    return Some(None);
                };
                let at = &self.mcps.as_ref()?.cells.get(self.mcol)?.get(a)?.at;
                // Claude rewrites its state file under a running session, so
                // it is changed through `claude mcp` and never opened here.
                let ours = *at != self.cfg.as_ref()?.claude_state;
                return Some((ours && at.is_file()).then(|| Edit(at.clone())));
            }
            _ => return None,
        }
        Some(None)
    }

    /// Save what the form describes: the server into the canon, its token
    /// into its kept file, then, for a server that was there already, an offer
    /// to rewrite every entry that has it. A refusal stays in the form.
    pub(super) fn save_server(&mut self, saved: Saved) {
        let (Some(cfg), Some(form)) = (&self.cfg, &mut self.server_form) else {
            return;
        };
        let new = form.old.is_none();
        let mut spec = saved.spec;
        let kept = cfg.tokens.join(&saved.name);
        if let Some(t) = &saved.token {
            if !crate::mcp::is_token(t) {
                form.error = Some(
                    "a token is letters, digits and `-._~+/=`, with no spaces: paste it again"
                        .into(),
                );
                return;
            }
            if let Spec::Http { bearer, .. } = &mut spec {
                *bearer = Some(Bearer::Kept(kept.clone()));
            }
        }
        let wrote =
            crate::mcp::define(&cfg.source.mcp, &saved.name, &spec, new).and_then(
                |()| match &saved.token {
                    Some(t) => crate::mcp::save_token(&kept, t),
                    None => Ok(()),
                },
            );
        if let Err(e) = wrote {
            form.error = Some(format!("{e:#}"));
            return;
        }
        let name = saved.name;
        self.server_form = None;
        self.reload();
        let Some(m) = &self.mcps else { return };
        let Some(i) = m.servers.iter().position(|s| s.name == name) else {
            return;
        };
        self.mcol = i;
        let changes = plan::dedup(
            m.cells[i]
                .iter()
                .filter(|c| matches!(c.state, State::Broken(_)))
                .filter_map(|c| c.change.clone())
                .chain(
                    m.projects
                        .iter()
                        .map(|p| &p[i])
                        .filter(|c| matches!(c.state, State::Broken(_)))
                        .filter_map(|c| c.change.clone()),
                ),
        );
        if new {
            self.set_status(format!("{name} is in your canon: ↵ on a row adds it there"));
        } else if changes.is_empty() {
            self.set_status(format!("{name} saved"));
        } else {
            let runs = Self::runs(&changes);
            let n = changes.len();
            self.confirm = Some(
                Confirm::offer(
                    "rewrite",
                    format!(
                        "{name} is saved. Rewrite it where agents have it ({n} change{})?",
                        if n == 1 { "" } else { "s" }
                    ),
                    Action::Changes(changes),
                )
                .runs(runs),
            );
        }
    }

    /// `D`: the server under the cursor out of every agent and project, out
    /// of mcp.toml, and its kept token deleted, behind its typed name, since
    /// the token exists nowhere else.
    fn delete_server(&mut self) {
        let (Some(m), Some(cfg)) = (&self.mcps, &self.cfg) else {
            return;
        };
        let Some(server) = m.servers.get(self.mcol) else {
            self.set_status("no MCP servers yet: n describes one");
            return;
        };
        let i = self.mcol;
        let mut changes = plan::dedup(
            m.cells[i]
                .iter()
                .filter_map(|c| c.undo.clone())
                .chain(m.projects.iter().filter_map(|p| p[i].undo.clone())),
        );
        changes.push(Change::McpUndefine {
            file: cfg.source.mcp.clone(),
            name: server.name.clone(),
            token: server.kept().map(Path::to_path_buf),
        });
        // One batch, which stops at the first failure: the canon's copy and
        // its token go only once no agent still names them.
        let changes = vec![Change::Batch {
            what: format!(
                "delete MCP server `{}` everywhere, then from your canon",
                server.name
            ),
            changes,
        }];
        let name = server.name.clone();
        let token = if server.kept().is_some_and(Path::is_file) {
            " Its kept token is deleted too, and it exists nowhere else."
        } else {
            ""
        };
        self.typed = Some(Typed {
            title: "delete server".into(),
            message: format!(
                "Delete {name} from your canon? It comes out of every agent and project that has it, and out of mcp.toml.{token}"
            ),
            name,
            input: String::new(),
            back: 0,
            changes,
        });
    }

    /// The server under the cursor, by name.
    fn mcp_server(&self) -> Option<String> {
        Some(self.mcps.as_ref()?.servers.get(self.mcol)?.name.clone())
    }

    /// Enter (`None`, a toggle) or `f` (`Some(true)`) in the MCP tab.
    fn mcp_toggle(&mut self, fix: Option<bool>) {
        let Some(m) = &self.mcps else { return };
        let state = match self.house_row_at(self.mrow) {
            Some(HouseRow::Agent(a)) => m
                .cells
                .get(self.mcol)
                .and_then(|r| r.get(a))
                .map(|c| &c.state),
            Some(HouseRow::Project(i)) => m
                .projects
                .get(i)
                .and_then(|r| r.get(self.mcol))
                .map(|c| &c.state),
            None => None,
        };
        let added = state.is_some_and(|s| *s == State::Linked);
        let fix = fix.unwrap_or(!added);
        match self.house_row_at(self.mrow) {
            Some(HouseRow::Agent(a)) => self.mcp_agent_cell(a, fix),
            Some(HouseRow::Project(i)) => self.mcp_project_cell(i, fix),
            None => {}
        }
    }

    /// Add or rewrite (`fix`), or take out (`!fix`), the selected server for
    /// agent `a`, in every project.
    fn mcp_agent_cell(&mut self, a: usize, fix: bool) {
        let (Some(m), Some(cfg)) = (&self.mcps, &self.cfg) else {
            return;
        };
        let (Some(server), Some(cell)) = (
            m.servers.get(self.mcol),
            m.cells.get(self.mcol).and_then(|r| r.get(a)),
        ) else {
            return;
        };
        let agent = &cfg.agents[a].name;
        let name = &server.name;
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            self.set_status(match (&cell.state, fix) {
                (State::Linked, true) => format!("{agent} already has {name}"),
                (_, false) => format!("{agent} does not have {name}"),
                (s, true) => match s.why() {
                    Some(why) => format!("{agent}: {why}"),
                    None => format!("{agent} has no MCP config canonize knows"),
                },
            });
            return;
        };
        let runs = Self::runs(std::slice::from_ref(&change));
        self.confirm = Some(
            if fix {
                let verb = if matches!(cell.state, State::Broken(_)) {
                    "Rewrite"
                } else {
                    "Add"
                };
                Confirm::offer(
                    &verb.to_lowercase(),
                    format!("{verb} {name} for {agent}, in every project?"),
                    Action::Changes(vec![change]),
                )
            } else {
                Confirm::gate(
                    "delete",
                    format!("Take {name} out of {agent}'s own config?"),
                    Action::Changes(vec![change]),
                )
            }
            .runs(runs),
        );
    }

    /// Add or complete (`fix`), or take out (`!fix`), the selected server for
    /// every agent in project `i`.
    fn mcp_project_cell(&mut self, i: usize, fix: bool) {
        let (Some(m), Some(cfg), Some(p)) = (&self.mcps, &self.cfg, &self.projects) else {
            return;
        };
        let (Some(server), Some(cell), Some(x)) = (
            m.servers.get(self.mcol),
            m.projects.get(i).and_then(|r| r.get(self.mcol)),
            p.list.get(i),
        ) else {
            return;
        };
        let project = projects::short(cfg, &x.root);
        let name = &server.name;
        let change = if fix { &cell.change } else { &cell.undo };
        let Some(change) = change.clone() else {
            self.set_status(match (&cell.state, fix) {
                (State::Linked, true) => format!("{project} already has {name} for every agent"),
                (_, false) => format!("{project} does not have {name}"),
                (s, true) => match s.why() {
                    Some(why) => format!("{project}: {why}"),
                    None => format!("no agent here can take {name}"),
                },
            });
            return;
        };
        let runs = Self::runs(std::slice::from_ref(&change));
        self.confirm = Some(
            if fix {
                let verb = if matches!(cell.state, State::Broken(_)) {
                    "Complete"
                } else {
                    "Add"
                };
                Confirm::offer(
                    &verb.to_lowercase(),
                    format!("{verb} {name} in {project}, for every agent?"),
                    Action::Changes(vec![change]),
                )
            } else {
                Confirm::gate(
                    "delete",
                    format!("Take {name} out of {project}?"),
                    Action::Changes(vec![change]),
                )
            }
            .runs(runs),
        );
    }

    /// The MCP tab's `a` and `d`: this cell, every server in this row, or
    /// this server in every agent (an agent's row) or every project.
    fn open_mcp_scope(&mut self, remove: bool) {
        let (Some(m), Some(cfg), Some(p)) = (&self.mcps, &self.cfg, &self.projects) else {
            return;
        };
        let Some(name) = self.mcp_server() else {
            self.set_status("no MCP servers yet: add one to your canon's mcp.toml");
            return;
        };
        let pick_agent = |c: &plan::Cell| {
            if remove {
                c.undo.clone()
            } else if c.state == State::Linked {
                None
            } else {
                c.change.clone()
            }
        };
        let pick_project = |c: &projects::Cell| {
            if remove {
                c.undo.clone()
            } else if c.state == State::Linked {
                None
            } else {
                c.change.clone()
            }
        };
        let col = self.mcol;
        let to = if remove { "from" } else { "to" };
        let mut items: Vec<(String, Vec<Change>)> = match self.house_row_at(self.mrow) {
            Some(HouseRow::Agent(a)) => {
                let agent = &cfg.agents[a].name;
                vec![
                    (
                        format!("{name} {to} {agent}, in every project"),
                        plan::dedup(pick_agent(&m.cells[col][a]).into_iter()),
                    ),
                    (
                        format!("every server {to} {agent}"),
                        plan::dedup(m.cells.iter().filter_map(|r| pick_agent(&r[a]))),
                    ),
                    (
                        format!("{name} {to} every agent"),
                        plan::dedup(m.cells[col].iter().filter_map(pick_agent)),
                    ),
                ]
            }
            Some(HouseRow::Project(i)) => {
                let project = projects::short(cfg, &p.list[i].root);
                vec![
                    (
                        format!("{name} {to} {project}"),
                        plan::dedup(pick_project(&m.projects[i][col]).into_iter()),
                    ),
                    (
                        format!("every server {to} {project}"),
                        plan::dedup(m.projects[i].iter().filter_map(pick_project)),
                    ),
                    (
                        format!("{name} {to} every project"),
                        plan::dedup(m.projects.iter().filter_map(|r| pick_project(&r[col]))),
                    ),
                ]
            }
            None => return,
        };
        items.retain(|(_, changes)| !changes.is_empty());
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
}
