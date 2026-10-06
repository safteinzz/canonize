//! canonize: one source of truth for your coding agents. Binary: `canon`.
//!
//! This file is the clap `Cmd` enum and the dispatch match; what you can run is
//! `canon --help`, which renders from the manifest, those doc comments and
//! `AFTER`. `config` finds the source and the agents, `plan` compares what each
//! agent has with what it should have, `houses` which agent reads which house
//! file everywhere, `check` validates the canon itself, `cli` prints those for
//! scripts and `tui` shows them to people.

/// `println!` for command output. A reader that goes away early (`| head`) is
/// a normal end for a command whose output is data, not a panic.
macro_rules! out {
    () => {{
        use std::io::Write;
        if writeln!(std::io::stdout()).is_err() {
            std::process::exit(0);
        }
    }};
    ($($t:tt)*) => {{
        use std::io::Write;
        if writeln!(std::io::stdout(), $($t)*).is_err() {
            std::process::exit(0);
        }
    }};
}

mod check;
mod cli;
mod config;
mod houses;
mod init;
mod mcp;
mod plan;
mod projects;
mod selfcmd;
mod setup;
#[cfg(test)]
mod tmp;
mod tui;

use clap::{Parser, Subcommand};

/// clap's own layout with `{before-help}` moved under `Usage:`, so the shapes
/// block lands on top of the command list rather than on top of the screen.
const TEMPLATE: &str =
    "{about-with-newline}\n{usage-heading} {usage}\n\n{before-help}{all-args}{after-help}\n";

const WAYS: &str = "\x1b[1mWays to run it (not subcommands):\x1b[0m
  canon                   open the dashboard (TUI): every agent and project, and what needs fixing";

