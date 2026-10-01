//! Projects: the repos under the user's project folders, and which house files
//! each one imports from the canon.
//!
//! A project is a folder with a `CLAUDE.md` or `AGENTS.md`. Its house imports
//! live in its own `CANON.md`, gitignored and written only by canonize, which
//! each agent loads its own way (Claude through `@CANON.md` in a gitignored
//! `CLAUDE.local.md`). House imports still sitting in CLAUDE.md or AGENTS.md,
//! and a `@CANON.md` line in CLAUDE.md, are moved by a fix. canonize never
//! touches any other line.

use crate::config::{self, Config, tilde};
use crate::plan::{CANON_FILE, Change, LOCAL_FILE, LOCAL_HEADER, State};
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
    /// Whether Claude may load the house files, which sit outside the project;
    /// `None` when the project imports none or Claude is not in use.
    pub claude: Option<Approval>,
    /// The folder Claude Code files its answer under: the git repo's root,
    /// which is not the project's when the project sits inside a repo.
    pub claude_key: PathBuf,
}

/// Claude Code loads a file outside the project only once the user has said
/// yes to it there, and it asks once per project.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Approved,
    /// Nobody has opened Claude there since the imports went in.
    NotAsked,
    /// The user said no, and Claude never asks again.
    Declined,
}

