//! The plain commands: the same engines the TUI drives, printed for a person
//! or a script.

use crate::check;
use crate::config::{self, Config, RulesMode, SkillsMode, tilde};
use crate::init;
use crate::plan::{self, Change, Plan, Row, State};
use crate::projects::{self, Projects};
use crate::setup;
use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::fs;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct AgentArgs {
    /// Only this agent (as named in `canon status`)
    #[arg(short, long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Also exit 1 when something needs a person (foreign, own)
    #[arg(long)]
    pub strict: bool,
    /// Print it as JSON for a script or an agent
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct JsonArgs {
    /// Print it as JSON for a script or an agent
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct ChangeArgs {
    /// Only this agent (as named in `canon status`)
    #[arg(short, long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Dry run: print what would change and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct SetupArgs {
    /// Dry run: print what it found and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// The folder your canon lives in
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,
    /// Your rules file inside it ("" for none)
    #[arg(long, value_name = "NAME")]
    pub rules: Option<String>,
    /// The schema `canon validate` holds them to ("" for none)
    #[arg(long, value_name = "NAME")]
    pub schema: Option<String>,
    /// Your house files, one `*` allowed in the name ("" for none)
    #[arg(long, value_name = "PATTERN")]
    pub house: Option<String>,
    /// The folder your skills live in
    #[arg(long, value_name = "DIR")]
    pub skills: Option<String>,
    /// A folder to look for projects in, once per folder ("" for none)
    #[arg(long, value_name = "DIR")]
    pub projects: Vec<String>,
}

#[derive(clap::Args)]
pub struct AdoptArgs {
    /// The agent whose skill it is (as named in `canon status`)
    #[arg(required_unless_present = "all")]
    pub agent: Option<String>,
    /// The skill's folder name, which `canon status` prints after `skill`
    #[arg(required_unless_present = "all")]
    pub skill: Option<String>,
    /// Every skill that agent keeps itself, or every agent's when none is named
    #[arg(long)]
    pub all: bool,
    /// Dry run: print what would change and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct EvictArgs {
    /// The folder in your canon's skills folder that is not a skill
    #[arg(required_unless_present = "all")]
    pub name: Option<String>,
    /// The agent to move it to, which keeps it in its own skills folder
    pub agent: String,
    /// Every folder in your canon that is not a skill
    #[arg(long)]
    pub all: bool,
    /// Delete your canon's copy instead, when the agent already has one
    #[arg(long)]
    pub drop: bool,
    /// Dry run: print what would change and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct MoveArgs {
    /// `canon` for the whole folder, or the name of something inside it
    pub what: String,
    /// Where it goes: the new folder, or a folder inside your canon
    pub to: String,
    /// Dry run: print what would change and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Subcommand)]
pub enum ConfigCmd {
    /// Set one setting, keeping every comment in the file
    ///   canon config set agents.pi.skills_mode per-skill
    ///   canon config set projects ~/dev ~/work
    #[command(verbatim_doc_comment)]
    Set(ConfigSetArgs),
}

#[derive(clap::Args)]
pub struct ConfigSetArgs {
    /// `projects`, `source.<name>` or `agents.<agent>.<name>`
    pub key: String,
    /// What to set it to, or several values for a list
    #[arg(required = true)]
    pub value: Vec<String>,
    /// Dry run: print the line it would write and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct InitArgs {
    /// Folder to lay out (default: `$CANONIZE_SOURCE`, else your config folder)
    pub dir: Option<PathBuf>,
}

/// The index of `--agent`, or an error naming the ones that exist.
fn pick(cfg: &Config, name: &Option<String>) -> Result<Option<usize>> {
    let Some(name) = name else { return Ok(None) };
    match cfg.agents.iter().position(|a| &a.name == name) {
        Some(i) => Ok(Some(i)),
        None => {
            let known: Vec<&str> = cfg.agents.iter().map(|a| a.name.as_str()).collect();
            bail!("no agent `{name}`: try one of `{}`", known.join("`, `"))
        }
    }
}

fn paint(state: &State) -> String {
    let w = state.word();
    match state {
        State::Linked => w.green().to_string(),
        State::Missing => w.normal().to_string(),
        State::Broken(_) => w.yellow().to_string(),
        State::Foreign(_) => w.magenta().to_string(),
        State::Own => w.cyan().to_string(),
        State::Off | State::Absent | State::Na => w.dimmed().to_string(),
    }
}

/// Exit code 1 when anything has drifted, so `canon status` can gate a script;
/// `--strict` counts what needs a person too.
pub fn status(args: AgentArgs) -> Result<i32> {
    let cfg = config::load()?;
    let only = pick(&cfg, &args.agent)?;
    let plan = Plan::build(&cfg);
    let projects = Projects::build(&cfg);
    let unsettled = plan.unsettled(only);
    // With `-a`, only that agent's drift counts: the projects table is not
    // printed either, so a gate on one agent is about that agent alone.
    let unfixed = plan.drifted(only) || (only.is_none() && projects.drifted());
    let code = i32::from(unfixed || (args.strict && !unsettled.is_empty()));
    if args.json {
        out!(
            "{}",
            serde_json::to_string_pretty(&status_json(&cfg, &plan, &projects, only, code))?
        );
        return Ok(code);
    }
    let cols: Vec<usize> = (0..cfg.agents.len())
        .filter(|i| only.is_none_or(|o| o == *i))
        .collect();

    out!("{} {}", "canon".dimmed(), tilde(&cfg.source.root));
    let label_w = plan
        .rows
        .iter()
        .map(|r| r.label().chars().count())
        .max()
        .unwrap_or(5)
        .max(5);
    let col_w: usize = 10;
    let mut head = format!("{:label_w$}", "");
    for &i in &cols {
        let a = &cfg.agents[i];
        let name = if a.active() {
            a.name.bold().to_string()
        } else {
            a.name.dimmed().to_string()
        };
        head.push_str(&format!(
            "  {}{}",
            name,
            " ".repeat(col_w.saturating_sub(a.name.len()))
        ));
    }
    out!("{}", head.trim_end());
    for (r, row) in plan.rows.iter().enumerate() {
        let mut line = format!("{:label_w$}", row.label());
        for &i in &cols {
            let s = &plan.cells[r][i].state;
            line.push_str(&format!(
                "  {}{}",
                paint(s),
                " ".repeat(col_w - s.word().len())
            ));
        }
        out!("{}", line.trim_end());
    }

    let mut notes = Vec::new();
    for &i in &cols {
        let a = &cfg.agents[i];
        if !a.installed() {
            notes.push(format!("{}: not installed (no {})", a.name, tilde(&a.home)));
        } else if !a.enabled {
            notes.push(format!("{}: disabled in {}", a.name, config::CONFIG_FILE));
        }
        // One reason shared by several rows is one fact about the agent, not one
        // per row: a skills folder that is a link breaks every skill at once.
        let broken: Vec<(String, &String)> = plan
            .rows
            .iter()
            .enumerate()
            .filter_map(|(r, row)| match &plan.cells[r][i].state {
                State::Broken(why) => Some((row.label(), why)),
                _ => None,
            })
            .collect();
        for (label, why) in &broken {
            let shared = broken.iter().filter(|(_, other)| other == why).count() > 1;
            if shared {
                if !notes.iter().any(|n| n.ends_with(why.as_str())) {
                    notes.push(format!("{}: {why}", a.name));
                }
            } else {
                notes.push(format!("{} {label}: {why}", a.name));
            }
        }
        if !plan.foreign[i].is_empty() {
            notes.push(format!(
                "{} keeps these in its skills folder and canonize leaves them alone, since they hold no SKILL.md: {}",
                a.name,
                plan.foreign[i].join(", ")
            ));
        }
    }
    for c in plan.stale_for(only) {
        notes.push(format!("stale: {}", c.describe()));
    }
    if only.is_none() && !cfg.projects.is_empty() {
        print_projects(&cfg, &projects, &mut notes);
    }
    if !notes.is_empty() {
        out!();
        for n in notes {
            out!("{}", n.dimmed());
        }
    }
    // One block for everything `fix` will not decide: a cell it leaves alone,
    // and an agent writing its own tree into the canon.
    let mut needs: Vec<(String, String)> = unsettled
        .iter()
        .map(|&(r, a)| {
            (
                format!("{} {}", cfg.agents[a].name, plan.rows[r].name()),
                advice(&cfg, &plan, r, a),
            )
        })
        .collect();
    needs.extend(
        writes_into_canon(&cfg, only)
            .map(|(names, advice)| (format!("{} skills", names.join(", ")), advice)),
    );
    if !needs.is_empty() {
        out!();
        out!(
            "{}",
            format!(
                "needs you ({}), because `canon fix` never touches these:",
                needs.len()
            )
            .bold()
        );
        let w = needs
            .iter()
            .map(|(who, _)| who.chars().count())
            .max()
            .unwrap_or(0);
        for (who, what) in needs {
            out!("  {who:w$}  {what}");
        }
    }
    Ok(code)
}

/// What a person can do about a cell `fix` leaves alone.
fn advice(cfg: &Config, plan: &Plan, r: usize, a: usize) -> String {
    let agent = &cfg.agents[a];
    let cell = &plan.cells[r][a];
    let name = plan.rows[r].name();
    match (&plan.rows[r], &cell.state) {
        (Row::Skill(_), State::Own) => format!(
            "only {} has it: `canon adopt {} {name}` moves it into your canon",
            agent.name, agent.name
        ),
        (Row::Rules(_), State::Foreign(why)) if agent.rules_mode == RulesMode::Link => format!(
            "{why}: `rules_mode = \"import\"` under `[agents.{}]` in {} writes your rules into it instead, or move it aside and run `canon fix`",
            agent.name,
            config::CONFIG_FILE
        ),
        (_, State::Foreign(why)) => {
            format!("{why}: move it aside yourself, then run `canon fix`")
        }
        (_, state) => state.word().to_string(),
    }
}

/// An agent whose skills folder is the canon's writes everything it makes into
/// the canon, which is how a bucket of its own lands in the user's dotfiles.
/// Worth saying only once something that is not a skill is sitting there, and
/// as one line however many agents share the folder.
fn writes_into_canon(cfg: &Config, only: Option<usize>) -> Option<(Vec<String>, String)> {
    let strays = config::strays(&cfg.source);
    if strays.is_empty() {
        return None;
    }
    let names: Vec<String> = cfg
        .agents
        .iter()
        .enumerate()
        .filter(|(i, _)| only.is_none_or(|o| o == *i))
        .filter(|(_, a)| a.active() && a.skills_mode == SkillsMode::Folder)
        .map(|(_, a)| a.name.clone())
        .collect();
    let (who, writes, whose, under) = match names.as_slice() {
        [] => return None,
        [name] => ("it", "writes", "its", format!("`[agents.{name}]`")),
        _ => ("they", "write", "their", "each of them".to_string()),
    };
    let strays: Vec<&str> = strays.iter().map(|(name, _)| name.as_str()).collect();
    Some((
        names,
        format!(
            "the skills folder is your canon's, so what {who} {writes} lands in it ({}): `skills_mode = \"per-skill\"` under {under} in {} keeps it on {whose} side",
            strays.join(", "),
            config::CONFIG_FILE
        ),
    ))
}

/// Everything `canon status` prints, as JSON: the same states under names that
/// do not change, and absolute paths.
fn status_json(
    cfg: &Config,
    plan: &Plan,
    projects: &Projects,
    only: Option<usize>,
    code: i32,
) -> serde_json::Value {
    use serde_json::{Value, json};
    let cols: Vec<usize> = (0..cfg.agents.len())
        .filter(|i| only.is_none_or(|o| o == *i))
        .collect();
    let agents: Vec<Value> = cols
        .iter()
        .map(|&i| {
            let a = &cfg.agents[i];
            json!({
                "name": a.name,
                "installed": a.installed(),
                "enabled": a.enabled,
                "home": a.home,
            })
        })
        .collect();
    let rows: Vec<Value> = plan
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells: serde_json::Map<String, Value> = cols
                .iter()
                .map(|&i| {
                    let cell = &plan.cells[r][i];
                    (
                        cfg.agents[i].name.clone(),
                        json!({
                            "state": cell.state.id(),
                            "why": cell.state.why(),
                            "at": cell.at,
                            "fix": cell.change.as_ref().map(Change::describe),
                        }),
                    )
                })
                .collect();
            json!({ "kind": row.kind(), "name": row.name(), "cells": cells })
        })
        .collect();
    let leftovers: serde_json::Map<String, Value> = cols
        .iter()
        .filter(|&&i| !plan.foreign[i].is_empty())
        .map(|&i| (cfg.agents[i].name.clone(), json!(plan.foreign[i])))
        .collect();
    let projects_json: Vec<Value> = projects
        .list
        .iter()
        .map(|p| {
            let house: serde_json::Map<String, Value> = projects
                .house
                .iter()
                .zip(&p.cells)
                .map(|(h, c)| {
                    (
                        house_label(h),
                        json!({ "state": c.state.id(), "why": c.state.why() }),
                    )
                })
                .collect();
            let wiring: Vec<Value> = p
                .wiring
                .iter()
                .map(|w| json!({ "what": w.what, "state": w.state.id() }))
                .collect();
            json!({
                "path": p.root,
                "name": projects::short(cfg, &p.root),
                "house": house,
                "wiring": wiring,
                "dead": p.dead,
            })
        })
        .collect();
    let unsettled: Vec<Value> = plan
        .unsettled(only)
        .into_iter()
        .map(|(r, a)| {
            json!({
                "agent": cfg.agents[a].name,
                "kind": plan.rows[r].kind(),
                "name": plan.rows[r].name(),
                "state": plan.cells[r][a].state.id(),
                "why": plan.cells[r][a].state.why(),
                "advice": advice(cfg, plan, r, a),
            })
        })
        .collect();
    json!({
        "canon": cfg.source.root,
        "config": cfg.path,
        "agents": agents,
        "rows": rows,
        "leftovers": leftovers,
        "stale": plan.stale_for(only).into_iter().map(Change::describe).collect::<Vec<_>>(),
        "projects": projects_json,
        "unsettled": unsettled,
        "strays": config::strays(&cfg.source)
            .into_iter()
            .map(|(name, skills)| json!({ "name": name, "skills": skills }))
            .collect::<Vec<Value>>(),
        "writes_into_canon": writes_into_canon(cfg, only).map(|(agents, advice)| json!({
            "agents": agents, "advice": advice
        })),
        "drifted": plan.drifted(only) || (only.is_none() && projects.drifted()),
        "exit": code,
    })
}

/// A house file's column name: its name without the pattern's common parts.
pub fn house_label(path: &std::path::Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    stem.strip_prefix("HOUSE-").unwrap_or(&stem).to_lowercase()
}

fn print_projects(cfg: &Config, p: &Projects, notes: &mut Vec<String>) {
    out!();
    let label_w = p
        .list
        .iter()
        .map(|x| projects::short(cfg, &x.root).chars().count())
        .max()
        .unwrap_or(8)
        .max(8);
    let cols: Vec<String> = p.house.iter().map(|h| house_label(h)).collect();
    // `dimmed` adds escape codes, so the padding is worked out on the bare word.
    let head = format!(
        "{}{}",
        "projects".dimmed(),
        " ".repeat(label_w.saturating_sub(8))
    );
    let mut line = head;
    for c in &cols {
        line.push_str(&format!(
            "  {}{}",
            c.bold(),
            " ".repeat(10usize.saturating_sub(c.chars().count()))
        ));
    }
    out!("{}", line.trim_end());
    for x in &p.list {
        let mut line = format!("{:label_w$}", projects::short(cfg, &x.root));
        for c in &x.cells {
            let (word, painted) = match &c.state {
                State::Linked => ("imported", "imported".green().to_string()),
                State::Broken(_) => ("broken", "broken".yellow().to_string()),
                _ => ("-", "-".dimmed().to_string()),
            };
            line.push_str(&format!("  {painted}{}", " ".repeat(10 - word.len())));
        }
        out!("{}", line.trim_end());
        for (c, h) in x.cells.iter().zip(&cols) {
            if let State::Broken(why) = &c.state {
                notes.push(format!("{} {h}: {why}", projects::short(cfg, &x.root)));
            }
        }
        if x.uses_canon() {
            for c in x.wiring_changes() {
                notes.push(format!(
                    "{}: {}",
                    projects::short(cfg, &x.root),
                    c.describe()
                ));
            }
        }
        for d in &x.dead {
            notes.push(format!(
                "{}: `{d}` names a file that does not exist",
                projects::short(cfg, &x.root)
            ));
        }
    }
}

fn run_changes(changes: Vec<Change>, dry_run: bool, nothing: &str) -> i32 {
    if changes.is_empty() {
        out!("{}", nothing.dimmed());
        return 0;
    }
    let mut failed = 0;
    for c in changes {
        if dry_run {
            out!("would {}", c.describe());
            continue;
        }
        match c.run() {
            Ok(()) => out!("{} {}", "✓".green(), c.describe()),
            Err(e) => {
                failed += 1;
                eprintln!("{} {}: {e:#}", "✗".red(), c.describe());
            }
        }
    }
    if failed > 0 { 1 } else { 0 }
}

pub fn fix(args: ChangeArgs) -> Result<i32> {
    let cfg = config::load()?;
    let only = pick(&cfg, &args.agent)?;
    let plan = Plan::build(&cfg);
    let changes = match only {
        Some(i) => plan.changes_for(i),
        None => {
            let mut all = plan.changes();
            all.extend(Projects::build(&cfg).fixes());
            all
        }
    };
    Ok(run_changes(
        changes,
        args.dry_run,
        "nothing to fix: every agent and project is up to date",
    ))
}

pub fn delete(args: ChangeArgs) -> Result<i32> {
    let cfg = config::load()?;
    let only = pick(&cfg, &args.agent)?;
    let plan = Plan::build(&cfg);
    let mut changes = plan.undos(only);
    // Without `-a` it takes back everything, each project's CANON.md wiring
    // included, which is what `--help` promises.
    if only.is_none() {
        for p in &Projects::build(&cfg).list {
            changes.extend(p.undos());
        }
    }
    Ok(run_changes(changes, args.dry_run, "nothing to delete"))
}

pub fn validate(args: JsonArgs) -> Result<i32> {
    let cfg = config::load()?;
    let report = check::run(&cfg.source);
    let code = if report.problems.is_empty() { 0 } else { 1 };
    if args.json {
        out!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "canon": cfg.source.root,
                "valid": report.problems.is_empty(),
                "problems": report.problems,
                "rules": report.rules,
                "skills": report.skills,
                "house": report.house,
                "exit": code,
            }))?
        );
        return Ok(code);
    }
    for p in &report.problems {
        out!("{p}");
    }
    if report.problems.is_empty() {
        out!("valid: {}", check::counts(&report));
    }
    Ok(code)
}

