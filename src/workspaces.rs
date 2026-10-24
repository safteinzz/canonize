//! Workspaces: folders kept outside the repos of the projects they work on
//! (notes, replies, data, a folder per issue), each pointed at one project or
//! more under `[workspaces]` in canonize.toml.
//!
//! At the top of each, canonize writes a CLAUDE.md for Claude and an AGENTS.md
//! for every other agent, which both read from any folder under it. The file
//! names the workspace's own CONTEXT.md (the user's notes) and CANON.md (the
//! conventions it imports, as a project does), then each project and the files
//! to read there by absolute path, its conventions included. Nothing in it
//! is an `@` import, so Claude has nothing to ask before reading it in a folder
//! it has never seen.

use crate::config::{self, Agent, Config, tilde};
use crate::plan::{self, CANON_FILE, Cell, Change, LOCAL_FILE, LOCAL_HEADER, State};
use crate::projects::{self, Project, Projects};
use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// The words on the first line that tell a workspace file canonize wrote from
/// one the user did, whatever the rest of the line says in a later version.
const MARK: &str = "Written by canonize for a workspace";

/// The first line of a workspace file: an HTML comment, which Claude strips.
const HEADER: &str = "<!-- Written by canonize for a workspace: `canon fix` rewrites it and `canon unlink` deletes it, so edit the link, not this file. -->";

/// The user's notes for a workspace, which every agent there is told to read
/// and to add to. canonize never writes it.
pub const CONTEXT_FILE: &str = "CONTEXT.md";

/// The project's own files a workspace's agents are told to read, in order.
const READ: [&str; 6] = [
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    LOCAL_FILE,
    "AGENTS.md",
    "AGENTS.override.md",
    "README.md",
];

/// The file `agent` reads at the top of a workspace: Claude reads no AGENTS.md.
fn file_for(agent: &Agent) -> &'static str {
    if agent.name == "claude" {
        "CLAUDE.md"
    } else {
        "AGENTS.md"
    }
}

pub struct Entry {
    /// The key as canonize.toml spells it.
    pub key: String,
    pub root: PathBuf,
    /// The projects it is for, in the order its line lists them.
    pub projects: Vec<PathBuf>,
    /// Whether the workspace folder is on this machine at all.
    pub here: bool,
    /// The projects whose folder is not there: while one is, nothing can be
    /// written, and only the user knows where it went.
    pub gone: Vec<PathBuf>,
    /// Whether it sits in a git repo. Outside one, Codex reads only the folder
    /// it starts in and Claude keeps MCP servers per folder, so from a folder
    /// under it neither reaches what is at its top.
    pub git: bool,
    /// Whether its CONTEXT.md is there.
    pub context: bool,
    /// `cells[agent]`, agents in config order: the file that agent reads.
    pub cells: Vec<Cell>,
    /// Its own conventions, imported in its CANON.md as a project's are.
    pub own: Project,
    /// `via[convention]`: the projects here that import it, so their section
    /// of the workspace's files already has agents read it.
    pub via: Vec<Vec<PathBuf>>,
}

pub struct Workspaces {
    pub list: Vec<Entry>,
}

impl Workspaces {
    pub fn build(cfg: &Config, projects: &Projects) -> Workspaces {
        let list = cfg
            .workspaces
            .iter()
            .map(|w| entry(cfg, projects, &w.key, &w.root, &w.projects))
            .collect();
        Workspaces { list }
    }

    /// Whether `fix` has something to write, for one agent or all of them.
    pub fn drifted(&self, only: Option<usize>) -> bool {
        !self.fixes(only).is_empty()
    }

    /// What `fix` does in every workspace (`Entry::fixes`).
    pub fn fixes(&self, only: Option<usize>) -> Vec<Change> {
        plan::dedup(self.list.iter().flat_map(|e| e.fixes(only)))
    }

    /// What `delete` takes back: everything canonize made in a workspace, or
    /// with `-a` the file that agent reads.
    pub fn undos(&self, only: Option<usize>) -> Vec<Change> {
        match only {
            Some(_) => plan::dedup(
                self.list
                    .iter()
                    .flat_map(|e| e.cells(only))
                    .filter_map(|c| c.undo.clone()),
            ),
            None => self.list.iter().flat_map(Entry::undos).collect(),
        }
    }
}

