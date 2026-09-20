//! Projects: the repos under the user's project folders, and which house files
//! each one imports from the canon.
//!
//! A project is a folder with a `CLAUDE.md` or `AGENTS.md`. Its house imports
//! live in its own `CANON.md`, gitignored and written only by canonize, which
//! each agent loads its own way (Claude through `@CANON.md` in the project's
//! CLAUDE.md). House imports still sitting in CLAUDE.md or AGENTS.md are moved
//! into it by a fix. canonize never touches any other line.

use crate::config::{self, Config, tilde};
use crate::plan::{CANON_FILE, Change, State};
use std::fs;
use std::path::{Path, PathBuf};

/// How deep under a project folder a project can sit (`~/dev/crates/x` is 2).
const DEPTH: usize = 3;

/// Folders never worth walking into.
const SKIP: [&str; 4] = ["target", "node_modules", "dist", "build"];

/// The instruction files a project can carry, most specific first: canonize
/// writes new imports into the first one that exists.
const FILES: [&str; 2] = ["CLAUDE.md", "AGENTS.md"];

pub struct Project {
    pub root: PathBuf,
    /// The project's own instruction file, for `o`: CLAUDE.md, else AGENTS.md.
    pub host: PathBuf,
    /// What makes the agents read CANON.md here.
    pub wiring: Vec<Wire>,
    /// `cells[house]`, house files in canon order.
    pub cells: Vec<Cell>,
    /// Imports of files that exist nowhere and match no house file.
    pub dead: Vec<String>,
}

pub struct Wire {
    pub what: String,
    pub state: State,
    pub change: Option<Change>,
    pub undo: Option<Change>,
}

pub struct Cell {
    pub state: State,
    /// Adds the import when it is missing, or fixes it when broken.
    pub change: Option<Change>,
    /// Removes it when it is there.
    pub undo: Option<Change>,
}

pub struct Projects {
    /// The canon's house files, as columns.
    pub house: Vec<PathBuf>,
    pub list: Vec<Project>,
}

impl Projects {
    pub fn build(cfg: &Config) -> Projects {
        let house = config::house_files(&cfg.source);
        let mut roots = Vec::new();
        for dir in &cfg.projects {
            walk(dir, 0, &mut roots);
        }
        roots.sort();
        roots.dedup();
        let list = roots.iter().map(|r| project(r, &house)).collect();
        Projects { house, list }
    }

    /// What `fix` does on its own: repoint or move broken imports, and wire
    /// CANON.md in every project that imports a house file.
    pub fn fixes(&self) -> Vec<Change> {
        let mut out = Vec::new();
        for p in &self.list {
            out.extend(
                p.cells
                    .iter()
                    .filter(|c| matches!(c.state, State::Broken(_)))
                    .filter_map(|c| c.change.clone()),
            );
            if p.uses_canon() {
                out.extend(p.wiring_changes());
            }
        }
        out
    }

    pub fn drifted(&self) -> bool {
        self.list.iter().any(|p| {
            p.cells.iter().any(|c| matches!(c.state, State::Broken(_)))
                || (p.uses_canon() && !p.wiring_changes().is_empty())
        })
    }
}

impl Project {
    /// Whether it imports any house file, wherever the import sits.
    pub fn uses_canon(&self) -> bool {
        self.cells
            .iter()
            .any(|c| matches!(c.state, State::Linked | State::Broken(_)))
    }

    /// The wiring still missing, which any import added here brings along.
    pub fn wiring_changes(&self) -> Vec<Change> {
        self.wiring
            .iter()
            .filter_map(|w| w.change.clone())
            .collect()
    }

    /// Everything canonize made here, for `canon remove`.
    pub fn undos(&self) -> Vec<Change> {
        self.cells
            .iter()
            .filter_map(|c| c.undo.clone())
            .chain(self.wiring.iter().filter_map(|w| w.undo.clone()))
            .collect()
    }
}

/// A short name for a project: its path under the project folder it was found in.
pub fn short(cfg: &Config, root: &Path) -> String {
    cfg.projects
        .iter()
        .find_map(|d| root.strip_prefix(d).ok())
        .map(|p| p.display().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| tilde(root))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if FILES.iter().any(|f| dir.join(f).is_file()) {
        out.push(dir.to_path_buf());
        return;
    }
    if depth >= DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        // A link is skipped so a folder linked into itself cannot loop.
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir || name.starts_with('.') || SKIP.contains(&name.as_str()) {
            continue;
        }
        walk(&path, depth + 1, out);
    }
}

/// Every `@path` line across a project's instruction files, with the file it
/// sits in and the line as written.
fn imports(root: &Path) -> Vec<(PathBuf, String, PathBuf)> {
    let mut out = Vec::new();
    for f in FILES {
        let file = root.join(f);
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines().map(str::trim) {
            let Some(p) = line.strip_prefix('@') else {
                continue;
            };
            if p.is_empty() || p.contains(' ') {
                continue;
            }
            let target = if p.starts_with('~') || p.starts_with('/') {
                config::expand(p)
            } else {
                root.join(p)
            };
            out.push((file.clone(), line.to_string(), target));
        }
    }
    out
}

