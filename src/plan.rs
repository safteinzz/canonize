//! What each agent has, set against what the source says it should have, and
//! the changes that would close the gap.
//!
//! Only two kinds of thing are ever touched: symlinks that point into the
//! source, and `@path` lines naming a file in the source. A real file, or a
//! link pointing anywhere else, is reported as foreign and left alone.

use crate::config::{self, Agent, Config, RulesMode, SkillsMode, tilde};
use anyhow::{Context, Result, bail};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq)]
pub enum Row {
    /// The rules file, by its name in the canon.
    Rules(String),
    /// How the agent loads a project's CANON.md.
    Loader,
    Skill(String),
}

impl Row {
    pub fn label(&self) -> String {
        match self {
            Row::Rules(name) => name.clone(),
            Row::Loader => "reads CANON.md".to_string(),
            Row::Skill(name) => format!("skill {name}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum State {
    Linked,
    Missing,
    /// Wired to the source, but wrongly: the reason says how.
    Broken(String),
    /// Something the user or the agent put there, which canonize never touches.
    Foreign(String),
    /// A skill the agent has as a real folder and the canon does not: `f`
    /// adopts it into the canon.
    Own,
    /// Does not apply to this agent: it has no way to use it.
    Na,
    /// The agent's config turns this off.
    Off,
    /// The agent is not installed, or is disabled.
    Absent,
}

impl State {
    pub fn word(&self) -> &'static str {
        match self {
            State::Linked => "linked",
            State::Missing => "unwired",
            State::Broken(_) => "broken",
            State::Foreign(_) => "foreign",
            State::Own => "own",
            State::Na => "n/a",
            State::Off => "off",
            State::Absent => "-",
        }
    }

    /// Whether this cell is something `fix` is supposed to have fixed.
    pub fn drifted(&self) -> bool {
        matches!(self, State::Missing | State::Broken(_))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Change {
    Link {
        at: PathBuf,
        to: PathBuf,
    },
    /// Replace a link into the source that points at the wrong thing.
    Relink {
        at: PathBuf,
        to: PathBuf,
    },
    AddImport {
        file: PathBuf,
        line: String,
    },
    ReplaceImport {
        file: PathBuf,
        old: String,
        new: String,
    },
    Unlink {
        at: PathBuf,
    },
    /// Append a line to a file, creating it: a `.gitignore` entry.
    AppendLine {
        file: PathBuf,
        line: String,
    },
    /// Move an import from the file it sits in into another (CLAUDE.md to
    /// CANON.md), repointed on the way when it names a moved file.
    MoveImport {
        from: PathBuf,
        old: String,
        to: PathBuf,
        new: String,
    },
    /// Write a file canonize owns, only when it is not there yet.
    WriteFile {
        file: PathBuf,
        body: String,
        what: String,
    },
    /// Add a value to the `instructions` list of an opencode JSON config.
    JsonInstruction {
        file: PathBuf,
        value: String,
    },
    /// Delete a real folder: an agent's own skill, which exists nowhere else.
    DeleteDir {
        at: PathBuf,
    },
    /// Delete a file canonize wrote itself, such as the pi extension.
    DeleteFile {
        file: PathBuf,
        what: String,
    },
    /// Take a value back out of an opencode config's `instructions` list.
    JsonUninstruction {
        file: PathBuf,
        value: String,
    },
    /// Drop a line canonize added to a file, such as a `.gitignore` entry.
    RemoveLine {
        file: PathBuf,
        line: String,
    },
    /// Several changes that only make sense together, run in order.
    Batch {
        what: String,
        changes: Vec<Change>,
    },
    /// Move an agent's own skill folder into the canon and link it back.
    Adopt {
        from: PathBuf,
        to: PathBuf,
    },
    RemoveImport {
        file: PathBuf,
        line: String,
    },
}

impl Change {
    /// The key two changes are the same by, so agents sharing a folder get
    /// one link.
    fn key(&self) -> (&Path, &str) {
        match self {
            Change::Link { at, .. } | Change::Relink { at, .. } | Change::Unlink { at } => (at, ""),
            Change::Adopt { from, .. } => (from, ""),
            Change::DeleteDir { at } => (at, "delete"),
            Change::DeleteFile { file, .. } => (file, "delete"),
            Change::JsonUninstruction { file, value } => (file, value),
            Change::RemoveLine { file, line } => (file, line),
            Change::Batch { changes, .. } => {
                changes.first().map_or((Path::new(""), ""), Change::key)
            }
            Change::AppendLine { file, line } => (file, line),
            Change::MoveImport { to, new, .. } => (to, new),
            Change::WriteFile { file, .. } => (file, ""),
            Change::JsonInstruction { file, value } => (file, value),
            Change::AddImport { file, line } | Change::RemoveImport { file, line } => (file, line),
            Change::ReplaceImport { file, new, .. } => (file, new),
        }
    }

    /// The one or two words a key does this with, shown beside the key.
    pub fn verb(&self) -> &'static str {
        match self {
            Change::Link { .. } => "link",
            Change::Relink { .. } => "relink",
            Change::AddImport { .. } => "add import",
            Change::ReplaceImport { .. } => "repoint",
            Change::Unlink { .. } => "unlink",
            Change::RemoveImport { .. } => "delete import",
            Change::AppendLine { .. } => "add line",
            Change::MoveImport { .. } => "move import",
            Change::WriteFile { .. } => "install",
            Change::JsonInstruction { .. } => "add setting",
            Change::DeleteDir { .. } => "delete",
            Change::DeleteFile { .. } => "uninstall",
            Change::JsonUninstruction { .. } => "delete setting",
            Change::RemoveLine { .. } => "delete line",
            Change::Batch { changes, .. } => changes.first().map_or("do", Change::verb),
            Change::Adopt { .. } => "adopt",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Change::Link { at, to } => format!("link {} -> {}", tilde(at), tilde(to)),
            Change::Relink { at, to } => format!("relink {} -> {}", tilde(at), tilde(to)),
            Change::AddImport { file, line } => format!("add `{line}` to {}", tilde(file)),
            Change::ReplaceImport { file, old, new } => {
                format!("replace `{old}` with `{new}` in {}", tilde(file))
            }
            Change::Unlink { at } => format!("delete link {}", tilde(at)),
            Change::AppendLine { file, line } => format!("add `{line}` to {}", tilde(file)),
            Change::DeleteDir { at } => format!("delete {} and everything in it", tilde(at)),
            Change::DeleteFile { file, what } => format!("delete {} ({what})", tilde(file)),
            Change::JsonUninstruction { file, value } => {
                format!("delete \"{value}\" from `instructions` in {}", tilde(file))
            }
            Change::RemoveLine { file, line } => format!("delete `{line}` from {}", tilde(file)),
            Change::Batch { what, .. } => what.clone(),
            Change::MoveImport { from, old, to, new } => {
                if old == new {
                    format!("move `{old}` from {} into {}", tilde(from), tilde(to))
                } else {
                    format!(
                        "move `{old}` from {} into {} as `{new}`",
                        tilde(from),
                        tilde(to)
                    )
                }
            }
            Change::WriteFile { file, what, .. } => format!("write {} ({what})", tilde(file)),
            Change::JsonInstruction { file, value } => {
                format!("add \"{value}\" to `instructions` in {}", tilde(file))
            }
            Change::Adopt { from, to } => format!(
                "move {} into your canon at {}, leaving a link in its place",
                tilde(from),
                tilde(to)
            ),
            Change::RemoveImport { file, line } => {
                format!("delete `{line}` from {}", tilde(file))
            }
        }
    }

    /// The shell commands that do the same, one per line, shown so the change
    /// teaches the tool underneath instead of hiding it.
    pub fn command(&self) -> String {
        match self {
            Change::Link { at, to } => format!("ln -s {} {}", tilde(to), tilde(at)),
            Change::Relink { at, to } => format!("ln -sfn {} {}", tilde(to), tilde(at)),
            Change::Unlink { at } => format!("rm {}", tilde(at)),
            Change::AppendLine { file, line } => format!("echo '{line}' >> {}", tilde(file)),
            Change::DeleteDir { at } => format!("rm -r {}", tilde(at)),
            Change::DeleteFile { file, .. } => format!("rm {}", tilde(file)),
            Change::JsonUninstruction { file, value } => format!(
                "jq '.instructions -= [\"{value}\"]' {f} > {f}.new && mv {f}.new {f}",
                f = tilde(file)
            ),
            Change::RemoveLine { file, line } => {
                format!("sed -i '\\#^{line}$#d' {}", tilde(file))
            }
            Change::Batch { changes, .. } => changes
                .iter()
                .map(Change::command)
                .collect::<Vec<_>>()
                .join("\n"),
            Change::MoveImport { from, old, to, new } => format!(
                "sed -i '\\#^{old}$#d' {}\necho '{new}' >> {}",
                tilde(from),
                tilde(to)
            ),
            Change::WriteFile { file, what, .. } => {
                format!("cat > {} <<'EOF'   # {what}", tilde(file))
            }
            Change::JsonInstruction { file, value } => format!(
                "jq '.instructions += [\"{value}\"]' {f} > {f}.new && mv {f}.new {f}",
                f = tilde(file)
            ),
            Change::Adopt { from, to } => format!(
                "mv {} {}\nln -s {} {}",
                tilde(from),
                tilde(to),
                tilde(to),
                tilde(from)
            ),
            Change::AddImport { file, line } if is_canon_file(file) => {
                format!("echo '{line}' >> {}", tilde(file))
            }
            Change::AddImport { file, line } => {
                if file.exists() {
                    format!("sed -i '1i {line}' {}", tilde(file))
                } else {
                    format!("echo '{line}' > {}", tilde(file))
                }
            }
            Change::ReplaceImport { file, old, new } => {
                format!("sed -i 's#^{old}$#{new}#' {}", tilde(file))
            }
            Change::RemoveImport { file, line } => {
                format!("sed -i '\\#^{line}$#d' {}", tilde(file))
            }
        }
    }

    pub fn run(&self) -> Result<()> {
        match self {
            Change::Link { at, to } => {
                if let Some(parent) = at.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("could not create `{}`", tilde(parent)))?;
                }
                symlink(to, at).with_context(|| format!("could not link `{}`", tilde(at)))
            }
            Change::Relink { at, to } => {
                remove_link(at)?;
                symlink(to, at).with_context(|| format!("could not link `{}`", tilde(at)))
            }
            Change::Unlink { at } => remove_link(at),
            Change::AppendLine { file, line } => {
                let mut text = fs::read_to_string(file).unwrap_or_default();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(line);
                text.push('\n');
                fs::write(file, text).with_context(|| format!("could not write `{}`", tilde(file)))
            }
            Change::DeleteDir { at } => {
                let meta = fs::symlink_metadata(at)
                    .with_context(|| format!("could not look at `{}`", tilde(at)))?;
                if !meta.is_dir() {
                    bail!("`{}` is not a real folder, so it was left alone", tilde(at));
                }
                fs::remove_dir_all(at).with_context(|| format!("could not delete `{}`", tilde(at)))
            }
            Change::MoveImport { from, old, to, new } => {
                edit_lines(from, |l| (l.trim() != old).then(|| l.to_string()))?;
                append_import(to, new)
            }
            Change::WriteFile { file, body, .. } => {
                if file.exists() {
                    return Ok(());
                }
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("could not create `{}`", tilde(parent)))?;
                }
                fs::write(file, body).with_context(|| format!("could not write `{}`", tilde(file)))
            }
            Change::JsonInstruction { file, value } => json_instruction(file, value, true),
            Change::JsonUninstruction { file, value } => json_instruction(file, value, false),
            Change::DeleteFile { file, .. } => {
                fs::remove_file(file).with_context(|| format!("could not delete `{}`", tilde(file)))
            }
            Change::RemoveLine { file, line } => {
                edit_lines(file, |l| (l.trim() != line).then(|| l.to_string()))
            }
            Change::Batch { changes, .. } => changes.iter().try_for_each(Change::run),
            Change::Adopt { from, to } => {
                crate::setup::adopt(from, to.parent().unwrap_or(Path::new("/"))).map(|_| ())
            }
            Change::AddImport { file, line } if is_canon_file(file) => append_import(file, line),
            Change::AddImport { file, .. }
                if fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink()) =>
            {
                bail!(
                    "`{}` is a link, so an import written there would land in the file it points at",
                    tilde(file)
                )
            }
            Change::AddImport { file, line } => {
                let old = fs::read_to_string(file).unwrap_or_default();
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("could not create `{}`", tilde(parent)))?;
                }
                let text = if old.is_empty() {
                    format!("{line}\n")
                } else {
                    format!("{line}\n{old}")
                };
                fs::write(file, text).with_context(|| format!("could not write `{}`", tilde(file)))
            }
            Change::ReplaceImport { file, old, new } => edit_lines(file, |l| {
                if l.trim() == old {
                    Some(new.clone())
                } else {
                    Some(l.to_string())
                }
            }),
            Change::RemoveImport { file, line } => {
                edit_lines(file, |l| (l.trim() != line).then(|| l.to_string()))?;
                // A CANON.md with no imports left is canonize's own leftover.
                if is_canon_file(file)
                    && fs::read_to_string(file)
                        .is_ok_and(|t| !t.lines().any(|l| l.trim_start().starts_with('@')))
                {
                    fs::remove_file(file)
                        .with_context(|| format!("could not remove `{}`", tilde(file)))?;
                }
                Ok(())
            }
        }
    }
}