impl Entry {
    /// The projects that bring convention `c` here, by their folder names.
    pub fn brought(&self, c: usize) -> Vec<String> {
        self.via
            .get(c)
            .map_or_else(Vec::new, |v| v.iter().map(|p| name(p)).collect())
    }

    /// Why no agent reads the workspace's CANON.md, when none does: each file
    /// canonize writes here for an active agent is missing or the user's.
    /// The state is that of the first such file.
    pub fn unread(&self) -> Option<(&State, String)> {
        let active: Vec<&Cell> = self
            .cells
            .iter()
            .filter(|c| c.state != State::Absent)
            .collect();
        let first = active.first()?;
        if active
            .iter()
            .any(|c| matches!(c.state, State::Linked | State::Broken(_)))
        {
            return None;
        }
        let name = first.at.file_name()?.to_string_lossy();
        Some((
            &first.state,
            format!(
                "no agent there reads its CANON.md while {name} is {}",
                first.state.word()
            ),
        ))
    }

    /// What `fix` does here: write a file that is missing or lists what the
    /// projects no longer have, for one agent or all of them, and with `None`
    /// also repoint the workspace's convention imports and keep its CANON.md
    /// out of git.
    pub fn fixes(&self, only: Option<usize>) -> Vec<Change> {
        let mut out = self.file_fixes(only);
        if only.is_none() && self.here {
            out.extend(
                self.own
                    .cells
                    .iter()
                    .filter(|c| matches!(c.state, State::Broken(_)))
                    .filter_map(|c| c.change.clone()),
            );
            if self.own.uses_canon() {
                out.extend(self.own.wiring_changes());
            }
        }
        plan::dedup(out.into_iter())
    }

    /// The files of `fixes` alone.
    pub fn file_fixes(&self, only: Option<usize>) -> Vec<Change> {
        plan::dedup(
            self.cells(only)
                .filter(|c| c.state.drifted())
                .filter_map(|c| c.change.clone()),
        )
    }

    fn cells(&self, only: Option<usize>) -> impl Iterator<Item = &Cell> {
        self.cells
            .iter()
            .enumerate()
            .filter(move |(a, _)| only.is_none_or(|o| o == *a))
            .map(|(_, c)| c)
    }

    /// Everything canonize made here: its files, and the workspace's
    /// convention imports with the line keeping CANON.md out of git. The
    /// CONTEXT.md is the user's, and stays.
    pub fn undos(&self) -> Vec<Change> {
        let mut out = ours(&self.root);
        if self.here {
            out.extend(self.own.undos());
        }
        out
    }

    /// Why each file in the way is left alone, once per file however many
    /// agents read it.
    pub fn foreign(&self) -> Vec<String> {
        let mut said: Vec<&Path> = Vec::new();
        let mut out = Vec::new();
        for c in &self.cells {
            if let State::Foreign(why) = &c.state
                && !said.contains(&c.at.as_path())
            {
                said.push(&c.at);
                out.push(why.clone());
            }
        }
        out
    }
}

/// Pointing a workspace at more projects: the setting when it changes, which
/// goes first, since a file written for a link that was not kept names a
/// project nothing points at, then the files.
pub struct Linking {
    pub key: String,
    pub setting: Option<Change>,
    /// The workspace as it will stand, its cells carrying the files to write.
    pub entry: Entry,
}