fn project(root: &Path, house: &[PathBuf]) -> Project {
    let host = FILES
        .iter()
        .map(|f| root.join(f))
        .find(|p| p.is_file())
        .unwrap_or_else(|| root.join(FILES[0]));
    let canon = root.join(CANON_FILE);
    let in_canon = imports_in(&canon, root);
    let legacy = imports(root);
    let name = |p: &Path| p.file_name().map(|n| n.to_os_string());
    // The same house file: this path, or a gone file of the same name (the
    // canon moved since the line was written).
    let matches = |t: &PathBuf, h: &PathBuf| t == h || (name(t) == name(h) && !t.exists());

    let cells = house
        .iter()
        .map(|h| {
            let line = format!("@{}", tilde(h));
            if let Some((_, old, t)) = in_canon.iter().find(|(_, _, t)| matches(t, h)) {
                let undo = Some(Change::RemoveImport {
                    file: canon.clone(),
                    line: old.clone(),
                });
                return if t == h {
                    Cell {
                        state: State::Linked,
                        change: None,
                        undo,
                    }
                } else {
                    Cell {
                        state: State::Broken(format!("imports `{}`, which is gone", &old[1..])),
                        change: Some(Change::ReplaceImport {
                            file: canon.clone(),
                            old: old.clone(),
                            new: line,
                        }),
                        undo,
                    }
                };
            }
            let copies: Vec<&(PathBuf, String, PathBuf)> =
                legacy.iter().filter(|(_, _, t)| matches(t, h)).collect();
            if let Some((file, old, _)) = copies.first() {
                let from = file
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                let mut changes = vec![Change::MoveImport {
                    from: file.clone(),
                    old: old.clone(),
                    to: canon.clone(),
                    new: line,
                }];
                // The same house file imported in CLAUDE.md and AGENTS.md: the
                // copies go too, or the project keeps a dead import.
                changes.extend(copies.iter().skip(1).map(|(f, o, _)| Change::RemoveImport {
                    file: f.clone(),
                    line: o.clone(),
                }));
                let more = copies.len() - 1;
                let undo = Change::RemoveImport {
                    file: file.clone(),
                    line: old.clone(),
                };
                return Cell {
                    state: State::Broken(if more > 0 {
                        format!("imported in {from} and {more} more, moves to {CANON_FILE}")
                    } else {
                        format!("imported in {from}, moves to {CANON_FILE}")
                    }),
                    change: Some(if changes.len() == 1 {
                        changes.remove(0)
                    } else {
                        Change::Batch {
                            what: format!("move it into {CANON_FILE} and drop its other copies"),
                            changes,
                        }
                    }),
                    undo: Some(undo),
                };
            }
            Cell {
                state: State::Missing,
                change: Some(Change::AddImport {
                    file: canon.clone(),
                    line,
                }),
                undo: None,
            }
        })
        .collect();

    let dead = in_canon
        .iter()
        .chain(&legacy)
        // canonize's own `@CANON.md` line names a file the first import
        // creates, so it is not a dead import.
        .filter(|(_, _, t)| *t != canon)
        .filter(|(_, _, t)| !t.exists() && !house.iter().any(|h| name(h) == name(t)))
        .map(|(_, l, _)| l.clone())
        .collect();

    Project {
        root: root.to_path_buf(),
        host,
        wiring: wiring(root),
        cells,
        dead,
    }
}

/// The `@` lines of one file, resolved against the project folder.
fn imports_in(file: &Path, root: &Path) -> Vec<(PathBuf, String, PathBuf)> {
    let Ok(text) = fs::read_to_string(file) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter_map(|line| {
            let p = line.strip_prefix('@')?;
            if p.is_empty() || p.contains(' ') {
                return None;
            }
            let target = if p.starts_with('~') || p.starts_with('/') {
                config::expand(p)
            } else {
                root.join(p)
            };
            Some((file.to_path_buf(), line.to_string(), target))
        })
        .collect()
}

/// What makes the agents read this project's CANON.md: kept out of git, and
/// named in the project's CLAUDE.md for Claude. pi and opencode load it on
/// their own once the agents tab has wired them.
fn wiring(root: &Path) -> Vec<Wire> {
    let mut out = Vec::new();
    let what = format!("{CANON_FILE} kept out of git");
    let ignore = root.join(".gitignore");
    if root.join(".git").exists() {
        let ignored = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["check-ignore", "-q", CANON_FILE])
            .status()
            .is_ok_and(|s| s.success());
        // Only a line canonize could have written itself is taken back.
        let ours =
            fs::read_to_string(&ignore).is_ok_and(|t| t.lines().any(|l| l.trim() == CANON_FILE));
        out.push(Wire {
            what,
            state: if ignored {
                State::Linked
            } else {
                State::Missing
            },
            change: (!ignored).then(|| Change::AppendLine {
                file: ignore.clone(),
                line: CANON_FILE.to_string(),
            }),
            undo: ours.then(|| Change::RemoveLine {
                file: ignore,
                line: CANON_FILE.to_string(),
            }),
        });
    } else {
        out.push(Wire {
            what,
            state: State::Na,
            change: None,
            undo: None,
        });
    }
    let claude = root.join("CLAUDE.md");
    let line = format!("@{CANON_FILE}");
    let what = format!("`{line}` in CLAUDE.md");
    match fs::read_to_string(&claude) {
        Ok(t) if t.lines().any(|l| l.trim() == line) => out.push(Wire {
            what,
            state: State::Linked,
            change: None,
            undo: Some(Change::RemoveImport { file: claude, line }),
        }),
        Ok(_) => out.push(Wire {
            what,
            state: State::Missing,
            change: Some(Change::AddImport { file: claude, line }),
            undo: None,
        }),
        Err(_) => out.push(Wire {
            what,
            state: State::Na,
            change: None,
            undo: None,
        }),
    }
    out
}

/// Folders that usually hold projects, for setup to suggest.
pub fn guess() -> Vec<String> {
    [
        "~/dev",
        "~/code",
        "~/projects",
        "~/src",
        "~/repos",
        "~/workspace",
        "~/git",
    ]
    .into_iter()
    .filter(|d| config::expand(d).is_dir())
    .map(str::to_string)
    .collect()
}