/// Move what is not a skill out of the canon, into the folder of the agent that
/// wrote it there.
pub fn evict(args: EvictArgs) -> Result<i32> {
    let cfg = config::load()?;
    let i = pick(&cfg, &Some(args.agent.clone()))?.unwrap_or(0);
    let agent = &cfg.agents[i];
    if agent.skills.as_os_str().is_empty() {
        bail!("`{}` has no skills folder to move it to", agent.name);
    }
    if fs::symlink_metadata(&agent.skills)
        .is_ok_and(|m| m.file_type().is_symlink())
        .then(|| fs::canonicalize(&agent.skills).unwrap_or_default())
        .is_some_and(|dest| dest.starts_with(&cfg.source.root))
    {
        bail!(
            "`{}` is a link into your canon, so moving it there would move nothing: set `skills_mode = \"per-skill\"` for `{}` first",
            tilde(&agent.skills),
            agent.name
        );
    }
    let strays = config::strays(&cfg.source);
    let names: Vec<String> = match &args.name {
        Some(name) => {
            if !strays.iter().any(|(n, _)| n == name) {
                bail!(
                    "`{}` is not there, or it is a skill of your canon, which `canon adopt` and `canon fix` own",
                    tilde(&cfg.source.skills.join(name))
                );
            }
            vec![name.clone()]
        }
        None => strays.iter().map(|(n, _)| n.clone()).collect(),
    };
    let mut changes = Vec::new();
    for name in &names {
        let from = cfg.source.skills.join(name);
        let to = agent.skills.join(name);
        // The agent having its own copy already is the ordinary end of this:
        // it wrote one the moment it ran again, and the canon's is the leftover.
        match (to.exists(), args.drop) {
            (true, true) => changes.push(Change::DeleteDir { at: from }),
            (true, false) => bail!(
                "`{}` already has `{name}`, so nothing was moved: `--drop` deletes your canon's copy instead, since the agent's is the live one",
                tilde(&agent.skills)
            ),
            (false, _) => changes.push(Change::Evict { from, to }),
        }
    }
    Ok(run_changes(
        changes,
        args.dry_run,
        "nothing to evict: every folder in your canon's skills folder is a skill",
    ))
}