/// Refuses moving `from` to `to` when a workspace would then hold, or sit in,
/// a project, the canon, an agent's own folder or another workspace, the
/// layout `link` refuses.
pub fn check_move(cfg: &Config, projects: &Projects, from: &Path, to: &Path) -> Result<()> {
    let from = fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
    let to = fs::canonicalize(to)
        .ok()
        .or_else(|| Some(fs::canonicalize(to.parent()?).ok()?.join(to.file_name()?)))
        .unwrap_or_else(|| to.to_path_buf());
    // Where a folder is once the move is done, and whether it went along.
    let at = |p: &Path| {
        let p = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        match p.strip_prefix(&from) {
            Ok(rest) => (to.join(rest), true),
            Err(_) => (p, false),
        }
    };
    let mut others: Vec<(&Path, &str)> = projects
        .list
        .iter()
        .map(|p| (p.root.as_path(), "a project"))
        .collect();
    others.extend(
        cfg.workspaces
            .iter()
            .flat_map(|w| &w.projects)
            .map(|p| (p.as_path(), "a project")),
    );
    others.push((&cfg.source.root, "your canon"));
    others.extend(
        cfg.agents
            .iter()
            .map(|a| (a.home.as_path(), "an agent's own folder")),
    );
    for w in &cfg.workspaces {
        let (root, moved) = at(&w.root);
        let rest = cfg
            .workspaces
            .iter()
            .filter(|o| !same(&o.root, &w.root))
            .map(|o| (o.root.as_path(), "another workspace"));
        for (other, what) in others.iter().copied().chain(rest) {
            let (other, went) = at(other);
            if (moved || went) && (root.starts_with(&other) || other.starts_with(&root)) {
                bail!(
                    "the workspace at `{}` would then overlap {what} at `{}`, so nothing was moved: a workspace is a folder of its own, outside every project, workspace and agent folder and your canon",
                    tilde(&root),
                    tilde(&other)
                );
            }
        }
    }
    Ok(())
}

/// What `canon link` does to point the folder `workspace` (as typed) at the
/// projects in `adding` as well as the ones it has, or why it cannot. A
/// folder already linked keeps the key its line has.
pub fn link(
    cfg: &Config,
    projects: &Projects,
    workspace: &str,
    adding: &[PathBuf],
) -> Result<Linking> {
    let root = real(workspace);
    if !root.is_dir() {
        bail!(
            "`{}` is not a folder: make it first, then try again",
            tilde(&root)
        );
    }
    let known = cfg.workspaces.iter().find(|w| same(&w.root, &root));
    // An agent started anywhere under the workspace reads its file, and one in
    // a project reads the project's, so neither may hold the other; nor may it
    // land in the canon or an agent's own folder, which canonize never writes.
    let overlaps = |other: &Path| {
        let other = fs::canonicalize(other).unwrap_or_else(|_| other.to_path_buf());
        root.starts_with(&other) || other.starts_with(&root)
    };
    let mut taken: Vec<(&Path, &str)> = adding
        .iter()
        .chain(known.into_iter().flat_map(|w| &w.projects))
        .map(|p| (p.as_path(), "a project it is for"))
        .collect();
    taken.extend(
        projects
            .list
            .iter()
            .map(|p| (p.root.as_path(), "a project")),
    );
    taken.push((&cfg.source.root, "your canon"));
    taken.extend(
        cfg.agents
            .iter()
            .map(|a| (a.home.as_path(), "an agent's own folder")),
    );
    taken.extend(
        cfg.workspaces
            .iter()
            .filter(|w| !same(&w.root, &root))
            .map(|w| (w.root.as_path(), "another workspace")),
    );
    if let Some((other, what)) = taken.into_iter().find(|(other, _)| overlaps(other)) {
        bail!(
            "`{}` overlaps {what} at `{}`: a workspace is a folder of its own, outside every project, workspace and agent folder and your canon",
            tilde(&root),
            tilde(other)
        );
    }
    let key = known.map_or_else(|| tilde(&root), |w| w.key.clone());
    let mut list = known.map_or_else(Vec::new, |w| w.projects.clone());
    for p in adding {
        if !list.iter().any(|x| same(x, p)) {
            list.push(p.clone());
        }
    }
    let changed = known.is_none_or(|w| w.projects.len() != list.len());
    let setting = changed.then(|| setting(cfg, &key, Some(&list)));
    let entry = entry(cfg, projects, &key, &root, &list);
    Ok(Linking {
        key,
        setting,
        entry,
    })
}

/// The line under `[workspaces]` pointing `key` at `projects`, or taking it
/// out with `None`.
fn setting(cfg: &Config, key: &str, projects: Option<&[PathBuf]>) -> Change {
    Change::Workspace {
        config: cfg.path.clone(),
        key: key.to_string(),
        projects: projects.map(|l| l.iter().map(|p| tilde(p)).collect()),
    }
}