/// The file name canonize keeps a project's house imports in.
pub const CANON_FILE: &str = "CANON.md";

const CANON_HEADER: &str = "# CANON.md: written by canonize, not tracked. `canon` edits it.\n";

fn is_canon_file(file: &Path) -> bool {
    file.file_name().is_some_and(|n| n == CANON_FILE)
}

/// Add an import at the end of a file, starting a CANON.md with its header.
fn append_import(file: &Path, line: &str) -> Result<()> {
    let mut text = fs::read_to_string(file).unwrap_or_default();
    if text.is_empty() && is_canon_file(file) {
        text.push_str(CANON_HEADER);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    fs::write(file, text).with_context(|| format!("could not write `{}`", tilde(file)))
}

/// Add `value` to the `instructions` list of an opencode config, keeping every
/// other key. A file with comments is not plain JSON and is left to the user.
fn json_instruction(file: &Path, value: &str, add: bool) -> Result<()> {
    let text = fs::read_to_string(file).unwrap_or_else(|_| "{}".to_string());
    let mut doc: serde_json::Value = serde_json::from_str(&text).with_context(|| {
        format!(
            "`{}` is not plain JSON, so add `\"instructions\": [\"{value}\"]` by hand",
            tilde(file)
        )
    })?;
    let Some(obj) = doc.as_object_mut() else {
        bail!("`{}` is not a JSON object", tilde(file));
    };
    let list = obj
        .entry("instructions")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(list) = list.as_array_mut() else {
        bail!("`instructions` in `{}` is not a list", tilde(file));
    };
    if add {
        if !list.iter().any(|v| v.as_str() == Some(value)) {
            list.push(serde_json::Value::String(value.to_string()));
        }
    } else {
        list.retain(|v| v.as_str() != Some(value));
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create `{}`", tilde(parent)))?;
    }
    let out = serde_json::to_string_pretty(&doc)? + "\n";
    fs::write(file, out).with_context(|| format!("could not write `{}`", tilde(file)))
}

/// Remove `at` only if it is a symlink: a real file there is the user's.
fn remove_link(at: &Path) -> Result<()> {
    let meta =
        fs::symlink_metadata(at).with_context(|| format!("could not look at `{}`", tilde(at)))?;
    if !meta.file_type().is_symlink() {
        bail!("`{}` is not a link, so it was left alone", tilde(at));
    }
    fs::remove_file(at).with_context(|| format!("could not remove `{}`", tilde(at)))
}

/// Rewrite `file` line by line, dropping the lines `f` returns `None` for.
/// Writing through the path keeps a symlinked instructions file a symlink.
fn edit_lines(file: &Path, f: impl Fn(&str) -> Option<String>) -> Result<()> {
    let text =
        fs::read_to_string(file).with_context(|| format!("could not read `{}`", tilde(file)))?;
    // Only the lines `f` drops go; everything else, blank lines included, is
    // the user's and stays exactly as it was.
    let out: Vec<String> = text.lines().filter_map(&f).collect();
    let mut body = out.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    fs::write(file, body).with_context(|| format!("could not write `{}`", tilde(file)))
}

pub struct Cell {
    pub state: State,
    /// Where in the agent's world this lives.
    pub at: PathBuf,
    pub change: Option<Change>,
    pub undo: Option<Change>,
}

pub struct Plan {
    pub rows: Vec<Row>,
    /// `cells[row][agent]`, agents in config order.
    pub cells: Vec<Vec<Cell>>,
    /// Per agent: what sits in its skills folder that is neither the canon's
    /// nor a skill of its own (no SKILL.md), like a folder the agent fills.
    pub foreign: Vec<Vec<String>>,
    /// How many skill rows are the canon's; the rest are agents' own.
    pub canon_skills: usize,
    /// Links into the source whose skill is gone from it, which apply removes.
    pub stale: Vec<Change>,
}

impl Plan {
    pub fn build(cfg: &Config) -> Plan {
        let skills = config::skill_names(&cfg.source);
        // An empty `rules` means the user keeps their rules in each agent's own
        // file, so there is no row for one.
        let mut rows: Vec<Row> = cfg
            .source
            .rules
            .file_name()
            .map(|n| Row::Rules(n.to_string_lossy().into_owned()))
            .into_iter()
            .collect();
        let loader = !cfg.projects.is_empty();
        if loader {
            rows.push(Row::Loader);
        }
        let first_skill = rows.len();
        rows.extend(skills.iter().cloned().map(Row::Skill));

        let mut cells: Vec<Vec<Cell>> = rows.iter().map(|_| Vec::new()).collect();
        let mut foreign = Vec::new();
        let mut stale = Vec::new();
        // Per agent, the skills it keeps itself: folders with a SKILL.md.
        let mut owns: Vec<Vec<String>> = Vec::new();
        for agent in &cfg.agents {
            if !rows.is_empty() && matches!(rows[0], Row::Rules(_)) {
                cells[0].push(rules_cell(cfg, agent));
            }
            if loader {
                cells[first_skill - 1].push(loader_cell(agent));
            }
            for (i, name) in skills.iter().enumerate() {
                cells[first_skill + i].push(skill_cell(cfg, agent, name));
            }
            let (other, gone) = leftovers(cfg, agent, &skills);
            let (own, rest): (Vec<String>, Vec<String>) = other
                .into_iter()
                .partition(|n| agent.skills.join(n).join("SKILL.md").is_file());
            owns.push(own);
            foreign.push(rest);
            stale.extend(gone);
        }
        // Each of those becomes a row of its own, so it is seen without digging.
        let mut own_names: Vec<String> = owns.iter().flatten().cloned().collect();
        own_names.sort();
        own_names.dedup();
        for name in own_names {
            let row: Vec<Cell> = cfg
                .agents
                .iter()
                .zip(&owns)
                .map(|(agent, own)| {
                    let at = agent.skills.join(&name);
                    if own.contains(&name) {
                        Cell {
                            state: State::Own,
                            change: Some(Change::Adopt {
                                from: at.clone(),
                                to: cfg.source.skills.join(&name),
                            }),
                            undo: Some(Change::DeleteDir { at: at.clone() }),
                            at,
                        }
                    } else {
                        absent(at, State::Absent)
                    }
                })
                .collect();
            rows.push(Row::Skill(name));
            cells.push(row);
        }
        Plan {
            rows,
            cells,
            foreign,
            canon_skills: skills.len(),
            stale,
        }
    }

    /// Every change `fix` would make, each target once.
    pub fn changes(&self) -> Vec<Change> {
        let all = self
            .cells
            .iter()
            .flatten()
            .filter_map(|c| c.change.clone())
            .filter(|c| !matches!(c, Change::Adopt { .. }))
            .chain(self.stale.iter().cloned());
        dedup(all)
    }

    /// Every change `fix` would make for one agent.
    pub fn changes_for(&self, agent: usize) -> Vec<Change> {
        let agent_path = |c: &Change| match c {
            Change::Unlink { at } => self.cells.iter().any(|r| at.starts_with(&r[agent].at)),
            _ => false,
        };
        dedup(
            self.cells
                .iter()
                .filter_map(|r| r[agent].change.clone())
                .filter(|c| !matches!(c, Change::Adopt { .. }))
                .chain(self.stale.iter().filter(|c| agent_path(c)).cloned()),
        )
    }

    /// Every change `remove` would make, each target once.
    pub fn undos(&self, agent: Option<usize>) -> Vec<Change> {
        dedup(
            self.cells
                .iter()
                .flat_map(|r| r.iter().enumerate())
                .filter(|(i, _)| agent.is_none_or(|a| a == *i))
                .filter_map(|(_, c)| c.undo.clone())
                .filter(|c| !matches!(c, Change::DeleteDir { .. })),
        )
    }

    /// The rows that are one agent's setup (rules, CANON.md loader), in order.
    pub fn setup_rows(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&r| !matches!(self.rows[r], Row::Skill(_)))
            .collect()
    }

    /// The skill rows, in order.
    pub fn skill_rows(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&r| matches!(self.rows[r], Row::Skill(_)))
            .collect()
    }

    pub fn drifted(&self) -> bool {
        self.cells.iter().flatten().any(|c| c.state.drifted()) || !self.stale.is_empty()
    }
}