/// Move the canon, or something in it, and repoint everything that named the
/// old path: imports in every agent and project file, links into the canon, the
/// shortcut `canon` finds the folder by, and absolute paths in canonize.toml.
pub fn move_it(args: MoveArgs) -> Result<i32> {
    let cfg = config::load()?;
    let (from, to) = targets(&cfg, &args.what, &args.to)?;
    let (rename, repoints) = plan_move(&cfg, &from, &to)?;
    // The move runs first and alone: a repoint written after a rename that
    // failed would name a path nothing is at.
    if !rename.is_empty() {
        let code = run_changes(rename, args.dry_run, "");
        if code != 0 {
            return Ok(code);
        }
    } else {
        out!(
            "{}",
            format!("{} is already at {}", tilde(&from), tilde(&to)).dimmed()
        );
    }
    // Repointing is worked out from what the files say now, so running the same
    // command again finishes a move that stopped halfway.
    let code = run_changes(repoints, args.dry_run, "nothing else named the old path");
    // The shortcut is repointed above, but an environment variable is the
    // user's shell, which nothing here can reach.
    let named_by_env = std::env::var_os("CANONIZE_SOURCE")
        .is_some_and(|var| config::expand(&var.to_string_lossy()) == from);
    if code == 0 && !args.dry_run && named_by_env {
        eprintln!(
            "{}",
            format!("set CANONIZE_SOURCE to {} for the next run", tilde(&to)).dimmed()
        );
    }
    Ok(code)
}

