//! Reading the canon and every agent, project and workspace, and running a
//! batch of changes, off the input thread. At start the read happens before
//! the terminal is taken, with a braille wordmark decoding on stderr above a
//! progress line once it outlasts a moment; a job started from the dashboard
//! runs in the background behind a moving mark in the footer.

use crate::check::{self, Report};
use crate::config::{self, Config};
use crate::houses::Houses;
use crate::mcp::Mcps;
use crate::plan::{Change, Plan};
use crate::projects::Projects;
use crate::workspaces::Workspaces;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Everything the dashboard shows, read in one go.
pub(crate) struct Data {
    pub cfg: Config,
    pub plan: Plan,
    pub projects: Projects,
    pub houses: Houses,
    pub mcps: Mcps,
    pub spaces: Workspaces,
    pub report: Report,
}

/// What a read is doing now, by index into `STAGES`, for the line under the
/// wordmark.
static STAGE: AtomicUsize = AtomicUsize::new(0);

const STAGES: [&str; 6] = [
    "reading canonize.toml",
    "looking at each agent",
    "looking through your scopes for projects",
    "reading MCP servers",
    "reading workspaces",
    "checking your canon",
];

/// Read the config and the disk, or say why the config does not load.
pub(super) fn read() -> Result<Data, String> {
    let stage = |s: usize| STAGE.store(s, Ordering::Relaxed);
    stage(0);
    let cfg = config::load().map_err(|e| crate::said(&e))?;
    stage(1);
    let plan = Plan::build(&cfg);
    let houses = Houses::build(&cfg);
    stage(2);
    let projects = Projects::build(&cfg);
    stage(3);
    let mcps = Mcps::build(&cfg, &projects);
    stage(4);
    let spaces = Workspaces::build(&cfg, &projects);
    stage(5);
    let report = check::run(&cfg.source);
    Ok(Data {
        cfg,
        plan,
        projects,
        houses,
        mcps,
        spaces,
        report,
    })
}

/// How long a read runs before it explains itself.
const GRACE: Duration = Duration::from_millis(150);

/// How often the wordmark and the footer's mark move.
pub(super) const TICK: Duration = Duration::from_millis(60);

/// The wordmark, in braille, two rows tall: the one block of glyphs a
/// terminal font is expected to have at a fixed width.
const MARK: [&str; 2] = ["⡎⠉ ⣎⣱ ⡷⣸ ⡎⢱ ⡷⣸ ⡇ ⠉⡹ ⣏⡉", "⢇⣀ ⡇⢸ ⡇⢹ ⢇⡸ ⡇⢹ ⡇ ⣎⣀ ⣇⣀"];
const SCRAMBLED: &str = "\x1b[2;90m";
const LOCKED: &str = "\x1b[1;96m";
const OFF: &str = "\x1b[0m";

/// `read` before the terminal is taken. Past `GRACE`, and only when stderr
/// is a terminal, the wordmark decodes above a line saying what is being
/// read, drawn from a timer so a slow folder never looks like a hang, and
/// wiped before the dashboard draws.
pub(super) fn read_at_start() -> Result<Data, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read());
    });
    let start = Instant::now();
    let shown = std::io::stderr().is_terminal();
    let mut err = std::io::stderr();
    let mut noise = 0x2545_F491_4F6C_DD1D_u64;
    let mut up = "";
    loop {
        match rx.recv_timeout(TICK) {
            Ok(data) => {
                if !up.is_empty() {
                    // Up to the wordmark's first row, then clear to the end.
                    let _ = write!(err, "\x1b[2A\r\x1b[J");
                    let _ = err.flush();
                }
                return data;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the read stopped before it finished".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !shown || start.elapsed() < GRACE {
            continue;
        }
        let stage = STAGE.load(Ordering::Relaxed).min(STAGES.len() - 1);
        let line = progress_line(STAGES[stage]);
        let mark = wordmark(stage as f64 / STAGES.len() as f64, &mut noise);
        // Back up over the wordmark, so every frame overwrites the last in place.
        let _ = write!(err, "{up}\r{mark}\r\x1b[K{line}");
        let _ = err.flush();
        up = "\x1b[2A";
    }
}