fn dedup(changes: impl Iterator<Item = Change>) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    for c in changes {
        if !out.iter().any(|o| o.key() == c.key()) {
            out.push(c);
        }
    }
    out
}

fn absent(at: PathBuf, state: State) -> Cell {
    Cell {
        state,
        at,
        change: None,
        undo: None,
    }
}

fn rules_cell(cfg: &Config, agent: &Agent) -> Cell {
    if !agent.active() {
        return absent(agent.rules.clone(), State::Absent);
    }
    match agent.rules_mode {
        RulesMode::Off => absent(agent.rules.clone(), State::Off),
        RulesMode::Link => link_cell(&agent.rules, &cfg.source.rules, &cfg.source.root),
        RulesMode::Import => import_cell(&agent.rules, &cfg.source.rules, &cfg.source.root),
    }
}

/// The pi extension canonize writes for CANON.md.
const PI_EXTENSION: &str = include_str!("../templates/pi-canonize.ts");

/// Whether the agent will read a project's CANON.md. Claude does it through
/// the project's own CLAUDE.md, which the projects tab wires; pi through an
/// extension canonize writes; opencode through its `instructions` setting;
/// anything else has no way to.
fn loader_cell(agent: &Agent) -> Cell {
    if !agent.active() {
        return absent(agent.home.clone(), State::Absent);
    }
    match agent.name.as_str() {
        "pi" => {
            let file = agent.home.join("extensions").join("canonize.ts");
            let state = if file.is_file() {
                State::Linked
            } else {
                State::Missing
            };
            Cell {
                change: (state == State::Missing).then(|| Change::WriteFile {
                    file: file.clone(),
                    body: PI_EXTENSION.to_string(),
                    what: "the pi extension that loads CANON.md".into(),
                }),
                undo: (state == State::Linked).then(|| Change::DeleteFile {
                    file: file.clone(),
                    what: "the pi extension canonize wrote".into(),
                }),
                at: file,
                state,
            }
        }
        "opencode" => {
            let file = agent.home.join("opencode.json");
            let has = fs::read_to_string(&file)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v.get("instructions").cloned())
                .and_then(|v| v.as_array().cloned())
                .is_some_and(|l| l.iter().any(|x| x.as_str() == Some(CANON_FILE)));
            let state = if has { State::Linked } else { State::Missing };
            Cell {
                change: (!has).then(|| Change::JsonInstruction {
                    file: file.clone(),
                    value: CANON_FILE.into(),
                }),
                undo: has.then(|| Change::JsonUninstruction {
                    file: file.clone(),
                    value: CANON_FILE.into(),
                }),
                at: file,
                state,
            }
        }
        _ => absent(agent.home.clone(), State::Na),
    }
}