/// What moves and where, from the two words the user typed: `canon` for the
/// folder as the config knows it, a path for one the config no longer knows
/// (a move that stopped halfway), or a name inside the canon.
fn targets(cfg: &Config, what: &str, to: &str) -> Result<(PathBuf, PathBuf)> {
    if what == "canon" {
        return Ok((cfg.source.root.clone(), config::expand(to)));
    }
    if what.starts_with('~') || what.starts_with('/') {
        let from = config::expand(what);
        let to = if to.starts_with('~') || to.starts_with('/') {
            config::expand(to)
        } else {
            cfg.source.root.join(to)
        };
        return Ok((from, to));
    }
    let from = cfg.source.root.join(what);
    let name = std::path::Path::new(what)
        .file_name()
        .map(|n| n.to_os_string())
        .with_context(|| format!("`{what}` has no name"))?;
    let dir = if to.starts_with('~') || to.starts_with('/') {
        config::expand(to)
    } else {
        cfg.source.root.join(to)
    };
    Ok((from, dir.join(name)))
}

/// The move itself, then everything that has to be repointed after it. The
/// move is empty when it already happened, so the same command can be run
/// again to repoint what is left.
fn plan_move(
    cfg: &Config,
    from: &std::path::Path,
    to: &std::path::Path,
) -> Result<(Vec<Change>, Vec<Change>)> {
    if from == to {
        bail!("`{}` is where it already is", tilde(from));
    }
    let there = fs::symlink_metadata(from).is_ok();
    let landed = fs::symlink_metadata(to).is_ok();
    if there && landed {
        bail!(
            "`{}` already exists, so nothing was moved: delete it, or name somewhere else",
            tilde(to)
        );
    }
    if !there && !landed {
        bail!(
            "neither `{}` nor `{}` is there, so there is nothing to move",
            tilde(from),
            tilde(to)
        );
    }
    let rename = if there {
        vec![Change::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        }]
    } else {
        Vec::new()
    };
    Ok((rename, repoint(cfg, from, to)))
}

