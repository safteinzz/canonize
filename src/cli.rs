//! The plain commands: the same engines the TUI drives, printed for a person
//! or a script.

use crate::check;
use crate::config::{self, Config, tilde};
use crate::init;
use crate::plan::{Change, Plan, State};
use crate::projects::{self, Projects};
use crate::setup;
use anyhow::{Result, bail};
use colored::Colorize;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct AgentArgs {
    /// Only this agent (as named in `canon status`)
    #[arg(short, long, value_name = "NAME")]
    pub agent: Option<String>,
}

#[derive(clap::Args)]
pub struct ChangeArgs {
    /// Only this agent (as named in `canon status`)
    #[arg(short, long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Print what would change and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct DryArgs {
    /// Print what would change and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct AdoptArgs {
    /// The agent whose skill it is (as named in `canon status`)
    pub agent: String,
    /// The skill's folder name inside that agent's skills folder
    pub skill: String,
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

/// Exit code 1 when anything has drifted, so `canon status` can gate a script.
pub fn status(args: AgentArgs) -> Result<i32> {
    let cfg = config::load()?;
    let only = pick(&cfg, &args.agent)?;
    let plan = Plan::build(&cfg);
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
        for (r, row) in plan.rows.iter().enumerate() {
            match &plan.cells[r][i].state {
                State::Broken(why) | State::Foreign(why) => {
                    notes.push(format!("{} {}: {why}", a.name, row.label()))
                }
                _ => {}
            }
        }
        if !plan.foreign[i].is_empty() {
            notes.push(format!(
                "{} also has, not skills (no SKILL.md): {}",
                a.name,
                plan.foreign[i].join(", ")
            ));
        }
    }
    for c in &plan.stale {
        notes.push(format!("stale: {}", c.describe()));
    }
    let projects = Projects::build(&cfg);
    if only.is_none() && !cfg.projects.is_empty() {
        print_projects(&cfg, &projects, &mut notes);
    }
    if !notes.is_empty() {
        out!();
        for n in notes {
            out!("{}", n.dimmed());
        }
    }
    Ok(if plan.drifted() || projects.drifted() {
        1
    } else {
        0
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

pub fn validate() -> Result<i32> {
    let cfg = config::load()?;
    let report = check::run(&cfg.source);
    for p in &report.problems {
        out!("{p}");
    }
    if report.problems.is_empty() {
        out!("valid: {}", check::counts(&report));
        Ok(0)
    } else {
        Ok(1)
    }
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

pub fn setup(args: DryArgs) -> Result<i32> {
    if config::load().is_ok() {
        out!(
            "{}",
            format!("already set up in {}", tilde(&config::source_dir())).dimmed()
        );
        return Ok(0);
    }
    let Some(found) = setup::detect_or_moved() else {
        bail!(
            "none of your agents reads a rules file yet: run `canon init` to lay out a new source"
        );
    };
    out!(
        "found {} (read by {})",
        tilde(&found.rules),
        tilde(&found.by)
    );
    let choice = found.choice();
    out!();
    for line in choice.summary() {
        out!("{line}");
    }
    if args.dry_run {
        out!(
            "\n{}",
            "nothing written: run `canon setup` to write it".dimmed()
        );
    } else {
        setup::apply(&choice)?;
        out!("\n{} set up", "✓".green());
    }
    Ok(0)
}

pub fn adopt(args: AdoptArgs) -> Result<i32> {
    let cfg = config::load()?;
    let i = pick(&cfg, &Some(args.agent))?.unwrap_or(0);
    let from = cfg.agents[i].skills.join(&args.skill);
    let to = setup::adopt(&from, &cfg.source.skills)?;
    out!(
        "{} moved {} to {}, linked back",
        "✓".green(),
        tilde(&from),
        tilde(&to)
    );
    Ok(0)
}