impl Approval {
    /// The name `--json` prints, which never changes.
    pub fn id(self) -> &'static str {
        match self {
            Approval::Approved => "approved",
            Approval::NotAsked => "not_asked",
            Approval::Declined => "declined",
        }
    }

    /// What the user does about it in project `p`, or `None` when there is
    /// nothing to do.
    pub fn advice(self, cfg: &Config, p: &Project) -> Option<String> {
        match self {
            Approval::Approved => None,
            Approval::NotAsked => Some(
                "Claude reads no house file here until you open `claude` in it once and allow external imports".to_string(),
            ),
            // Claude never asks again and has no command to undo a no, so its
            // own file is the only way back.
            Approval::Declined => Some(format!(
                "you told Claude not to load files outside this project, so it reads no house file here: with Claude closed, set `hasClaudeMdExternalIncludesApproved` to `true` under `projects` > `{}` in {}",
                p.claude_key.display(),
                tilde(&cfg.claude_state)
            )),
        }
    }
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
        let claude = cfg.agents.iter().any(|a| a.name == "claude" && a.active());
        let approvals = approvals(cfg, claude);
        let list = roots
            .iter()
            .map(|r| {
                let mut p = project(r, &house, claude);
                if p.uses_canon() {
                    p.claude = approvals.as_ref().map(|a| approval(a, &mut p));
                }
                p
            })
            .collect();
        Projects { house, list }
    }

    /// What `fix` does on its own: repoint or move broken imports, and wire
    /// CANON.md in every project that imports a house file.
    pub fn fixes(&self) -> Vec<Change> {
        let mut out = Vec::new();
        for p in &self.list {
            out.extend(p.settleable().filter_map(|c| c.change.clone()));
            if p.uses_canon() {
                out.extend(p.wiring_changes());
            }
        }
        out
    }

    /// The projects whose house files Claude will not load until the user
    /// answers it, with what to do; `fix` cannot answer for them.
    pub fn unapproved(&self) -> Vec<(&Project, Approval)> {
        self.list
            .iter()
            .filter_map(|p| {
                p.claude
                    .filter(|a| *a != Approval::Approved)
                    .map(|a| (p, a))
            })
            .collect()
    }

    pub fn drifted(&self) -> bool {
        self.list.iter().any(|p| {
            p.settleable().next().is_some() || (p.uses_canon() && !p.wiring_changes().is_empty())
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

    /// The broken cells `fix` repairs. While Claude cannot be pointed at
    /// CANON.md, a house import Claude reads from CLAUDE.md or AGENTS.md stays
    /// there, because moving it would leave Claude without it.
    fn settleable(&self) -> impl Iterator<Item = &Cell> {
        let blocked = self
            .wiring
            .iter()
            .any(|w| matches!(w.state, State::Foreign(_)));
        self.cells.iter().filter(move |c| {
            matches!(c.state, State::Broken(_))
                && !(blocked && !matches!(c.change, Some(Change::ReplaceImport { .. })))
        })
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

fn project(root: &Path, house: &[PathBuf], claude: bool) -> Project {
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
        wiring: wiring(root, claude),
        cells,
        dead,
        claude: None,
        claude_key: root.to_path_buf(),
    }
}

/// The `projects` table of Claude Code's state file, or `None` when Claude is
/// not in use or the file cannot be read.
fn approvals(cfg: &Config, claude: bool) -> Option<serde_json::Value> {
    if !claude {
        return None;
    }
    let text = fs::read_to_string(&cfg.claude_state).ok()?;
    let mut doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(doc.get_mut("projects")?.take())
}

/// What Claude recorded for project `p`, which it files under the git repo's
/// root (checked against Claude Code 2.1.287), else the folder it was opened in.
fn approval(projects: &serde_json::Value, p: &mut Project) -> Approval {
    let top = std::process::Command::new("git")
        .arg("-C")
        .arg(&p.root)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let real = fs::canonicalize(&p.root).ok();
    let keys: Vec<PathBuf> = top
        .into_iter()
        .chain([p.root.clone()])
        .chain(real)
        .collect();
    p.claude_key = keys[0].clone();
    let entry = keys.iter().find_map(|k| {
        let e = projects.get(k.to_str()?)?;
        p.claude_key = k.clone();
        Some(e)
    });
    let said = |key: &str| {
        entry
            .and_then(|e| e.get(key))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    };
    if said("hasClaudeMdExternalIncludesApproved") {
        Approval::Approved
    } else if said("hasClaudeMdExternalIncludesWarningShown") {
        Approval::Declined
    } else {
        Approval::NotAsked
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

/// What makes the agents read this project's CANON.md: it kept out of git,
/// and for Claude when `claude` is in use, a CLAUDE.local.md naming it. pi and
/// opencode load it on their own once the agents tab has wired them.
fn wiring(root: &Path, claude: bool) -> Vec<Wire> {
    let mut out = vec![ignored(root, CANON_FILE, root.join(CANON_FILE).is_file())];
    if claude {
        out.extend(claude_wiring(root));
    }
    out
}

/// A CLAUDE.local.md kept out of git that imports CANON.md, and AGENTS.md
/// when Claude would otherwise stop reading it.
fn claude_wiring(root: &Path) -> Vec<Wire> {
    let local = root.join(LOCAL_FILE);
    let text = fs::read_to_string(&local).ok();
    let has = |line: &str| {
        text.as_deref()
            .is_some_and(|t| t.lines().any(|l| l.trim() == line))
    };
    let agents_line = "@AGENTS.md";
    let canon_line = format!("@{CANON_FILE}");
    let what = format!("`{canon_line}` in {LOCAL_FILE}");

    let link = fs::symlink_metadata(&local).is_ok_and(|m| m.file_type().is_symlink());
    let tracked = !link
        && text.is_some()
        && std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-files", "--error-unmatch", LOCAL_FILE])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    if link || tracked {
        let why = if link {
            format!(
                "{LOCAL_FILE} is a link, so canonize leaves it alone and cannot point Claude at CANON.md through it: make it a plain file, then run `canon fix`"
            )
        } else {
            format!(
                "{LOCAL_FILE} is tracked by git, so canonize leaves it alone and cannot point Claude at CANON.md through it: `git rm --cached {LOCAL_FILE}`, then `canon fix`"
            )
        };
        return vec![Wire {
            what,
            state: if has(&canon_line) {
                State::Linked
            } else {
                State::Foreign(why)
            },
            change: None,
            undo: None,
        }];
    }

    // Only a CLAUDE.local.md canonize started is canonize's to take apart.
    let started = text
        .as_deref()
        .is_some_and(|t| t.starts_with(LOCAL_HEADER.trim_end()));
    let only_ours = started
        && text.as_deref().is_some_and(|t| {
            t.lines().map(str::trim).all(|l| {
                l.is_empty() || l == LOCAL_HEADER.trim() || l == agents_line || l == canon_line
            })
        });
    let mut out = vec![ignored(root, LOCAL_FILE, only_ours)];

    let what_agents = format!("`{agents_line}` in {LOCAL_FILE}");
    // Claude reads AGENTS.md on its own only while the project has no
    // CLAUDE.md and no CLAUDE.local.md, so starting the second hides the first.
    // One the user started already did that, and is left as they wrote it.
    let claude_md = ["CLAUDE.md", ".claude/CLAUDE.md"]
        .iter()
        .any(|f| root.join(f).is_file());
    if has(agents_line) {
        out.push(Wire {
            what: what_agents,
            state: State::Linked,
            change: None,
            undo: started.then(|| Change::RemoveImport {
                file: local.clone(),
                line: agents_line.to_string(),
            }),
        });
    } else if (text.is_none() || started) && root.join("AGENTS.md").is_file() && !claude_md {
        out.push(Wire {
            what: what_agents,
            state: State::Missing,
            change: Some(Change::AddImport {
                file: local.clone(),
                line: agents_line.to_string(),
            }),
            undo: None,
        });
    }

    let claude = root.join("CLAUDE.md");
    let in_claude =
        fs::read_to_string(&claude).is_ok_and(|t| t.lines().any(|l| l.trim() == canon_line));
    let take_back = |file: &Path| Change::RemoveImport {
        file: file.to_path_buf(),
        line: canon_line.clone(),
    };
    // A `@CANON.md` in CLAUDE.md is how 0.2.0 wired Claude: that file may be
    // tracked, so the line moves out of it.
    out.push(match (has(&canon_line), in_claude) {
        (true, false) => Wire {
            what,
            state: State::Linked,
            change: None,
            undo: Some(take_back(&local)),
        },
        (true, true) => Wire {
            what,
            state: State::Broken("also in CLAUDE.md, which may be tracked".to_string()),
            change: Some(take_back(&claude)),
            undo: Some(Change::Batch {
                what: format!("delete `{canon_line}` from {LOCAL_FILE} and CLAUDE.md"),
                changes: vec![take_back(&local), take_back(&claude)],
            }),
        },
        (false, true) => Wire {
            what,
            state: State::Broken(format!(
                "in CLAUDE.md, which may be tracked, so it moves to {LOCAL_FILE}"
            )),
            change: Some(Change::MoveImport {
                from: claude.clone(),
                old: canon_line.clone(),
                to: local.clone(),
                new: canon_line.clone(),
            }),
            undo: Some(take_back(&claude)),
        },
        (false, false) => Wire {
            what,
            state: State::Missing,
            change: Some(Change::AddImport {
                file: local.clone(),
                line: canon_line.clone(),
            }),
            undo: None,
        },
    });
    out
}

/// `name` kept out of git by the project's `.gitignore`; the line goes back
/// only when `ours` says the file it was added for is canonize's alone.
fn ignored(root: &Path, name: &str, ours: bool) -> Wire {
    let what = format!("{name} kept out of git");
    if !root.join(".git").exists() {
        return Wire {
            what,
            state: State::Na,
            change: None,
            undo: None,
        };
    }
    let ignore = root.join(".gitignore");
    let listed = fs::read_to_string(&ignore).is_ok_and(|t| t.lines().any(|l| l.trim() == name));
    // `check-ignore` reads the index too, so a file somebody committed is
    // reported as not ignored however many times the line is there.
    let ignored = listed
        || std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["check-ignore", "-q", name])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    Wire {
        what,
        state: if ignored {
            State::Linked
        } else {
            State::Missing
        },
        change: (!ignored).then(|| Change::AppendLine {
            file: ignore.clone(),
            line: name.to_string(),
        }),
        undo: (listed && ours).then(|| Change::RemoveLine {
            file: ignore,
            line: name.to_string(),
        }),
    }
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

    /// A canon with one house file, Claude installed, and `dev/` as the folder
    /// to look in.
    fn world() -> (Temp, Config, PathBuf) {
        let t = Temp::new();
        t.write("canon/rules.yaml", "title: MYRULES\n");
        let house = t.write("canon/house/HOUSE-RUST.md", "# HOUSE-RUST\n");
        t.dir("dev");
        let claude = tmp::claude(&t.dir("home/.claude"));
        let cfg = tmp::config(tmp::source(&t.at("canon")), vec![claude], vec![t.at("dev")]);
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
            !claude.contains("@CANON.md"),
            "a tracked CLAUDE.md never names CANON.md: {claude}"
        );
        assert!(claude.contains("# app") && claude.contains("my own line"));
        let local = fs::read_to_string(t.at("dev/app/CLAUDE.local.md"))
            .expect("CLAUDE.local.md should be written");
        assert!(
            local.contains("@CANON.md"),
            "Claude is pointed at CANON.md: {local}"
        );

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
    fn a_project_with_only_agents_md_keeps_claude_reading_it_once_house_files_go_in() {
        let (t, cfg, _) = world();
        t.dir("dev/app/.git");
        t.write("dev/app/AGENTS.md", "# app\n");

        let project = only(&cfg);
        apply(project.cells[0].change.clone().into_iter().collect());
        apply(Projects::build(&cfg).fixes());

        let local = fs::read_to_string(t.at("dev/app/CLAUDE.local.md"))
            .expect("CLAUDE.local.md should be written");
        let lines: Vec<&str> = local.lines().filter(|l| l.starts_with('@')).collect();
        assert_eq!(
            lines,
            ["@AGENTS.md", "@CANON.md"],
            "a CLAUDE.local.md hides AGENTS.md from Claude unless it imports it"
        );
        assert_eq!(
            fs::read_to_string(t.at("dev/app/AGENTS.md")).expect("AGENTS.md should be there"),
            "# app\n",
            "AGENTS.md is tracked, so it is never written"
        );
        let ignore =
            fs::read_to_string(t.at("dev/app/.gitignore")).expect(".gitignore should be written");
        for name in ["CANON.md", "CLAUDE.local.md"] {
            assert!(
                ignore.lines().any(|l| l == name),
                "{name} is kept out of git: {ignore}"
            );
        }
        assert!(only(&cfg).cells[0].state == State::Linked);
        assert!(!Projects::build(&cfg).drifted(), "one fix is the whole job");
    }

    #[test]
    fn the_canon_md_line_0_2_0_wrote_into_claude_md_moves_out_of_it() {
        let (t, cfg, house) = world();
        t.write("dev/app/CLAUDE.md", "# app\n\n@CANON.md\n\nmy own line\n");
        t.write("dev/app/CANON.md", &format!("@{}\n", house.display()));

        assert!(Projects::build(&cfg).drifted(), "the old wiring is drift");
        apply(Projects::build(&cfg).fixes());

        let claude = fs::read_to_string(t.at("dev/app/CLAUDE.md")).expect("CLAUDE.md is kept");
        assert_eq!(claude, "# app\n\n\nmy own line\n", "only the line moved");
        let local = fs::read_to_string(t.at("dev/app/CLAUDE.local.md"))
            .expect("CLAUDE.local.md should be written");
        assert!(local.lines().any(|l| l == "@CANON.md"), "{local}");
        assert!(
            !local.contains("@AGENTS.md"),
            "with a CLAUDE.md, Claude never read AGENTS.md: {local}"
        );
        assert!(!Projects::build(&cfg).drifted());
    }

    #[test]
    fn taking_everything_back_leaves_a_claude_local_md_the_user_wrote() {
        let (t, cfg, _) = world();
        t.dir("dev/app/.git");
        t.write("dev/app/AGENTS.md", "# app\n");
        t.write("dev/app/CLAUDE.local.md", "my sandbox is example.com\n");
        t.write("dev/app/.gitignore", "CLAUDE.local.md\n");

        let project = only(&cfg);
        apply(project.cells[0].change.clone().into_iter().collect());
        apply(Projects::build(&cfg).fixes());
        apply(only(&cfg).undos());

        let local = fs::read_to_string(t.at("dev/app/CLAUDE.local.md"))
            .expect("the user's CLAUDE.local.md stays");
        assert!(local.contains("my sandbox is example.com"), "{local}");
        assert_eq!(
            local, "my sandbox is example.com\n",
            "the user's file ends as they wrote it"
        );
        let ignore = fs::read_to_string(t.at("dev/app/.gitignore")).expect(".gitignore stays");
        assert!(
            ignore.lines().any(|l| l == "CLAUDE.local.md"),
            "the user's file stays out of git: {ignore}"
        );
        assert!(!t.at("dev/app/CANON.md").exists());
    }

    #[test]
    fn taking_everything_back_deletes_the_claude_local_md_canonize_started() {
        let (t, cfg, _) = world();
        t.dir("dev/app/.git");
        t.write("dev/app/AGENTS.md", "# app\n");

        let project = only(&cfg);
        apply(project.cells[0].change.clone().into_iter().collect());
        apply(Projects::build(&cfg).fixes());
        apply(only(&cfg).undos());

        assert!(!t.at("dev/app/CLAUDE.local.md").exists());
        let ignore = fs::read_to_string(t.at("dev/app/.gitignore")).unwrap_or_default();
        assert!(
            ignore.trim().is_empty(),
            "every line canonize added goes back: {ignore}"
        );
    }

    #[test]
    fn a_project_waits_on_claude_until_it_allows_external_imports() {
        let (t, cfg, house) = world();
        let root = t.write("dev/app/CLAUDE.md", "# app\n");
        let root = root.parent().expect("a project folder").to_path_buf();
        t.write("dev/app/CANON.md", &format!("@{}\n", house.display()));
        let said = |approved: bool, shown: bool| {
            let state = serde_json::json!({ "projects": { root.to_str().expect("a UTF-8 temp path"): {
                "hasClaudeMdExternalIncludesApproved": approved,
                "hasClaudeMdExternalIncludesWarningShown": shown,
            }}});
            fs::write(&cfg.claude_state, state.to_string()).expect("could not write the state");
            only(&cfg).claude
        };

        assert!(
            said(true, true) == Some(Approval::Approved),
            "allowed once is allowed"
        );
        assert!(
            said(false, true) == Some(Approval::Declined),
            "asked and refused"
        );
        assert!(
            said(false, false) == Some(Approval::NotAsked),
            "never asked"
        );
        fs::write(&cfg.claude_state, "{\"projects\": {}}").expect("could not write the state");
        assert!(
            only(&cfg).claude == Some(Approval::NotAsked),
            "a project Claude has never opened has not been asked"
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