/// Everything that names `from`, rewritten to `to`: `@` imports, links into it,
/// and the settings that spell a path out.
fn repoint(cfg: &Config, from: &std::path::Path, to: &std::path::Path) -> Vec<Change> {
    let under = |p: &std::path::Path| -> Option<PathBuf> {
        if p == from {
            Some(to.to_path_buf())
        } else {
            p.strip_prefix(from).ok().map(|rest| to.join(rest))
        }
    };
    // A file inside what moves is read where it was and written where it lands.
    let after = |p: &std::path::Path| under(p).unwrap_or_else(|| p.to_path_buf());

    let mut files: Vec<PathBuf> = cfg
        .agents
        .iter()
        .filter(|a| a.active())
        .map(|a| a.rules.clone())
        .collect();
    for p in &Projects::build(cfg).list {
        for name in ["CLAUDE.md", "AGENTS.md", plan::CANON_FILE] {
            files.push(p.root.join(name));
        }
    }
    files.push(cfg.path.clone());
    files.sort();
    files.dedup();

    let mut out = Vec::new();
    for file in files {
        // Writing through a link would edit the file it points at, which is
        // usually the rules file itself.
        if fs::symlink_metadata(&file).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let is_config = file == cfg.path;
        for line in text.lines().map(str::trim) {
            let new = if is_config {
                // A setting spelling a path out, such as `skills = "~/…"`.
                line.split_once('=')
                    .map(|(_, value)| config::expand(value.trim().trim_matches(['"', '\''])))
                    .and_then(|p| under(&p))
                    .map(|p| {
                        line.replace(
                            &line[line.find('"').unwrap_or(0)..],
                            &format!("\"{}\"", tilde(&p)),
                        )
                    })
            } else {
                line.strip_prefix('@')
                    .filter(|p| !p.is_empty() && !p.contains(' '))
                    .map(config::expand)
                    .and_then(|p| under(&p))
                    .map(|p| format!("@{}", tilde(&p)))
            };
            if let Some(new) = new {
                out.push(Change::ReplaceImport {
                    file: after(&file),
                    old: line.to_string(),
                    new,
                });
            }
        }
    }

    // Links: an agent's rules file or skills folder, what is inside a skills
    // folder, and the shortcut `canon` finds the canon by.
    let mut links: Vec<PathBuf> = vec![config::source_dir()];
    for a in cfg.agents.iter().filter(|a| a.active()) {
        links.push(a.rules.clone());
        links.push(a.skills.clone());
        if let Ok(entries) = fs::read_dir(&a.skills) {
            links.extend(entries.flatten().map(|e| e.path()));
        }
    }
    links.sort();
    links.dedup();
    for at in links {
        let Some(dest) = plan::link_target(&at) else {
            continue;
        };
        if let Some(new) = under(&dest) {
            out.push(Change::Relink {
                at: after(&at),
                to: new,
            });
        }
    }
    out
}