/// What `canon unlink` does to `e`: take back the MCP servers it has (its
/// cells in `Mcps::spaces`), everything else canonize made there, then forget
/// it.
pub fn unlinking(cfg: &Config, e: &Entry, servers: &[projects::Cell]) -> Vec<Change> {
    let mut out: Vec<Change> = servers.iter().filter_map(|c| c.undo.clone()).collect();
    out.extend(e.undos());
    out.push(setting(cfg, &e.key, None));
    out
}

/// What `canon unlink` does to drop the projects in `removing`, as `e` lists
/// them: point it at the ones left and rewrite its files for them, or, when
/// none is left, all `unlinking` does.
pub fn dropping(
    cfg: &Config,
    projects: &Projects,
    e: &Entry,
    servers: &[projects::Cell],
    removing: &[PathBuf],
) -> Vec<Change> {
    let left: Vec<PathBuf> = e
        .projects
        .iter()
        .filter(|p| !removing.contains(p))
        .cloned()
        .collect();
    if left.is_empty() {
        return unlinking(cfg, e, servers);
    }
    let mut out = vec![setting(cfg, &e.key, Some(&left))];
    out.extend(entry(cfg, projects, &e.key, &e.root, &left).file_fixes(None));
    out
}

/// Where the agents in a workspace get a convention or a server from outside
/// it: the projects there that import it, and the active agents that have it
/// everywhere or do not.
pub struct Reach {
    pub projects: Vec<String>,
    pub has: Vec<String>,
    pub lacks: Vec<String>,
}

impl Reach {
    /// `everywhere` is the thing's cell per agent, in config order, in each
    /// agent's own global config.
    pub fn new(cfg: &Config, projects: Vec<String>, everywhere: &[Cell]) -> Reach {
        let (has, lacks): (Vec<_>, Vec<_>) = cfg
            .agents
            .iter()
            .zip(everywhere)
            .filter(|(a, _)| a.active())
            .partition(|(_, c)| c.state == State::Linked);
        let names = |l: Vec<(&Agent, &Cell)>| l.into_iter().map(|(a, _)| a.name.clone()).collect();
        Reach {
            projects,
            has: names(has),
            lacks: names(lacks),
        }
    }

    /// Whether every agent there gets it without the workspace's own.
    pub fn covered(&self) -> bool {
        !self.projects.is_empty() || (!self.has.is_empty() && self.lacks.is_empty())
    }