const AFTER: &str = concat!(
    "\
\x1b[1mWhat a cell says:\x1b[0m
  linked   wired to your canon; a convention says imported, an MCP server added
  unwired  `fix` wires it         broken    wired wrongly, `fix` repoints it
  own      only that agent has it, `adopt` takes it into your canon
  foreign  yours or the agent's, left alone        n/a, off  cannot, turned off
  -        not installed (agents table); elsewhere not there (`none`), no drift
  unapproved  a project imports it, but Claude is not allowed to load it there

\x1b[1mConventions:\x1b[0m
  Files an agent reads for one kind of work: an agent's row reads one everywhere,
  a project imports it in its gitignored CANON.md, which Claude reads through
  CLAUDE.local.md once allowed there (`--json` `claude`: approved, not_asked,
  declined). Codex reads no other file. `add` and `remove` change who reads one.

\x1b[1mMCP servers:\x1b[0m
  A table each in your canon's mcp.toml: a `url` (`token = true` for one typed in
  the TUI and kept outside your canon; `--json` `token`: kept, missing) or a
  `command`. The TUI's MCPs tab adds one to an agent or project (c create,
  e edit, d delete); `fix` rewrites what differs, `delete` takes them back.

Your canon is `$CANONIZE_SOURCE`, else your config folder (~/.config/canonize,
and ~/Library/Application Support/canonize on macOS), and holds canonize.toml.
`status` and `validate` print for people and exit 1 when something needs fixing;
`status --strict` counts foreign, own and those waiting projects too, `--json`
on either prints it for a script, with those state names (`na`, `absent`) and
absolute paths. `fix` never settles a foreign or own cell or a waiting project,
so `status` ends by naming them and what to do, and what `validate` reports is
yours to edit. Every command that changes something takes `-n` (`--dry-run`), a
dry run printing the very lines the real run would, and writing nothing; `self
update` is the exception, since all it changes is canonize itself. Errors go
to stderr, exit non-zero.
Run `canon <command> --help` for a command's details.",
    "\n\n",
    env!("CARGO_PKG_REPOSITORY"),
    "\ncontributors: ",
    env!("CARGO_PKG_AUTHORS"),
);

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n",
    env!("CARGO_PKG_LICENSE"),
    "  ",
    env!("CARGO_PKG_REPOSITORY"),
    "\ncontributors: ",
    env!("CARGO_PKG_AUTHORS"),
);

#[derive(Parser)]
#[command(
    name = "canonize",
    bin_name = "canon",
    version,
    long_version = LONG_VERSION,
    about,
    help_template = TEMPLATE,
    before_help = WAYS,
    after_help = AFTER
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show which agent has which rules, skills, conventions and MCP servers, and what drifted
    ///   -a NAME   only this agent
    ///   --strict  exit 1 when something needs a person too (foreign, own, waiting on Claude)
    ///   --json    print it for a script instead of a person
    #[command(verbatim_doc_comment)]
    Status(cli::AgentArgs),
    /// Fix what is broken or missing: link, repoint, rewrite what differs; foreign files stay
    ///   -a NAME   only this agent
    ///   -n        dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Fix(cli::ChangeArgs),
    /// Delete every link, import and MCP entry canonize made, and nothing else
    ///   -a NAME   only this agent
    ///   -n        dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Delete(cli::ChangeArgs),
    /// Make projects, or an agent in every project, read one convention  <CONVENTION> [PROJECT]...
    ///   -a NAME                 an agent, which then reads it in every project
    ///   --every project|agent   every project, or every agent that can read it
    ///   -n                      dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Add(cli::PlaceArgs),
    /// Stop projects, or an agent, reading one convention  <CONVENTION> [PROJECT]...
    ///   -a NAME                 an agent, which then reads it in no project
    ///   --every project|agent   every project, or every agent
    ///   -n                      dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Remove(cli::PlaceArgs),
    /// Validate your canon: the rules file against its schema, every SKILL.md, mcp.toml
    ///   --json    print it for a script instead of a person
    #[command(verbatim_doc_comment)]
    Validate(cli::JsonArgs),
    /// Set canonize up around the rules file your agents already read
    ///   -n            dry run: print what it found and change nothing
    ///   --root DIR    the folder your canon lives in, when it found none
    ///   --rules NAME  your rules file in it (every answer has a flag)
    #[command(verbatim_doc_comment)]
    Setup(cli::SetupArgs),
    /// Move an agent's own skill into your canon and leave a link in its place  [AGENT] [SKILL]
    ///   --all     every skill that agent keeps itself, or every agent's
    ///   -n        dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Adopt(cli::AdoptArgs),
    /// Move a folder that is not a skill out of your canon, back to one agent  [NAME] [AGENT]
    ///   --all     every folder in your canon that is not a skill
    ///   --drop    delete your canon's copy, when the agent already has one
    ///   -n        dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Evict(cli::EvictArgs),
    /// Move your canon, or something in it, and repoint every import and link  <WHAT> <TO>
    ///   canon move canon ~/dotfiles/canon        the whole folder, agents repointed
    ///   canon move house conventions             renamed, or into a folder that is there
    #[command(verbatim_doc_comment)]
    Move(cli::MoveArgs),
    /// Change a setting in canonize.toml, comments and all
    ///   config set <KEY> <VALUE>...   `agents.pi.skills_mode`, `projects`, `source.rules`
    ///     -n        dry run: print the line it would write and change nothing
    #[command(verbatim_doc_comment, subcommand)]
    Config(cli::ConfigCmd),
    /// Lay out a new canon folder: canonize.toml, rules.yaml, its schema, conventions/, skills/  [DIR]
    ///   -n        dry run: print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Init(cli::InitArgs),
    /// Manage canonize itself: `self update` reinstalls, `self check` looks for a newer release
    #[command(name = "self", subcommand)]
    Selfie(selfcmd::Cmd),
}

fn main() {
    let cli = Cli::parse();
    let res = match cli.command {
        None => tui::run().map(|_| 0),
        Some(Cmd::Status(a)) => cli::status(a),
        Some(Cmd::Fix(a)) => cli::fix(a),
        Some(Cmd::Delete(a)) => cli::delete(a),
        Some(Cmd::Add(a)) => cli::place(a, false),
        Some(Cmd::Remove(a)) => cli::place(a, true),
        Some(Cmd::Validate(a)) => cli::validate(a),
        Some(Cmd::Setup(a)) => cli::setup(a),
        Some(Cmd::Adopt(a)) => cli::adopt(a),
        Some(Cmd::Evict(a)) => cli::evict(a),
        Some(Cmd::Move(a)) => cli::move_it(a),
        Some(Cmd::Config(c)) => cli::config(c),
        Some(Cmd::Init(a)) => cli::init(a),
        Some(Cmd::Selfie(c)) => {
            selfcmd::run(c);
            Ok(0)
        }
    };
    match res {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("canon: {e:#}");
            std::process::exit(1);
        }
    }
}
