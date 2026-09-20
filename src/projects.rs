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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmp::{self, Temp};

    /// A canon with one house file, and `dev/` as the folder to look in.
    fn world() -> (Temp, Config, PathBuf) {
        let t = Temp::new();
        t.write("canon/rules.yaml", "title: MYRULES\n");
        let house = t.write("canon/house/HOUSE-RUST.md", "# HOUSE-RUST\n");
        t.dir("dev");
        let cfg = tmp::config(tmp::source(&t.at("canon")), Vec::new(), vec![t.at("dev")]);
        (t, cfg, house)
    }

    fn only(cfg: &Config) -> Project {
        let mut built = Projects::build(cfg);
        assert_eq!(built.list.len(), 1, "one project was expected");
        built.list.remove(0)
    }

    fn apply(changes: Vec<Change>) {
        for change in changes {
            change.run().expect("a change the plan offered should run");
        }
    }

    #[test]
    fn a_project_is_the_first_folder_with_an_instruction_file_within_three_levels() {
        let (t, cfg, _) = world();
        t.write("dev/app/CLAUDE.md", "");
        t.write("dev/crates/tools/cli/AGENTS.md", "");
        t.write("dev/app/sub/CLAUDE.md", "");
        t.write("dev/a/b/c/d/CLAUDE.md", "");

        let roots: Vec<PathBuf> = Projects::build(&cfg)
            .list
            .iter()
            .map(|p| p.root.clone())
            .collect();
        assert_eq!(roots, [t.at("dev/app"), t.at("dev/crates/tools/cli")]);
    }

    #[test]
    fn hidden_and_build_folders_are_never_walked_into() {
        let (t, cfg, _) = world();
        t.write("dev/.cache/app/CLAUDE.md", "");
        t.write("dev/mono/target/old/CLAUDE.md", "");
        t.write("dev/mono/node_modules/dep/CLAUDE.md", "");
        t.write("dev/app/CLAUDE.md", "");

        let roots: Vec<PathBuf> = Projects::build(&cfg)
            .list
            .iter()
            .map(|p| p.root.clone())
            .collect();
        assert_eq!(roots, [t.at("dev/app")]);
    }

    #[test]
    fn a_house_import_sitting_in_claude_md_moves_into_canon_md() {
        let (t, cfg, house) = world();
        t.write(
            "dev/app/CLAUDE.md",
            &format!("# app\n\n@{}\n\nmy own line\n", house.display()),
        );

        let project = only(&cfg);
        assert_eq!(project.cells[0].state.word(), "broken");
        apply(
            project
                .cells
                .iter()
                .filter_map(|c| c.change.clone())
                .collect(),
        );
        apply(project.wiring_changes());

        let canon =
            fs::read_to_string(t.at("dev/app/CANON.md")).expect("CANON.md should be written");
        assert!(canon.contains(&format!("@{}", house.display())), "{canon}");
        let claude =
            fs::read_to_string(t.at("dev/app/CLAUDE.md")).expect("CLAUDE.md should be there");
        assert!(
            !claude.contains("HOUSE-RUST.md"),
            "the house import moved out: {claude}"
        );
        assert!(
            claude.contains("@CANON.md"),
            "Claude is pointed at CANON.md: {claude}"
        );
        assert!(claude.contains("# app") && claude.contains("my own line"));

        let project = only(&cfg);
        assert_eq!(project.cells[0].state.word(), "linked");
        assert!(!Projects::build(&cfg).drifted());
    }

    #[test]
    fn the_same_house_file_imported_in_two_files_leaves_no_copy_behind() {
        let (t, cfg, house) = world();
        let line = format!("@{}\n", house.display());
        t.write("dev/app/CLAUDE.md", &format!("# app\n{line}"));
        t.write("dev/app/AGENTS.md", &format!("# app\n{line}"));

        let project = only(&cfg);
        apply(
            project
                .cells
                .iter()
                .filter_map(|c| c.change.clone())
                .collect(),
        );

        for file in ["dev/app/CLAUDE.md", "dev/app/AGENTS.md"] {
            let text = fs::read_to_string(t.at(file)).expect("the file should be there");
            assert!(
                !text.contains("HOUSE-RUST.md"),
                "{file} still imports it: {text}"
            );
        }
        assert_eq!(only(&cfg).cells[0].state.word(), "linked");
    }

    #[test]
    fn an_import_added_and_then_taken_back_leaves_no_canon_md() {
        let (t, cfg, _) = world();
        t.write("dev/app/CLAUDE.md", "# app\n");

        let project = only(&cfg);
        assert_eq!(project.cells[0].state.word(), "unwired");
        apply(
            project
                .cells
                .iter()
                .filter_map(|c| c.change.clone())
                .collect(),
        );
        assert!(t.at("dev/app/CANON.md").is_file());

        apply(only(&cfg).undos());
        assert!(
            !t.at("dev/app/CANON.md").exists(),
            "an empty CANON.md is canonize's own leftover"
        );
    }

    #[test]
    fn the_line_that_loads_canon_md_is_not_a_dead_import() {
        let (t, cfg, _) = world();
        t.write("dev/app/CLAUDE.md", "# app\n\n@CANON.md\n");

        let project = only(&cfg);
        assert!(
            project.dead.is_empty(),
            "CANON.md is written by the first import, not missing: {:?}",
            project.dead
        );
    }

    #[test]
    fn an_import_naming_a_file_that_exists_nowhere_is_listed() {
        let (t, cfg, _) = world();
        let gone = t.at("nowhere/gone.md");
        t.write(
            "dev/app/CLAUDE.md",
            &format!("# app\n\n@{}\n", gone.display()),
        );

        assert_eq!(only(&cfg).dead, [format!("@{}", gone.display())]);
    }
}