    /// Where it comes from, `verb` saying what one agent and several do with
    /// it (`("reads", "read")`), or `None` when nothing brings it.
    pub fn note(&self, verb: (&str, &str)) -> Option<String> {
        let mut parts = Vec::new();
        if !self.projects.is_empty() {
            let v = if self.projects.len() == 1 {
                "imports"
            } else {
                "import"
            };
            parts.push(format!("{} {v} it", sentence(&self.projects)));
        }
        if !self.has.is_empty() {
            let v = if self.has.len() == 1 { verb.0 } else { verb.1 };
            parts.push(format!("{} {v} it everywhere", sentence(&self.has)));
            if self.projects.is_empty() && !self.lacks.is_empty() {
                let not = if self.lacks.len() == 1 {
                    "does not"
                } else {
                    "do not"
                };
                parts.push(format!("{} {not}", sentence(&self.lacks)));
            }
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

/// One workspace as it stands: `root` pointed at the projects in `list`.
pub fn entry(cfg: &Config, projects: &Projects, key: &str, root: &Path, list: &[PathBuf]) -> Entry {
    let here = root.is_dir();
    let gone: Vec<PathBuf> = list.iter().filter(|p| !p.is_dir()).cloned().collect();
    let brought: Vec<Vec<PathBuf>> = list.iter().map(|p| conventions(projects, p)).collect();
    let sections: Vec<Section> = list
        .iter()
        .zip(&brought)
        .map(|(p, c)| (p.clone(), reads(p, p, c)))
        .collect();
    let body = body(root, &sections);
    let names = names(list);
    let git = here && projects::in_git(root);
    let cells = cfg
        .agents
        .iter()
        .map(|agent| {
            let file = root.join(file_for(agent));
            if !agent.active() || !here {
                return plan::absent(file, State::Absent);
            }
            let mut c = cell(&file, &body, &names);
            if git {
                c = kept_out(root, file_for(agent), c);
            }
            // Rewriting would only list a project that is not there.
            let why = match gone.as_slice() {
                [] if list.is_empty() => "it points at no project".to_string(),
                [] => return c,
                [one] => format!("the project at `{}` is not there", tilde(one)),
                many => format!(
                    "the projects at `{}` are not there",
                    many.iter()
                        .map(|p| tilde(p))
                        .collect::<Vec<_>>()
                        .join("`, `")
                ),
            };
            match c.state {
                State::Foreign(_) => c,
                _ => Cell {
                    state: State::Broken(why),
                    change: None,
                    ..c
                },
            }
        })
        .collect();
    Entry {
        key: key.to_string(),
        root: root.to_path_buf(),
        projects: list.to_vec(),
        here,
        gone,
        git,
        context: root.join(CONTEXT_FILE).is_file(),
        cells,
        own: if here {
            projects::workspace(root, &projects.house)
        } else {
            // Nothing is read in a folder that is not on this machine.
            let mut own = projects::workspace(root, &projects.house);
            for c in &mut own.cells {
                *c = projects::Cell {
                    state: State::Absent,
                    change: None,
                    undo: None,
                };
            }
            own
        },
        via: projects
            .house
            .iter()
            .map(|h| {
                list.iter()
                    .zip(&brought)
                    .filter(|(_, c)| c.contains(h))
                    .map(|(p, _)| p.clone())
                    .collect()
            })
            .collect(),
    }
}

/// The projects in `list` by their folder names, as a sentence says them.
fn names(list: &[PathBuf]) -> String {
    let names: Vec<String> = list.iter().map(|p| name(p)).collect();
    sentence(&names)
}

/// `names` as a sentence lists them: "a, b and c".
fn sentence(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn name(project: &Path) -> String {
    project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| project.display().to_string())
}

/// The file at `file`, set against what canonize would write there now for
/// the projects `names`.
fn cell(file: &Path, body: &str, names: &str) -> Cell {
    let at = file.to_path_buf();
    if plan::is_link(file) {
        return plan::absent(
            at,
            State::Foreign(format!(
                "`{}` is a link, so canonize leaves it alone",
                tilde(file)
            )),
        );
    }
    let write = Change::WriteFile {
        file: at.clone(),
        body: body.to_string(),
        what: format!("pointing agents there at {names}"),
    };
    let delete = Change::DeleteFile {
        file: at.clone(),
        what: "the workspace file canonize wrote".into(),
    };
    let text = match fs::read(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return plan::absent(
                at,
                State::Foreign(format!(
                    "`{}` cannot be read, so canonize leaves it alone",
                    tilde(file)
                )),
            );
        }
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                return plan::absent(
                    at,
                    State::Foreign(format!(
                        "`{}` is not valid UTF-8, so canonize leaves it alone",
                        tilde(file)
                    )),
                );
            }
        },
    };
    match text {
        None => Cell {
            state: State::Missing,
            at,
            change: Some(write),
            undo: None,
        },
        Some(text) if !is_ours(&text) => plan::absent(
            at,
            State::Foreign(format!(
                "`{}` is a file of yours, so canonize leaves it alone",
                tilde(file)
            )),
        ),
        Some(text) if text == body => Cell {
            state: State::Linked,
            at,
            change: None,
            undo: Some(delete),
        },
        Some(_) => Cell {
            state: State::Broken("is not what canonize writes for it now".into()),
            change: Some(rewrite(&at, write)),
            at,
            undo: Some(delete),
        },
    }
}