fn skill_cell(cfg: &Config, agent: &Agent, name: &str) -> Cell {
    if !agent.active() {
        return absent(agent.skills.join(name), State::Absent);
    }
    match agent.skills_mode {
        SkillsMode::Off => absent(agent.skills.join(name), State::Off),
        SkillsMode::Folder => link_cell(&agent.skills, &cfg.source.skills, &cfg.source.root),
        SkillsMode::PerSkill => {
            if is_link(&agent.skills) {
                return absent(
                    agent.skills.clone(),
                    State::Foreign(format!(
                        "`{}` is a link, but this agent links skills one by one",
                        tilde(&agent.skills)
                    )),
                );
            }
            link_cell(
                &agent.skills.join(name),
                &cfg.source.skills.join(name),
                &cfg.source.root,
            )
        }
    }
}

/// What sits in a per-skill folder besides the source's skills: the foreign
/// names, and removals for links into the source whose skill is gone.
fn leftovers(cfg: &Config, agent: &Agent, skills: &[String]) -> (Vec<String>, Vec<Change>) {
    if !agent.active() || agent.skills_mode != SkillsMode::PerSkill || is_link(&agent.skills) {
        return (Vec::new(), Vec::new());
    }
    let Ok(entries) = fs::read_dir(&agent.skills) else {
        return (Vec::new(), Vec::new());
    };
    let mut other = Vec::new();
    let mut gone = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || skills.contains(&name) {
            continue;
        }
        let at = e.path();
        match link_target(&at) {
            Some(dest) if dest.starts_with(&cfg.source.root) => gone.push(Change::Unlink { at }),
            _ => other.push(name),
        }
    }
    other.sort();
    (other, gone)
}

