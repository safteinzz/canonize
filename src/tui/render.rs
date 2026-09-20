//! Drawing the frame: the source line, the table, the selected cell explained,
//! the status line, and every box on top.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap};

use super::alert::render_note;
use super::confirm::render_confirm;
use super::scope::render_scope;
use super::typed::render_typed;
use super::wizard::render_wizard;
use super::{App, View};
use crate::config::{RulesMode, SkillsMode, tilde};
use crate::plan::{self, State};
use crate::projects;

const HINTS: &str =
    "j/k agent · ↵ open its card · F fix every agent · D delete every agent's setup · ? help";
const CARD_HINTS: &str = "j/k line · ↵ f fix · d delete · esc back to the list · ? help";
const SKILL_HINTS: &str =
    "↵ toggle · a link… · d delete… · F link all · D delete all links · ? help";
/// `{file}` is the selected project's instruction file, `CLAUDE.md` or `AGENTS.md`.
const PROJECT_HINTS: &str =
    "↵ toggle · a add… · d delete… · F fix imports · D delete all · o open {file} · ? help";

pub(super) fn ui(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(11),
        Constraint::Length(1),
    ])
    .split(area);

    render_source(f, chunks[0], app);
    match &app.load_error {
        Some(e) => {
            let mut lines = vec![Line::raw(
                "canonize is not set up yet: press s to set it up.",
            )];
            // The reason is for someone who quit the wizard; while it is up it
            // would only be noise behind it.
            if app.wizard.is_none() {
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    e.clone(),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            let para = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).title(" welcome "))
                .wrap(Wrap { trim: false });
            f.render_widget(para, chunks[1]);
        }
        None if app.view == View::Projects => {
            render_projects(f, chunks[1], app);
            render_project_detail(f, chunks[2], app);
        }
        None if app.view == View::Skills => {
            render_skills(f, chunks[1], app);
            render_detail(f, chunks[2], app);
        }
        None => {
            render_agents(f, chunks[1], app);
            render_detail(f, chunks[2], app);
        }
    }
    render_status(f, chunks[3], app);

    if let Some(w) = &app.wizard {
        render_wizard(f, area, w);
    }
    if let Some(sc) = &app.scope {
        render_scope(f, area, sc);
    }
    if let Some(c) = &app.confirm {
        render_confirm(f, area, c);
    }
    if let Some(t) = &app.typed {
        render_typed(f, area, t);
    }
    // Last, so a failure is never drawn under the thing that caused it.
    if let Some(n) = &app.note {
        render_note(f, area, n);
    }
}

fn render_source(f: &mut Frame, area: Rect, app: &App) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut spans = vec![Span::styled("canon ", dim)];
    match &app.cfg {
        Some(cfg) => spans.push(Span::raw(tilde(&cfg.source.root))),
        None => spans.push(Span::raw(tilde(&crate::config::source_dir()))),
    }

    if let Some(r) = &app.report {
        spans.push(Span::styled("  ·  ", dim));
        if r.problems.is_empty() {
            spans.push(Span::styled("valid", Style::default().fg(Color::Green)));
        } else {
            spans.push(Span::styled(
                format!("{} problems (v)", r.problems.len()),
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    if app.cfg.is_some() {
        spans.push(Span::styled("  ·  e edit config", dim));
    }
    // The tabs on the left, where the canon is on the right, in one frame.
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" canonize · canon ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let cols = Layout::horizontal([
        Constraint::Length(30),
        Constraint::Length(9),
        Constraint::Min(0),
    ])
    .split(inner);
    let idx = match app.view {
        View::Agents => 0,
        View::Skills => 1,
        View::Projects => 2,
    };
    let tabs = Tabs::new(vec!["Agents", "Skills", "Projects"])
        .select(idx)
        .divider("│")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, cols[0]);
    f.render_widget(
        Paragraph::new("tab ⇄").style(Style::default().add_modifier(Modifier::DIM)),
        cols[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        cols[2],
    );
}

fn state_style(s: &State) -> Style {
    match s {
        State::Linked => Style::default().fg(Color::Green),
        State::Missing => Style::default(),
        State::Broken(_) => Style::default().fg(Color::Yellow),
        State::Foreign(_) => Style::default().fg(Color::Magenta),
        State::Own => Style::default().fg(Color::Cyan),
        State::Off | State::Absent | State::Na => Style::default().add_modifier(Modifier::DIM),
    }
}

/// A pane's frame, cyan when it has the keys.
fn focused(on: bool, title: &str) -> Block<'static> {
    let b = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string());
    if on {
        b.border_style(Style::default().fg(Color::Cyan))
    } else {
        b
    }
}