/// `c`, the file `name` canonize writes at the top of `root`, a folder of a
/// git repo, also kept out of git: it names paths on this machine alone. The
/// line is anchored, since a bare name would hide the user's own files of
/// that name in every folder under it. The line stays when the file goes.
fn kept_out(root: &Path, name: &str, c: Cell) -> Cell {
    if !matches!(c.state, State::Linked | State::Missing | State::Broken(_)) {
        return c;
    }
    let line = format!("/{name}");
    let ignore = root.join(".gitignore");
    let listed = fs::read_to_string(&ignore).is_ok_and(|t| t.lines().any(|l| l.trim() == line));
    let ignored = listed
        || std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["check-ignore", "-q", name])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    if ignored {
        return c;
    }
    let append = Change::AppendLine { file: ignore, line };
    match c.change {
        None => Cell {
            state: State::Broken("is not kept out of git".into()),
            change: Some(append),
            ..c
        },
        Some(write) => Cell {
            change: Some(Change::Batch {
                what: format!("{}, and keep it out of git", write.describe()),
                changes: vec![write, append],
            }),
            ..c
        },
    }
}

fn is_ours(text: &str) -> bool {
    text.lines().next().is_some_and(|l| l.contains(MARK))
}

/// Delete the file canonize wrote at `file`, then write it again.
fn rewrite(file: &Path, write: Change) -> Change {
    Change::Batch {
        what: format!("update {}, which canonize wrote", tilde(file)),
        changes: vec![
            Change::DeleteFile {
                file: file.to_path_buf(),
                what: "the workspace file canonize wrote".into(),
            },
            write,
        ],
    }
}

/// Deleting every file canonize wrote at the top of the workspace `root`,
/// whichever agent reads it.
pub fn ours(root: &Path) -> Vec<Change> {
    ["CLAUDE.md", "AGENTS.md"]
        .iter()
        .map(|f| root.join(f))
        .filter(|f| !plan::is_link(f))
        .filter(|f| {
            plan::read_text(f)
                .ok()
                .flatten()
                .is_some_and(|t| is_ours(&t))
        })
        .map(|file| Change::DeleteFile {
            file,
            what: "the workspace file canonize wrote".into(),
        })
        .collect()
}

/// The conventions the project at `root` imports, or is meant to once `fix`
/// repoints them, as their paths in the canon. A project outside the folders
/// canonize looks through is read where it is.
pub fn conventions(projects: &Projects, root: &Path) -> Vec<PathBuf> {
    match projects.list.iter().find(|p| same(&p.root, root)) {
        Some(p) => after(projects, p, &[], &[]),
        None if root.is_dir() => after(
            projects,
            &crate::projects::project(root, &projects.house, false),
            &[],
            &[],
        ),
        None => Vec::new(),
    }
}

/// The conventions `p` imports once columns `add` are added and `remove`
/// taken out, in the canon's order.
fn after(projects: &Projects, p: &Project, add: &[usize], remove: &[usize]) -> Vec<PathBuf> {
    projects
        .house
        .iter()
        .zip(&p.cells)
        .enumerate()
        .filter(|(i, (_, c))| {
            !remove.contains(i)
                && (add.contains(i) || matches!(c.state, State::Linked | State::Broken(_)))
        })
        .map(|(_, (h, _))| h.clone())
        .collect()
}

/// What a workspace tells its agents to read: the project's own files that are
/// there in `now`, named under `root` (where it will be after a move), then
/// `conventions`. A CLAUDE.local.md canonize started holds only imports of
/// files already on the list.
pub fn reads(now: &Path, root: &Path, conventions: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = READ
        .iter()
        .filter(|f| {
            let file = now.join(f);
            if **f == LOCAL_FILE {
                return fs::read_to_string(&file).is_ok_and(|t| !t.starts_with(LOCAL_HEADER));
            }
            file.is_file()
        })
        .map(|f| root.join(f))
        .collect();
    out.extend(conventions.iter().cloned());
    out
}

/// A project as a workspace's file lists it: its path, and what to read there.
pub type Section = (PathBuf, Vec<PathBuf>);

/// The project at `project` as a workspace lists it today.
fn section(projects: &Projects, project: &Path) -> Section {
    (
        project.to_path_buf(),
        reads(project, project, &conventions(projects, project)),
    )
}