pub fn config(cmd: ConfigCmd) -> Result<i32> {
    let ConfigCmd::Set(args) = cmd;
    let cfg = config::load()?;
    // A list, a bool and a number are written as they are; everything else is
    // text, so it goes in quotes.
    let literal = if args.key == "projects" || args.value.len() > 1 {
        let items: Vec<String> = args.value.iter().map(|v| format!("\"{v}\"")).collect();
        format!("[{}]", items.join(", "))
    } else {
        let value = &args.value[0];
        match value.as_str() {
            "true" | "false" => value.clone(),
            _ if value.parse::<i64>().is_ok() => value.clone(),
            _ => format!("\"{value}\""),
        }
    };
    // The import of the rules file is canonize's own line, and once the setting
    // changes nothing recognises it, so it is taken back before the switch.
    let old_rules: Vec<Change> = if args.key == "source.rules" {
        let plan = Plan::build(&cfg);
        plan.rows
            .iter()
            .position(|r| matches!(r, Row::Rules(_)))
            .map(|r| {
                plan.cells[r]
                    .iter()
                    .filter_map(|c| c.undo.clone())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let line = config::set_key(&cfg.path, &args.key, &literal, args.dry_run)?;
    if args.dry_run {
        out!("would write `{line}` in {}", tilde(&cfg.path));
    } else {
        out!("{} {line} in {}", "✓".green(), tilde(&cfg.path));
    }
    let code = run_changes(old_rules, args.dry_run, "");
    if code == 0 {
        out!(
            "{}",
            "run `canon fix` to wire what the new setting asks for".dimmed()
        );
    }
    Ok(code)
}

pub fn init(args: InitArgs) -> Result<i32> {
    let root = match args.dir {
        Some(d) => config::expand(&d.to_string_lossy()),
        None => config::source_dir(),
    };
    for (path, created) in init::run(&root)? {
        if created {
            out!("{} {path}", "created".green());
        } else {
            out!("{} {path}", "exists ".dimmed());
        }
    }
    if root != config::source_dir() {
        eprintln!(
            "{}",
            format!(
                "canon looks in {} by default: set `CANONIZE_SOURCE={}` or link that folder here",
                tilde(&config::source_dir()),
                tilde(&root)
            )
            .dimmed()
        );
    }
    Ok(0)
}

/// The answers `setup` starts from when nothing was detected: whatever the
/// folder the user named already holds.
fn from_root(root: &std::path::Path) -> setup::Choice {
    let first = |names: Vec<String>| names.into_iter().next();
    setup::Choice {
        root: root.to_path_buf(),
        rules: first(setup::rule_files(root)).map_or(setup::Pick::New, setup::Pick::Existing),
        schema: first(setup::schema_files(root)).map_or(setup::Pick::None, setup::Pick::Existing),
        house: setup::house_patterns(root).first().map(|(p, _)| p.clone()),
        skills: "skills".to_string(),
        projects: projects::guess(),
    }
}

/// A file named on the command line: one that is there, the starter when it is
/// the default name, or none.
fn pick_file(root: &std::path::Path, value: &str, default: &str) -> Result<setup::Pick> {
    if value.is_empty() {
        return Ok(setup::Pick::None);
    }
    if root.join(value).is_file() {
        return Ok(setup::Pick::Existing(value.to_string()));
    }
    if value == default {
        return Ok(setup::Pick::New);
    }
    bail!(
        "no `{value}` in `{}`: name a file that is there, `{default}` to start a new one, or \"\" for none",
        tilde(root)
    )
}

pub fn setup(args: SetupArgs) -> Result<i32> {
    if config::load().is_ok() {
        out!(
            "{}",
            format!("already set up in {}", tilde(&config::source_dir())).dimmed()
        );
        return Ok(0);
    }
    // The two ways a canon turns up read differently, so they are kept apart:
    // an agent reads the file today, or it imports a path that is gone and a
    // file by that name sits in a folder next door.
    let read_now = setup::detect();
    let moved = match read_now {
        Some(_) => None,
        None => setup::lost(),
    };
    let found = read_now.or_else(|| {
        let moved = moved.as_ref()?;
        setup::around(moved.now.as_ref()?, &moved.by)
    });
    let mut choice = match (&found, &args.root) {
        (Some(found), None) => found.choice(),
        (_, Some(root)) => from_root(&config::expand(&root.to_string_lossy())),
        (None, None) => bail!(
            "none of your agents reads a rules file yet: run `canon setup --root DIR` to set it up around a folder you already have, or `canon init` to lay out a new canon"
        ),
    };
    if let (Some(found), None) = (&found, &args.root) {
        match &moved {
            Some(moved) => out!(
                "found {} ({} imports {}, which is gone, so your canon most likely moved here)",
                tilde(&found.rules),
                tilde(&moved.by),
                tilde(&moved.path)
            ),
            None => out!(
                "found {} (read by {})",
                tilde(&found.rules),
                tilde(&found.by)
            ),
        }
    }
    if let Some(name) = &args.rules {
        choice.rules = pick_file(&choice.root, name, "rules.yaml")?;
    }
    if let Some(name) = &args.schema {
        choice.schema = pick_file(&choice.root, name, "rules.schema.json")?;
    }
    if let Some(pattern) = &args.house {
        choice.house = (!pattern.is_empty()).then(|| pattern.clone());
    }
    if let Some(dir) = &args.skills {
        choice.skills = dir.clone();
    }
    if !args.projects.is_empty() {
        choice.projects = args
            .projects
            .iter()
            .filter(|p| !p.is_empty())
            .cloned()
            .collect();
    }
    out!();
    for line in choice.summary() {
        out!("{line}");
    }
    if args.dry_run {
        out!(
            "\n{}",
            "nothing written: run the same without -n to write it".dimmed()
        );
    } else {
        setup::apply(&choice)?;
        out!("\n{} set up", "✓".green());
    }
    Ok(0)
}

pub fn adopt(args: AdoptArgs) -> Result<i32> {
    let cfg = config::load()?;
    let plan = Plan::build(&cfg);
    let changes: Vec<Change> = if args.all {
        let only = pick(&cfg, &args.agent)?;
        plan.own(only)
            .into_iter()
            .filter_map(|(r, a)| plan.cells[r][a].change.clone())
            .collect()
    } else {
        let (Some(agent), Some(skill)) = (&args.agent, &args.skill) else {
            bail!("name an agent and a skill, or pass `--all`");
        };
        let i = pick(&cfg, &Some(agent.clone()))?.unwrap_or(0);
        let name = skill.strip_prefix("skill ").unwrap_or(skill);
        vec![Change::Adopt {
            from: cfg.agents[i].skills.join(name),
            to: cfg.source.skills.join(name),
        }]
    };
    let code = run_changes(
        changes,
        args.dry_run,
        "nothing to adopt: no agent keeps a skill your canon does not have",
    );
    if code == 0 && !args.dry_run {
        out!(
            "{}",
            "run `canon fix` to link what was adopted into every agent".dimmed()
        );
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RulesMode, SkillsMode};
    use crate::tmp::{self, Temp};
    use std::os::unix::fs::symlink;

    /// A canon with a rules file, a house file and one skill, Claude beside it,
    /// and one project importing the house file.
    fn world() -> (Temp, Config) {
        let t = Temp::new();
        t.write("canon/rules.yaml", "title: MYRULES\n");
        t.write("canon/house/HOUSE-RUST.md", "# rust\n");
        t.skill("canon/skills/audit", "audit");
        t.write(
            "canon/canonize.toml",
            "projects = []\n[source]\nrules = \"rules.yaml\"\n",
        );
        t.dir("claude/skills");
        t.dir("dev/app");
        let cfg = tmp::config(
            tmp::source(&t.at("canon")),
            vec![tmp::claude(&t.at("claude"))],
            vec![t.at("dev")],
        );
        (t, cfg)
    }

    fn run(changes: Vec<Change>) {
        for c in changes {
            c.run().expect("a change the plan offered should run");
        }
    }

    #[test]
    fn every_import_of_the_moved_canon_is_repointed_wherever_it_sits() {
        let (t, cfg) = world();
        let old = t.at("canon");
        let new = t.at("moved");
        // The rules import canonize wrote, and a house file the user imports
        // globally, which canonize otherwise never touches.
        t.write(
            "claude/CLAUDE.md",
            &format!(
                "# mine\n\n@{}\n@{}\n\nmy own line\n",
                old.join("rules.yaml").display(),
                old.join("house/HOUSE-RUST.md").display()
            ),
        );
        t.write(
            "dev/app/CLAUDE.md",
            &format!("# app\n@{}\n", t.at("dev/app/CANON.md").display()),
        );
        t.write(
            "dev/app/CANON.md",
            &format!("@{}\n", old.join("house/HOUSE-RUST.md").display()),
        );

        run(repoint(&cfg, &old, &new));

        let claude = fs::read_to_string(t.at("claude/CLAUDE.md")).expect("it should be there");
        assert!(
            claude.contains(&format!("@{}", new.join("rules.yaml").display())),
            "{claude}"
        );
        assert!(
            claude.contains(&format!("@{}", new.join("house/HOUSE-RUST.md").display())),
            "a house file imported globally moves with the canon: {claude}"
        );
        assert!(
            claude.contains("my own line"),
            "the user's lines are theirs"
        );
        let canon_md = fs::read_to_string(t.at("dev/app/CANON.md")).expect("it should be there");
        assert_eq!(
            canon_md.trim(),
            format!("@{}", new.join("house/HOUSE-RUST.md").display())
        );
    }

    #[test]
    fn an_import_that_names_something_else_is_left_alone() {
        let (t, cfg) = world();
        let mine = t.write("notes/RULES.md", "mine\n");
        t.write("claude/CLAUDE.md", &format!("@{}\n", mine.display()));
        let changes = repoint(&cfg, &t.at("canon"), &t.at("moved"));
        assert!(
            changes.is_empty(),
            "nothing named the canon, so nothing moves: {:?}",
            changes.iter().map(Change::describe).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_link_into_the_moved_canon_is_relinked_and_one_pointing_elsewhere_is_not() {
        let (t, cfg) = world();
        let old = t.at("canon");
        let new = t.at("moved");
        symlink(old.join("skills/audit"), t.at("claude/skills/audit")).expect("could not link");
        let outside = t.skill("elsewhere/other", "other");
        symlink(&outside, t.at("claude/skills/other")).expect("could not link");

        run(repoint(&cfg, &old, &new));

        assert_eq!(
            fs::read_link(t.at("claude/skills/audit")).expect("still a link"),
            new.join("skills/audit")
        );
        assert_eq!(
            fs::read_link(t.at("claude/skills/other")).expect("still a link"),
            outside,
            "a link outside the canon is the user's"
        );
    }

    #[test]
    fn a_file_inside_what_moved_is_edited_where_it_lands() {
        let (t, cfg) = world();
        let old = t.at("canon");
        let new = t.at("moved");
        // A skills folder spelled out as an absolute path inside the canon.
        t.write(
            "canon/canonize.toml",
            &format!(
                "projects = []\n[source]\nrules = \"rules.yaml\"\nskills = \"{}\"\n",
                old.join("skills").display()
            ),
        );
        let changes = repoint(&cfg, &old, &new);
        fs::rename(&old, &new).expect("the move happens first");
        run(changes);

        let text = fs::read_to_string(new.join("canonize.toml")).expect("it moved with the canon");
        assert!(
            text.contains(&format!("\"{}\"", new.join("skills").display())),
            "{text}"
        );
    }

    #[test]
    fn a_rules_file_reached_through_a_link_is_never_written_into() {
        let (t, cfg) = world();
        let old = t.at("canon");
        // The rules file imports a house file, so reading it through the link
        // would find a line to rewrite, and rewrite the canon's own file.
        let body = format!(
            "title: MYRULES\n@{}\n",
            old.join("house/HOUSE-RUST.md").display()
        );
        let rules = t.write("canon/rules.yaml", &body);
        // Codex's instructions file is the rules file itself, seen from its home.
        let mut cfg = cfg;
        cfg.agents = vec![tmp::agent(
            "codex",
            &t.at("codex"),
            &t.at("codex/AGENTS.md"),
            RulesMode::Link,
            &t.at("codex/skills"),
            SkillsMode::Folder,
        )];
        t.dir("codex");
        symlink(&rules, t.at("codex/AGENTS.md")).expect("could not link");

        run(repoint(&cfg, &old, &t.at("moved")));

        assert_eq!(
            fs::read_to_string(&rules).expect("the rules file should be there"),
            body,
            "an import written through the link would land in the rules file"
        );
    }

    #[test]
    fn a_move_that_already_happened_only_repoints() {
        let (t, cfg) = world();
        let old = t.at("canon");
        let new = t.at("moved");
        t.write(
            "claude/CLAUDE.md",
            &format!("@{}\n", old.join("rules.yaml").display()),
        );
        fs::rename(&old, &new).expect("moved by hand");

        let (rename, repoints) = plan_move(&cfg, &old, &new).expect("it should plan");
        assert!(rename.is_empty(), "there is nothing left to move");
        assert_eq!(repoints.len(), 1, "the import still names the old path");
    }

    #[test]
    fn a_move_onto_something_that_exists_is_refused() {
        let (t, cfg) = world();
        t.dir("moved");
        assert!(
            plan_move(&cfg, &t.at("canon"), &t.at("moved")).is_err(),
            "it would have to merge two folders, or lose one"
        );
        assert!(
            plan_move(&cfg, &t.at("gone"), &t.at("also-gone")).is_err(),
            "neither path is there"
        );
    }

    #[test]
    fn what_moves_is_named_by_the_canon_or_by_a_path() {
        let (t, cfg) = world();
        assert_eq!(
            targets(&cfg, "canon", &t.at("moved").display().to_string())
                .expect("canon")
                .0,
            t.at("canon")
        );
        let (from, to) = targets(&cfg, "HOUSE-RUST.md", "house").expect("a name inside the canon");
        assert_eq!(from, t.at("canon/HOUSE-RUST.md"));
        assert_eq!(to, t.at("canon/house/HOUSE-RUST.md"));
        let (from, to) = targets(
            &cfg,
            &t.at("old-canon").display().to_string(),
            &t.at("moved").display().to_string(),
        )
        .expect("two paths");
        assert_eq!((from, to), (t.at("old-canon"), t.at("moved")));
    }
}
