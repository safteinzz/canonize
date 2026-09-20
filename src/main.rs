//! canonize: one source of truth for your coding agents. Binary: `canon`.
//!
//! This file is the clap `Cmd` enum and the dispatch match; what you can run is
//! `canon --help`, which renders from the manifest, those doc comments and
//! `AFTER`. `config` finds the source and the agents, `plan` compares what each
//! agent has with what it should have, `check` validates the canon itself,
//! `cli` prints those for scripts and `tui` shows them to people.

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
mod init;
mod plan;
mod projects;
mod selfcmd;
mod setup;
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
Your canon is `$CANONIZE_SOURCE`, else your config folder (~/.config/canonize,
and ~/Library/Application Support/canonize on macOS), and holds canonize.toml.
`status` and `validate` print for people and exit 1 when something needs fixing,
so a script can gate on them; errors go to stderr and exit non-zero.
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
    /// Show which agent has which rules and skills, and what drifted
    ///   -a NAME   only this agent
    #[command(verbatim_doc_comment)]
    Status(cli::AgentArgs),
    /// Fix what is broken or missing: link it, or repoint an import; foreign files stay
    ///   -a NAME   only this agent
    ///   -n        print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Fix(cli::ChangeArgs),
    /// Delete every link and import canonize made, and nothing else
    ///   -a NAME   only this agent
    ///   -n        print what would change and change nothing
    #[command(verbatim_doc_comment)]
    Delete(cli::ChangeArgs),
    /// Validate your canon: the rules file against its schema, and every skill's SKILL.md
    Validate,
    /// Set canonize up around the rules file your agents already read
    ///   -n        print what it found and change nothing
    #[command(verbatim_doc_comment)]
    Setup(cli::DryArgs),
    /// Move an agent's own skill into your canon and leave a link in its place  <AGENT> <SKILL>
    Adopt(cli::AdoptArgs),
    /// Lay out a new canon folder: canonize.toml, rules.yaml, its schema, house/, skills/  [DIR]
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
        Some(Cmd::Validate) => cli::validate(),
        Some(Cmd::Setup(a)) => cli::setup(a),
        Some(Cmd::Adopt(a)) => cli::adopt(a),
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
