//! House files an agent reads in every project, wired into the agent's own setup
//! rather than into a project's CANON.md.
//!
//! Each agent takes them its own way: an `@` line in its rules file when it
//! imports them (Claude), an `@` line in the CANON.md in its own folder that
//! canonize's extension loads (pi), or an entry in `instructions` (opencode).
//! Codex has no way to load another file. Which agent reads which house file is
//! whatever its files say: `fix` repoints a broken one and never adds one.

use crate::config::{self, Agent, Config, RulesMode, tilde};
use crate::plan::{self, CANON_FILE, Cell, Change, State};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Houses {
    /// The canon's house files, as rows.
    pub house: Vec<PathBuf>,
    /// `cells[house][agent]`, agents in config order.
    pub cells: Vec<Vec<Cell>>,
}

impl Houses {
    pub fn build(cfg: &Config) -> Houses {
        let house = config::house_files(&cfg.source);
        let cells = house
            .iter()
            .map(|h| cfg.agents.iter().map(|a| cell(a, h)).collect())
            .collect();
        Houses { house, cells }
    }

    fn of(&self, only: Option<usize>) -> impl Iterator<Item = &Cell> {
        self.cells.iter().flat_map(move |row| {
            row.iter()
                .enumerate()
                .filter(move |(a, _)| only.is_none_or(|o| o == *a))
                .map(|(_, c)| c)
        })
    }

    /// What `fix` does: repoint every broken import, and nothing else.
    pub fn fixes(&self, only: Option<usize>) -> Vec<Change> {
        plan::dedup(
            self.of(only)
                .filter(|c| matches!(c.state, State::Broken(_)))
                .filter_map(|c| c.change.clone()),
        )
    }

    pub fn drifted(&self, only: Option<usize>) -> bool {
        self.of(only).any(|c| matches!(c.state, State::Broken(_)))
    }

    /// Every house import, for `canon delete`.
    pub fn undos(&self, only: Option<usize>) -> Vec<Change> {
        plan::dedup(self.of(only).filter_map(|c| c.undo.clone()))
    }
}

/// How one agent is made to read a house file everywhere.
enum Via {
    /// An `@` line in this file.
    Import(PathBuf),
    /// An `@` line in the CANON.md in pi's folder, which its extension loads.
    Pi {
        canon: PathBuf,
        ext: PathBuf,
        body: String,
    },
    /// An entry in the `instructions` list of this opencode config.
    Json(PathBuf),
}

fn via(agent: &Agent) -> Option<Via> {
    if agent.rules_mode == RulesMode::Import {
        return Some(Via::Import(agent.rules.clone()));
    }
    match agent.name.as_str() {
        "pi" => {
            let (ext, body) = plan::pi_extension(agent);
            Some(Via::Pi {
                canon: agent.home.join(CANON_FILE),
                ext,
                body,
            })
        }
        "opencode" => Some(Via::Json(agent.home.join("opencode.json"))),
        _ => None,
    }
}

fn cell(agent: &Agent, house: &Path) -> Cell {
    if !agent.active() {
        return plan::absent(agent.home.clone(), State::Absent);
    }
    match via(agent) {
        None => plan::absent(agent.home.clone(), State::Na),
        Some(Via::Import(file)) => line_cell(&file, house),
        Some(Via::Pi { canon, ext, body }) => pi_cell(line_cell(&canon, house), &ext, body),
        Some(Via::Json(file)) => json_cell(&file, house),
    }
}

/// `file` should carry an `@` line naming `house`.
fn line_cell(file: &Path, house: &Path) -> Cell {
    let line = format!("@{}", tilde(house));
    let at = file.to_path_buf();
    if plan::is_link(file) {
        return plan::absent(
            at,
            State::Foreign(format!(
                "`{}` is a link, so an import written there would land in the file it points at",
                tilde(file)
            )),
        );
    }
    let text = match plan::read_text(file) {
        Ok(text) => text.unwrap_or_default(),
        Err(_) => return plan::absent(at, State::Foreign(plan::NOT_UTF8.into())),
    };
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    if let Some(found) = lines.iter().find(|l| plan::names_file(l, house)) {
        return Cell {
            state: State::Linked,
            change: None,
            undo: Some(Change::RemoveImport {
                file: at.clone(),
                line: found.to_string(),
            }),
            at,
        };
    }
    // An import of a gone file with this name is this house file before the
    // canon moved.
    let gone = lines.iter().find(|l| {
        l.strip_prefix('@').is_some_and(|p| {
            let p = config::expand(p);
            !p.exists() && p.file_name() == house.file_name()
        })
    });
    match gone {
        Some(old) => Cell {
            state: State::Broken(format!("imports `{}`, which is gone", &old[1..])),
            change: Some(Change::ReplaceImport {
                file: at.clone(),
                old: old.to_string(),
                new: line,
            }),
            undo: Some(Change::RemoveImport {
                file: at.clone(),
                line: old.to_string(),
            }),
            at,
        },
        None => Cell {
            state: State::Missing,
            change: Some(Change::AddImport {
                file: at.clone(),
                line,
            }),
            undo: None,
            at,
        },
    }
}