/// The file canonize writes at the top of the workspace at `root` for the
/// projects in `sections`: what the workspace holds, its CONTEXT.md and
/// CANON.md whether they are there yet or not, so neither appearing changes
/// it, then each project and the files to read there.
pub fn body(root: &Path, sections: &[Section]) -> String {
    let list: Vec<PathBuf> = sections.iter().map(|(p, _)| p.clone()).collect();
    let mut out = format!(
        "{HEADER}\n# Workspace for {}\n\nThis folder and every folder under it hold the working files (notes, replies, data) for {}. Write those files in the folder you were started in. Changes to a project itself, its code included, are made in that project, as they would be working inside it.\n",
        names(&list),
        if sections.len() == 1 {
            "the project below"
        } else {
            "the projects below"
        }
    );
    out.push_str(&format!(
        "\nBefore anything else, read these two files when they are there, and follow them with every file they import through an `@path` line:\n\n- `{}`: what to know working here. When you are told to remember something for this workspace, write it there.\n- `{}`: the conventions this workspace follows.\n",
        root.join(CONTEXT_FILE).display(),
        root.join(CANON_FILE).display()
    ));
    for (project, reads) in sections {
        out.push_str(&format!(
            "\n## {}\n\nThe project at `{}`, whose code lives there.",
            name(project),
            project.display()
        ));
        if reads.is_empty() {
            out.push('\n');
            continue;
        }
        out.push_str(" Read these files and follow them as you would working inside it, with every file they import through an `@path` line:\n\n");
        for r in reads {
            out.push_str(&format!("- `{}`\n", r.display()));
        }
    }
    out
}

/// Bringing the files canonize wrote in the workspace at `now` in line with
/// `body`, each written where it will be (`root`); a file not there yet is
/// left to `fix`.
pub fn rewrites(cfg: &Config, now: &Path, root: &Path, body: &str) -> Vec<Change> {
    let mut names: Vec<&str> = cfg
        .agents
        .iter()
        .filter(|a| a.active())
        .map(file_for)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .filter(|f| !plan::is_link(&now.join(f)))
        .filter(|f| {
            plan::read_text(&now.join(f))
                .ok()
                .flatten()
                .is_some_and(|t| is_ours(&t) && t != body)
        })
        .map(|f| {
            let file = root.join(f);
            rewrite(
                &file,
                Change::WriteFile {
                    file: file.clone(),
                    body: body.to_string(),
                    what: "pointing agents there at its project".into(),
                },
            )
        })
        .collect()
}

/// The same folder, however each path reaches it.
pub fn same(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (fs::canonicalize(a), fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// A path as the user typed it, made absolute and real, for a key that matches
/// what the folder is whichever way it was reached.
pub fn real(arg: &str) -> PathBuf {
    let p = config::expand(arg);
    fs::canonicalize(&p).unwrap_or(p)
}

/// A project with the convention columns about to be added to it, then
/// those about to be taken out.
pub type Edit<'a> = (&'a Project, Vec<usize>, Vec<usize>);

/// Rewriting every workspace of the projects in `edits` for when their
/// columns are added and taken out, which `add` and `remove` are about to
/// make true, each workspace once with every edit of its projects. A
/// workspace with a project gone is left to the user, as in `fix`.
pub fn refresh(cfg: &Config, projects: &Projects, edits: &[Edit]) -> Vec<Change> {
    let edited = |q: &Path| {
        edits
            .iter()
            .find(|(p, _, _)| same(q, &p.root))
            .map(|(p, add, remove)| after(projects, p, add, remove))
    };
    cfg.workspaces
        .iter()
        .filter(|w| w.root.is_dir() && w.projects.iter().any(|q| edited(q).is_some()))
        .filter(|w| w.projects.iter().all(|q| q.is_dir()))
        .flat_map(|w| {
            let sections: Vec<Section> = w
                .projects
                .iter()
                .map(|q| match edited(q) {
                    Some(conventions) => (q.clone(), reads(q, q, &conventions)),
                    None => section(projects, q),
                })
                .collect();
            rewrites(cfg, &w.root, &w.root, &body(&w.root, &sections))
        })
        .collect()
}