/// The line under the wordmark, cut to the window since a line that wraps is
/// one `\r` can no longer rewrite.
fn progress_line(stage: &str) -> String {
    let cols = ratatui::crossterm::terminal::size().map_or(80, |(c, _)| usize::from(c));
    format!("{stage} ...")
        .chars()
        .take(cols.saturating_sub(1))
        .collect()
}

/// The wordmark with the leftmost `share` of its columns locked in and the
/// rest noise, ending each row with a newline.
fn wordmark(share: f64, noise: &mut u64) -> String {
    let cols = MARK[0].chars().count();
    let locked = (share.clamp(0.0, 1.0) * cols as f64).round() as usize;
    let mut out = String::new();
    for row in MARK {
        for (i, ch) in row.chars().enumerate() {
            // A gap stays a gap, or the letters would run together.
            if ch == ' ' || i < locked {
                out.push_str(LOCKED);
                out.push(ch);
            } else {
                out.push_str(SCRAMBLED);
                out.push(scramble(noise));
            }
            out.push_str(OFF);
        }
        out.push_str("\x1b[K\n");
    }
    out
}

/// One braille cell of noise from the six dot patterns the wordmark mostly
/// uses, so it looks like what it resolves into.
fn scramble(noise: &mut u64) -> char {
    *noise ^= *noise << 13;
    *noise ^= *noise >> 7;
    *noise ^= *noise << 17;
    char::from_u32(0x2800 + (*noise % 64) as u32).unwrap_or('⠿')
}

/// The footer's moving mark at `elapsed`.
pub(super) fn spinner(elapsed: Duration) -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    FRAMES[(elapsed.as_millis() / TICK.as_millis()) as usize % FRAMES.len()]
}

/// What the dashboard does once a job lands, beyond showing what it read.
pub(crate) enum Then {
    /// Say how the changes went.
    Report,
    /// Say how they went, and put the projects tab's cursor on this folder.
    Select(PathBuf),
    /// Say the workspace `key` points at `all`, or why a file there was left
    /// alone, and put the workspaces tab's cursor on it.
    Linked {
        key: String,
        message: String,
        foreign: Option<String>,
    },
    /// A refresh: say `message`.
    Refreshed(String),
}

/// A batch of changes, then a read, running in the background.
pub(crate) struct Job {
    /// What it is, as the footer and the result name it.
    pub what: String,
    pub total: usize,
    pub done: Arc<AtomicUsize>,
    pub started: Instant,
    pub then: Then,
    landed: Receiver<Landed>,
}

/// What a job hands back: the changes that failed, each with why, and what
/// the disk says afterwards.
pub(crate) struct Landed {
    pub failed: Vec<String>,
    pub data: Result<Data, String>,
}

impl Job {
    /// Run `changes` in order, a failure stopping only the ones that run
    /// after everything else worked, then read everything again.
    pub(super) fn start(what: String, changes: Vec<Change>, then: Then) -> Job {
        let (tx, landed) = mpsc::channel();
        let done = Arc::new(AtomicUsize::new(0));
        let total = changes.len();
        let counter = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut failed = Vec::new();
            for c in changes {
                if !failed.is_empty() && c.after_all() {
                    failed.push(format!(
                        "{}: not done, since a step before it failed",
                        c.describe()
                    ));
                } else if let Err(e) = c.run() {
                    failed.push(format!("{}: {}", c.describe(), crate::said(&e)));
                }
                counter.fetch_add(1, Ordering::Relaxed);
            }
            let _ = tx.send(Landed {
                failed,
                data: read(),
            });
        });
        Job {
            what,
            total,
            done,
            started: Instant::now(),
            then,
            landed,
        }
    }

    /// The job's result, once it has one.
    pub(super) fn landed(&self) -> Option<Landed> {
        match self.landed.try_recv() {
            Ok(l) => Some(l),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Landed {
                failed: vec![format!("{} stopped before it finished", self.what)],
                data: read(),
            }),
        }
    }

    /// The footer's first entry: the mark, what runs, and how far it got.
    pub(super) fn footer(&self) -> String {
        let mark = spinner(self.started.elapsed());
        if self.total == 0 {
            format!("{mark} {}", self.what)
        } else {
            let done = self.done.load(Ordering::Relaxed).min(self.total);
            format!("{mark} {} {done}/{}", self.what, self.total)
        }
    }
}