/// A line in pi's CANON.md only counts once the extension that loads it is
/// the one canonize writes today.
fn pi_cell(mut cell: Cell, ext: &Path, body: String) -> Cell {
    let current = fs::read_to_string(ext);
    if current.as_ref().is_ok_and(|t| *t == body) {
        return cell;
    }
    let write = Change::WriteFile {
        file: ext.to_path_buf(),
        body,
        what: "the pi extension that loads CANON.md".into(),
    };
    let need = match current {
        Err(_) => write,
        // An older version canonize wrote, which reads no CANON.md of pi's own.
        Ok(t) if t.contains("Written by `canon`") => Change::Batch {
            what: format!("update {}, the pi extension canonize wrote", tilde(ext)),
            changes: vec![
                Change::DeleteFile {
                    file: ext.to_path_buf(),
                    what: "the old pi extension".into(),
                },
                write,
            ],
        },
        Ok(_) => {
            if matches!(cell.state, State::Foreign(_)) {
                return cell;
            }
            return plan::absent(
                cell.at,
                State::Foreign(format!(
                    "`{}` is not the extension canonize writes, so it is left alone",
                    tilde(ext)
                )),
            );
        }
    };
    match cell.state {
        State::Linked => {
            cell.state = State::Broken("pi's extension does not load it yet".into());
            cell.change = Some(need);
        }
        State::Missing | State::Broken(_) => {
            // The import first, so dedup by the first change keeps one per
            // house file while the extension is written once and then kept.
            if let Some(line) = cell.change.take() {
                cell.change = Some(Change::Batch {
                    what: format!("{} and install pi's extension", line.describe()),
                    changes: vec![line, need],
                });
            }
        }
        _ => {}
    }
    cell
}

/// `file`'s `instructions` should list `house` by its absolute path.
fn json_cell(file: &Path, house: &Path) -> Cell {
    let at = file.to_path_buf();
    let value = house.to_string_lossy().into_owned();
    let list: Vec<String> = match fs::read_to_string(file) {
        Err(_) => Vec::new(),
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => v
                .get("instructions")
                .and_then(|l| l.as_array())
                .map(|l| {
                    l.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => {
                return plan::absent(
                    at,
                    State::Foreign(format!(
                        "`{}` is not plain JSON, so canonize cannot edit its `instructions`",
                        tilde(file)
                    )),
                );
            }
        },
    };
    if let Some(found) = list
        .iter()
        .find(|v| plan::names_file(&format!("@{v}"), house))
    {
        return Cell {
            state: State::Linked,
            change: None,
            undo: Some(Change::JsonUninstruction {
                file: at.clone(),
                value: found.clone(),
            }),
            at,
        };
    }
    let gone = list.iter().find(|v| {
        let p = config::expand(v);
        p.is_absolute() && !p.exists() && p.file_name() == house.file_name()
    });
    match gone {
        Some(old) => Cell {
            state: State::Broken(format!("lists `{old}`, which is gone")),
            change: Some(Change::Batch {
                what: format!(
                    "replace \"{old}\" with \"{value}\" in `instructions` in {}",
                    tilde(file)
                ),
                changes: vec![
                    Change::JsonUninstruction {
                        file: at.clone(),
                        value: old.clone(),
                    },
                    Change::JsonInstruction {
                        file: at.clone(),
                        value,
                    },
                ],
            }),
            undo: Some(Change::JsonUninstruction {
                file: at.clone(),
                value: old.clone(),
            }),
            at,
        },
        None => Cell {
            state: State::Missing,
            change: Some(Change::JsonInstruction {
                file: at.clone(),
                value,
            }),
            undo: None,
            at,
        },
    }
}