fn is_link(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

/// Where a symlink points, made absolute against its own folder.
fn link_target(at: &Path) -> Option<PathBuf> {
    let dest = fs::read_link(at).ok()?;
    Some(if dest.is_absolute() {
        dest
    } else {
        at.parent().unwrap_or(Path::new("/")).join(dest)
    })
}

fn same(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (fs::canonicalize(a), fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// `at` should be a symlink to `to`, a path inside `root`.
fn link_cell(at: &Path, to: &Path, root: &Path) -> Cell {
    let cell = |state, change, undo| Cell {
        state,
        at: at.to_path_buf(),
        change,
        undo,
    };
    let unlink = Some(Change::Unlink {
        at: at.to_path_buf(),
    });
    match fs::symlink_metadata(at) {
        Err(_) => cell(
            State::Missing,
            Some(Change::Link {
                at: at.to_path_buf(),
                to: to.to_path_buf(),
            }),
            None,
        ),
        Ok(m) if m.file_type().is_symlink() => {
            let dest = link_target(at).unwrap_or_default();
            if same(&dest, to) {
                if to.exists() {
                    cell(State::Linked, None, unlink)
                } else {
                    cell(
                        State::Broken(format!("`{}` is not in the source", tilde(to))),
                        None,
                        unlink,
                    )
                }
            } else if dest.starts_with(root) {
                cell(
                    State::Broken(format!("points to `{}`", tilde(&dest))),
                    Some(Change::Relink {
                        at: at.to_path_buf(),
                        to: to.to_path_buf(),
                    }),
                    unlink,
                )
            } else {
                cell(
                    State::Foreign(format!("a link to `{}`", tilde(&dest))),
                    None,
                    None,
                )
            }
        }
        Ok(m) if m.is_dir() => cell(State::Foreign("a real folder".into()), None, None),
        Ok(_) => cell(State::Foreign("a real file".into()), None, None),
    }
}

/// `file` should carry an `@path` line naming `rules`.
fn import_cell(file: &Path, rules: &Path, root: &Path) -> Cell {
    let line = format!("@{}", tilde(rules));
    let cell = |state, change, undo| Cell {
        state,
        at: file.to_path_buf(),
        change,
        undo,
    };
    let add = Some(Change::AddImport {
        file: file.to_path_buf(),
        line: line.clone(),
    });
    // A file that is a link into the canon is the rules file seen from
    // somewhere else: writing an import into it would edit the rules.
    if let Ok(dest) = fs::read_link(file) {
        let dest = if dest.is_absolute() {
            dest
        } else {
            file.parent().unwrap_or(Path::new("/")).join(dest)
        };
        if dest.starts_with(root) {
            return cell(
                State::Foreign(format!(
                    "a link to `{}`, so an import would be written into your canon",
                    tilde(&dest)
                )),
                None,
                None,
            );
        }
    }
    let Ok(text) = fs::read_to_string(file) else {
        return cell(State::Missing, add, None);
    };
    let remove = Some(Change::RemoveImport {
        file: file.to_path_buf(),
        line: line.clone(),
    });
    if text.lines().any(|l| l.trim() == line) {
        return if rules.exists() {
            cell(State::Linked, None, remove)
        } else {
            cell(
                State::Broken(format!("`{}` is not in the source", tilde(rules))),
                None,
                remove,
            )
        };
    }
    // An import of a file that is gone, from inside the canon or under the
    // rules file's own name (the canon moved), is a rename the agent has not
    // caught up with, so it is replaced rather than added beside. One that
    // still exists (a house file imported globally) is the user's and stays.
    let old = text.lines().map(str::trim).find(|l| {
        l.strip_prefix('@').is_some_and(|p| {
            let p = config::expand(p);
            (p.starts_with(root) || p.file_name() == rules.file_name()) && !p.exists()
        })
    });
    match old {
        Some(old) => cell(
            State::Broken(format!("imports `{}`", &old[1..])),
            Some(Change::ReplaceImport {
                file: file.to_path_buf(),
                old: old.to_string(),
                new: line,
            }),
            None,
        ),
        None => cell(State::Missing, add, None),
    }
}