/// The agents tab: a list of agents, and a card of the selected one's setup.
fn render_agents(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(plan)) = (&app.cfg, &app.plan) else {
        return;
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    let cols = Layout::horizontal([Constraint::Length(26), Constraint::Min(0)]).split(area);

    let items: Vec<Line> = cfg
        .agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let drifted = plan.cells.iter().any(|r| r[i].state.drifted());
            let (dot, colour) = if !a.active() {
                ("○", Color::DarkGray)
            } else if drifted {
                ("●", Color::Yellow)
            } else {
                ("●", Color::Green)
            };
            let name = if !a.installed() {
                format!("{} (not installed)", a.name)
            } else if !a.enabled {
                format!("{} (disabled)", a.name)
            } else {
                a.name.clone()
            };
            let mut style = if a.active() { Style::default() } else { dim };
            if i == app.col {
                // Reversed while the list has the keys, underlined once the card does.
                style = style.add_modifier(if app.in_card {
                    Modifier::UNDERLINED
                } else {
                    Modifier::REVERSED
                });
            }
            Line::from(vec![
                Span::styled(format!(" {dot} "), Style::default().fg(colour)),
                Span::styled(name, style),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(items).block(focused(!app.in_card, " agents ")),
        cols[0],
    );

    let Some(agent) = cfg.agents.get(app.col) else {
        return;
    };
    let lines: Vec<Line> = app
        .card_lines()
        .iter()
        .enumerate()
        .map(|(n, line)| {
            let (label, word, style, note) = match line {
                super::CardLine::Row(r) => {
                    let cell = &plan.cells[*r][app.col];
                    let per_project = plan.rows[*r] == plan::Row::Loader && agent.name == "claude";
                    let word = if per_project {
                        "per project"
                    } else {
                        cell.state.word()
                    };
                    let note = match &plan.rows[*r] {
                        plan::Row::Rules(_) => tilde(&cell.at),
                        _ if per_project => "through each project's CLAUDE.md".to_string(),
                        _ if cell.state == State::Na => "cannot load another file".to_string(),
                        _ => tilde(&cell.at),
                    };
                    (
                        plan.rows[*r].label(),
                        word.to_string(),
                        state_style(&cell.state),
                        note,
                    )
                }
                super::CardLine::Skills => {
                    let rows = plan.skill_rows();
                    let count = |s: fn(&State) -> bool| {
                        rows.iter()
                            .filter(|r| s(&plan.cells[**r][app.col].state))
                            .count()
                    };
                    let linked = count(|s| *s == State::Linked);
                    let missing = count(|s| s.drifted());
                    let own = count(|s| *s == State::Own);
                    let canon = plan.canon_skills;
                    let mut parts = vec![format!("{linked} of {canon} linked")];
                    if missing > 0 {
                        parts.push(format!("{missing} to fix"));
                    }
                    if own > 0 {
                        parts.push(format!("{own} own"));
                    }
                    let style = if missing > 0 {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::Green)
                    };
                    let word = if agent.active() {
                        parts.join(" · ")
                    } else {
                        "-".to_string()
                    };
                    (
                        "skills".to_string(),
                        word,
                        style,
                        "each one in the skills tab".to_string(),
                    )
                }
            };
            let mut word_style = if agent.active() { style } else { dim };
            if app.in_card && n == app.aline {
                word_style = word_style.add_modifier(Modifier::REVERSED);
            }
            Line::from(vec![
                Span::raw(format!("{label:<16}")),
                Span::styled(format!(" {word} "), word_style),
                Span::styled(format!("  {note}"), dim),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .block(focused(app.in_card, &format!(" {} ", agent.name)))
            .wrap(Wrap { trim: false }),
        cols[1],
    );
}

/// The skills tab: a row per skill, in the canon or an agent's own, and a
/// column per agent.
fn render_skills(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(plan)) = (&app.cfg, &app.plan) else {
        return;
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    let block = Block::default().borders(Borders::ALL).title(" skills ");
    let rows_idx = plan.skill_rows();
    if rows_idx.is_empty() {
        let para = Paragraph::new(
            "No skills yet: put a folder with a SKILL.md in your canon's skills/, and a skill an agent keeps itself shows here as own, for f to adopt.",
        )
        .style(dim)
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }
    let label_w = rows_idx
        .iter()
        .map(|r| plan.rows[*r].label().chars().count())
        .max()
        .unwrap_or(5)
        .max(5) as u16;

    let mut header = vec![Cell::from("")];
    let mut widths = vec![Constraint::Length(label_w + 1)];
    for a in &cfg.agents {
        let text = if !a.installed() {
            format!("{} (not installed)", a.name)
        } else if !a.enabled {
            format!("{} (disabled)", a.name)
        } else {
            a.name.clone()
        };
        let style = if a.active() {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            dim
        };
        widths.push(Constraint::Length(text.chars().count().max(9) as u16 + 2));
        header.push(Cell::from(text).style(style));
    }

    let mut rows: Vec<Row> = Vec::new();
    for (n, r) in rows_idx.iter().enumerate() {
        let mut cells = vec![Cell::from(plan.rows[*r].label())];
        for (c, cell) in plan.cells[*r].iter().enumerate() {
            let mut style = state_style(&cell.state);
            if app.srow == n && app.col == c {
                style = style.add_modifier(Modifier::REVERSED);
            }
            cells.push(Cell::from(format!(" {} ", cell.state.word())).style(style));
        }
        rows.push(Row::new(cells));
    }
    let table = Table::new(rows, widths)
        .header(Row::new(header).bottom_margin(1))
        .block(block);
    f.render_widget(table, area);
}

/// The selected cell spelled out: where it lives, how it is wired, and what
/// `f` or `d` would do to it.
fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(plan)) = (&app.cfg, &app.plan) else {
        return;
    };
    let Some(agent) = cfg.agents.get(app.col) else {
        return;
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    let field =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<16}"), dim), Span::raw(v)]);
    let mut lines = Vec::new();
    let Some(r) = app.selected_row() else {
        // The skills line of the card: what f and d do for all of them.
        let rows = plan.skill_rows();
        for r in &rows {
            let cell = &plan.cells[*r][app.col];
            lines.push(Line::from(vec![
                Span::raw(format!("{:<24}", plan.rows[*r].label())),
                Span::styled(cell.state.word(), state_style(&cell.state)),
            ]));
        }
        lines.push(Line::styled(
            "f links every missing one · d deletes every link canonize made · own ones adopt in the skills tab",
            dim,
        ));
        let para = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} · skills ", agent.name)),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    };
    let row = &plan.rows[r];
    let cell = &plan.cells[r][app.col];
    let title = format!(" {} · {} ", agent.name, row.label());
    lines.push(Line::from(vec![
        Span::styled(format!("{:<16}", "state"), dim),
        Span::styled(cell.state.word(), state_style(&cell.state)),
    ]));
    if let State::Broken(why) | State::Foreign(why) = &cell.state {
        lines.push(field("why", why.clone()));
    }
    if let plan::Row::Rules(_) = row {
        lines.push(field("file", tilde(&cfg.source.rules)));
    }
    lines.push(field("at", tilde(&cell.at)));
    let how = match row {
        plan::Row::Rules(_) => match agent.rules_mode {
            RulesMode::Import => {
                format!("an `@{}` line in that file", tilde(&cfg.source.rules))
            }
            RulesMode::Link => format!("link to {}", tilde(&cfg.source.rules)),
            RulesMode::Off => "off".into(),
        },
        plan::Row::Loader => match agent.name.as_str() {
            "claude" => "per project: `@CANON.md` in its CLAUDE.md (projects tab)".into(),
            "pi" => "an extension that loads ./CANON.md".into(),
            "opencode" => "`\"instructions\": [\"CANON.md\"]` in opencode.json".into(),
            _ => "this agent has no way to load another file".into(),
        },
        plan::Row::Skill(name) => match agent.skills_mode {
            SkillsMode::PerSkill => format!("link to {}", tilde(&cfg.source.skills.join(name))),
            SkillsMode::Folder => {
                format!("the whole folder links to {}", tilde(&cfg.source.skills))
            }
            SkillsMode::Off => "off".into(),
        },
    };
    let how = if cell.state == State::Own {
        "a folder of the agent's own, not in your canon".to_string()
    } else {
        how
    };
    lines.push(field("how", how));
    // Each key is shown with the words it does here, so the panel is also the
    // key list for this cell: `↵ link`, `d unlink`, `f repoint`.
    if app.view == View::Skills {
        let toggle = match (&cell.state, &cell.change, &cell.undo) {
            (State::Linked, _, Some(u)) => Some(u),
            (State::Linked, _, None) => None,
            (_, Some(c), _) => Some(c),
            _ => None,
        };
        match toggle {
            Some(c) => lines.push(field(&format!("↵ {}", c.verb()), c.describe())),
            None => lines.push(field("↵", "nothing to do".to_string())),
        }
        if let (State::Own, Some(u)) = (&cell.state, &cell.undo) {
            lines.push(field(&format!("d… {}", u.verb()), u.describe()));
        }
    } else {
        match &cell.change {
            Some(c) => lines.push(field(&format!("f {}", c.verb()), c.describe())),
            None => lines.push(field("f", "nothing to fix".to_string())),
        }
        if let Some(u) = &cell.undo {
            lines.push(field(&format!("d {}", u.verb()), u.describe()));
        }
    }
    if !agent.installed() {
        lines.push(Line::styled(
            format!("not installed: {} does not exist", tilde(&agent.home)),
            dim,
        ));
    } else if !agent.enabled {
        lines.push(Line::styled("disabled in canonize.toml", dim));
    }
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn render_status(f: &mut Frame, area: Rect, app: &App) {
    let (text, style) = match app.live_status() {
        Some(msg) => (
            msg.to_string(),
            Style::default().fg(if app.status_failed {
                Color::Yellow
            } else {
                Color::Green
            }),
        ),
        None => (
            if app.load_error.is_some() {
                "s set up · ? help · q quit".to_string()
            } else if app.view == View::Projects {
                let file = app
                    .projects
                    .as_ref()
                    .and_then(|p| p.list.get(app.prow))
                    .and_then(|x| x.host.file_name())
                    .map_or("CLAUDE.md".to_string(), |n| {
                        n.to_string_lossy().into_owned()
                    });
                PROJECT_HINTS.replace("{file}", &file)
            } else if app.view == View::Skills {
                SKILL_HINTS.to_string()
            } else if app.in_card {
                CARD_HINTS.to_string()
            } else {
                HINTS.to_string()
            },
            Style::default().add_modifier(Modifier::DIM),
        ),
    };
    f.render_widget(Paragraph::new(format!(" {text}")).style(style), area);
}

/// The help, as one body for the reader box that `?` opens.
pub(super) const HELP: &str = "canonize: one source of truth for your coding agents\n\n\nAgents   j/k agent · ↵ open its card · esc back to the list\n         in the card: f fix the line · d delete it\n         F fix every agent's setup · D delete it all\nSkills   j/k skill · h/l agent · ↵ toggle (link, unlink, or adopt an own one)\n         a link… · d delete… (this cell, row or column; an own skill itself)\n         F link every missing skill · D delete every skill link\nProjects j/k project · h/l house file · ↵ toggle an import\n         a add… · d delete… (this cell, row or column)\n         F fix broken imports and CANON.md wiring · D delete all\n         o open the project's CLAUDE.md or AGENTS.md\nAnywhere tab switch · v validate your canon · e edit canonize.toml\n         r reload · ? help · q quit\n\nlinked   wired to your canon\nunwired  f wires it\nbroken   wired to the wrong thing; f repoints it\nforeign  something of yours or the agent's; left alone\nn/a      the agent has no way to use it\noff, -   turned off, or the agent is not installed\nown      a skill the agent keeps itself; f adopts it into your canon\n";

fn render_projects(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(p)) = (&app.cfg, &app.projects) else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" projects · house imports ");
    let dim = Style::default().add_modifier(Modifier::DIM);
    if cfg.projects.is_empty() || p.list.is_empty() {
        let text = if cfg.projects.is_empty() {
            "No project folders yet. Add `projects = [\"~/dev\"]` to canonize.toml (e to edit) and canonize lists every project with a CLAUDE.md or AGENTS.md."
        } else {
            "No project under your project folders has a CLAUDE.md or AGENTS.md."
        };
        let para = Paragraph::new(text)
            .style(dim)
            .block(block)
            .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }
    let names: Vec<String> = p
        .list
        .iter()
        .map(|x| projects::short(cfg, &x.root))
        .collect();
    let label_w = names
        .iter()
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(8)
        .max(8) as u16;
    let mut header = vec![Cell::from("house files →").style(dim)];
    let label_w = label_w.max(14);
    let mut widths = vec![Constraint::Length(label_w + 1)];
    for h in &p.house {
        let label = crate::cli::house_label(h);
        widths.push(Constraint::Length(label.chars().count().max(10) as u16 + 2));
        header.push(Cell::from(label).style(Style::default().add_modifier(Modifier::BOLD)));
    }
    let rows: Vec<Row> = p
        .list
        .iter()
        .enumerate()
        .map(|(r, x)| {
            let mut cells = vec![Cell::from(names[r].clone())];
            for (c, cell) in x.cells.iter().enumerate() {
                let (word, mut style) = match &cell.state {
                    State::Linked => ("imported", Style::default().fg(Color::Green)),
                    State::Broken(_) => ("broken", Style::default().fg(Color::Yellow)),
                    _ => ("-", dim),
                };
                if app.prow == r && app.pcol == c {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                cells.push(Cell::from(format!(" {word} ")).style(style));
            }
            Row::new(cells)
        })
        .collect();
    let table = Table::new(rows, widths)
        .header(Row::new(header).bottom_margin(1))
        .block(block);
    f.render_widget(table, area);
}

fn render_project_detail(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(p)) = (&app.cfg, &app.projects) else {
        return;
    };
    let (Some(x), Some(h)) = (p.list.get(app.prow), p.house.get(app.pcol)) else {
        let para = Paragraph::new("").block(Block::default().borders(Borders::ALL));
        f.render_widget(para, area);
        return;
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    let field =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<16}"), dim), Span::raw(v)]);
    let cell = &x.cells[app.pcol];
    let mut lines = vec![field(
        "state",
        match &cell.state {
            State::Linked => "imported".into(),
            State::Broken(why) => format!("broken: {why}"),
            _ => "not imported".into(),
        },
    )];
    lines.push(field("project", tilde(&x.root)));
    lines.push(field("file", tilde(&x.host)));
    lines.push(field("house", tilde(h)));
    // What Enter does for this cell, named by its verb like the skills tab.
    let toggle = if cell.state == State::Linked {
        cell.undo.as_ref()
    } else {
        cell.change.as_ref()
    };
    match toggle {
        Some(c) => lines.push(field(&format!("↵ {}", c.verb()), c.describe())),
        None => lines.push(field("↵", "nothing to do".to_string())),
    }
    for w in &x.wiring {
        let text = match (&w.state, &w.change) {
            (State::Linked, _) => "yes".to_string(),
            (State::Na, _) => "n/a".to_string(),
            (_, Some(c)) => format!("no · F {}", c.describe()),
            _ => "no".to_string(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}: ", w.what), dim),
            Span::styled(text, state_style(&w.state)),
        ]));
    }
    for d in &x.dead {
        lines.push(Line::styled(
            format!("{d} names a file that does not exist"),
            Style::default().fg(Color::Yellow),
        ));
    }
    let title = format!(
        " {} · {} ",
        projects::short(cfg, &x.root),
        crate::cli::house_label(h)
    );
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}
