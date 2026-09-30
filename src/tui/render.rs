//! Drawing the frame: the source line, the table, the selected cell explained,
//! the status line, and every box on top.

use ratatui::prelude::*;
use std::cell::Cell as Kept;

use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState, Tabs, Wrap,
};

use super::alert::render_note;
use super::confirm::render_confirm;
use super::scope::render_scope;
use super::typed::render_typed;
use super::widgets::wrapped_line_count;
use super::wizard::render_wizard;
use super::{App, HouseRow, View};
use crate::config::{RulesMode, SkillsMode, tilde};
use crate::plan::{self, State};
use crate::projects;

const HINTS: &str =
    "j/k agent · ↵ open its card · F fix every agent · D delete every agent's setup · ? help";
const CARD_HINTS: &str = "j/k line · ↵ f fix · d delete · esc back to the list · ? help";
const SKILL_HINTS: &str =
    "↵ toggle · a link… · d delete… · F link all · D delete all links · ? help";
/// `{open}` is ` · o open <file>` for the row's file, or nothing when it has none.
const HOUSE_HINTS: &str =
    "↵ toggle · a add… · d delete… · F fix broken · D delete all{open} · ? help";

pub(super) fn ui(f: &mut Frame, app: &App) {
    let area = f.area();
    let pane = detail_pane(app, area.width);
    let chunks = Layout::vertical([
        Constraint::Length(3),
        // Three rows of a grid under its header, taken from the detail pane
        // on a short screen, so the selection is never scrolled out of sight.
        Constraint::Min(7),
        Constraint::Length(pane.as_ref().map_or(0, |(_, h)| *h)),
        Constraint::Length(1),
    ])
    .split(area);
    if let Some((p, _)) = pane {
        p.render(f, chunks[2]);
    }

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
        None if app.view == View::Skills => render_skills(f, chunks[1], app),
        None if app.view == View::Houses => render_houses(f, chunks[1], app),
        None => render_agents(f, chunks[1], app),
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
            let n = r.problems.len();
            spans.push(Span::styled(
                format!("{n} problem{} (v)", if n == 1 { "" } else { "s" }),
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
        View::Houses => 2,
    };
    let tabs = Tabs::new(vec!["Agents", "Skills", "Houses"])
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
    let total = rows.len();
    let table = Table::new(rows, widths)
        .header(Row::new(header).bottom_margin(1))
        .block(block);
    render_grid(f, area, table, total, app.srow, &app.stop);
}

/// Rows the header of a grid takes: its line and the blank under it.
const GRID_HEADER_H: u16 = 2;

/// Draws a grid scrolled so the `selected` row is on screen, starting from
/// `top` and writing back where it ended up, with a scrollbar on the right
/// border whenever some of the `total` rows are out of sight.
fn render_grid(
    f: &mut Frame,
    area: Rect,
    table: Table,
    total: usize,
    selected: usize,
    top: &Kept<usize>,
) {
    let mut state = TableState::new()
        .with_offset(top.get())
        .with_selected(Some(selected));
    f.render_stateful_widget(table, area, &mut state);
    top.set(state.offset());
    let viewport = area.height.saturating_sub(2 + GRID_HEADER_H) as usize;
    if total <= viewport {
        return;
    }
    let mut bar = ScrollbarState::new(total - viewport).position(state.offset());
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        Rect {
            y: area.y + 1 + GRID_HEADER_H,
            height: viewport as u16,
            ..area
        },
        &mut bar,
    );
}

/// A detail pane's title and body, built before the frame is laid out so the
/// pane can be given exactly the rows it needs.
struct Pane {
    title: String,
    lines: Vec<Line<'static>>,
}

impl Pane {
    /// Rows the pane takes at `width` once wrapped, borders included.
    fn height(&self, width: u16) -> u16 {
        let inner = width.saturating_sub(2) as usize;
        let body: usize = self
            .lines
            .iter()
            .map(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                wrapped_line_count(&text, inner)
            })
            .sum();
        body as u16 + 2
    }

    fn render(self, f: &mut Frame, area: Rect) {
        let para = Paragraph::new(self.lines)
            .block(Block::default().borders(Borders::ALL).title(self.title))
            .wrap(Wrap { trim: false });
        f.render_widget(para, area);
    }
}

/// The detail pane for the current view: the selected cell's, and the height
/// of the tallest one any cell of that view would show, so moving the cursor
/// never resizes the grid under it.
fn detail_pane(app: &App, width: u16) -> Option<(Pane, u16)> {
    if app.load_error.is_some() {
        return None;
    }
    let tallest = |panes: Vec<Pane>| panes.iter().map(|p| p.height(width)).max().unwrap_or(0);
    if app.view == View::Houses {
        let cols = app.houses.as_ref()?.house.len();
        let pane = |r: usize, c: usize| match app.house_row_at(r)? {
            HouseRow::Agent(a) => house_detail(app, a, c),
            HouseRow::Project(i) => project_detail(app, i, c),
        };
        let all = (0..app.house_rows())
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .filter_map(|(r, c)| pane(r, c))
            .collect();
        return Some((pane(app.prow, app.pcol)?, tallest(all)));
    }
    let (cfg, plan) = (app.cfg.as_ref()?, app.plan.as_ref()?);
    let rows: Vec<Option<usize>> = if app.view == View::Skills {
        plan.skill_rows().into_iter().map(Some).collect()
    } else {
        plan.setup_rows()
            .into_iter()
            .map(Some)
            .chain([None])
            .collect()
    };
    let all = (0..cfg.agents.len())
        .flat_map(|c| rows.iter().map(move |r| (*r, c)))
        .filter_map(|(r, c)| detail(app, r, c))
        .collect();
    Some((detail(app, app.selected_row(), app.col)?, tallest(all)))
}

/// Agent `col`'s cell on plan row `row` spelled out: where it lives, how it
/// is wired, and what `f` or `d` would do to it; `None` is the card's skills
/// summary.
fn detail(app: &App, row: Option<usize>, col: usize) -> Option<Pane> {
    let (cfg, plan) = (app.cfg.as_ref()?, app.plan.as_ref()?);
    let agent = cfg.agents.get(col)?;
    let dim = Style::default().add_modifier(Modifier::DIM);
    let field =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<17}"), dim), Span::raw(v)]);
    let mut lines = Vec::new();
    let Some(r) = row else {
        // The skills line of the card: what f and d do for all of them.
        let rows = plan.skill_rows();
        for r in &rows {
            let cell = &plan.cells[*r][col];
            lines.push(Line::from(vec![
                Span::raw(format!("{:<24}", plan.rows[*r].label())),
                Span::styled(cell.state.word(), state_style(&cell.state)),
            ]));
        }
        lines.push(Line::styled(
            "f links every missing one · d deletes every link canonize made · own ones adopt in the skills tab",
            dim,
        ));
        return Some(Pane {
            title: format!(" {} · skills ", agent.name),
            lines,
        });
    };
    let row = &plan.rows[r];
    let cell = &plan.cells[r][col];
    let title = format!(" {} · {} ", agent.name, row.label());
    lines.push(Line::from(vec![
        Span::styled(format!("{:<17}", "state"), dim),
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
            "claude" => "per project: `@CANON.md` in its CLAUDE.md (houses tab)".into(),
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
    Some(Pane { title, lines })
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
            } else if app.view == View::Houses {
                let file = match app.house_row() {
                    Some(HouseRow::Project(i)) => app
                        .projects
                        .as_ref()
                        .and_then(|p| p.list.get(i))
                        .map(|x| x.host.clone()),
                    Some(HouseRow::Agent(a)) => app
                        .houses
                        .as_ref()
                        .and_then(|h| h.cells.get(app.pcol)?.get(a))
                        .map(|c| c.at.clone())
                        .filter(|at| at.is_file()),
                    None => None,
                };
                let open = file
                    .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .map_or(String::new(), |n| format!(" · o open {n}"));
                HOUSE_HINTS.replace("{open}", &open)
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
pub(super) const HELP: &str = "canonize: one source of truth for your coding agents\n\n\nAgents   j/k agent · ↵ open its card · esc back to the list\n         in the card: f fix the line · d delete it\n         F fix every agent's setup · D delete it all\nSkills   j/k skill · h/l agent · ↵ toggle (link, unlink, or adopt an own one)\n         a link… · d delete… (this cell, row or column; an own skill itself)\n         F link every missing skill · D delete every skill link\nHouses   j/k agent or project · h/l house file · ↵ toggle an import\n         an agent's row: it reads the file in every repo\n         a add… · d delete… (this cell, row or column)\n         F fix broken imports and CANON.md wiring · D delete all\n         o open the file the import sits in\nAnywhere tab switch · v validate your canon · e edit canonize.toml\n         r reload · ? help · q quit\n\nlinked   wired to your canon\nimported a house file is read there\nunwired  f wires it\nbroken   wired to the wrong thing; f repoints it\nforeign  something of yours or the agent's; left alone\nn/a      the agent has no way to use it\noff, -   turned off, or the agent is not installed\nown      a skill the agent keeps itself; f adopts it into your canon\n";

/// A house cell's word: `imported` for one that is wired, `-` for one that
/// is not, since neither needs fixing.
fn house_word(s: &State) -> &'static str {
    match s {
        State::Linked => "imported",
        State::Missing => "-",
        s => s.word(),
    }
}

/// The houses tab: a column per house file, and a row per place one can be
/// imported, every agent (for every repo) above every project.
fn render_houses(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(h), Some(p)) = (&app.cfg, &app.houses, &app.projects) else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" houses · who reads which house file ");
    let dim = Style::default().add_modifier(Modifier::DIM);
    if h.house.is_empty() {
        let para = Paragraph::new(
            "No house files yet: put a HOUSE-<NAME>.md in your canon's house/, then import it here into an agent, for every repo, or into a project.",
        )
        .style(dim)
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }
    let names: Vec<String> = cfg
        .agents
        .iter()
        .map(|a| format!("  {}", a.name))
        .chain(
            p.list
                .iter()
                .map(|x| format!("  {}", projects::short(cfg, &x.root))),
        )
        .collect();
    let label_w = names
        .iter()
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(0)
        .max(12) as u16;
    let mut header = vec![Cell::from("")];
    let mut widths = vec![Constraint::Length(label_w + 1)];
    for house in &h.house {
        let label = crate::cli::house_label(house);
        widths.push(Constraint::Length(label.chars().count().max(10) as u16 + 2));
        header.push(Cell::from(label).style(Style::default().add_modifier(Modifier::BOLD)));
    }
    let section = |text: &str| Row::new(vec![Cell::from(text.to_string()).style(dim)]);
    let mut rows = vec![section("every repo")];
    // The display row of the selection, past the section lines above it.
    let mut selected = 0;
    for (r, name) in names.iter().enumerate() {
        let (label, states): (Style, Vec<&State>) = match app.house_row_at(r) {
            Some(HouseRow::Agent(a)) => (
                if cfg.agents[a].active() {
                    Style::default()
                } else {
                    dim
                },
                h.cells.iter().map(|row| &row[a].state).collect(),
            ),
            Some(HouseRow::Project(i)) => {
                if i == 0 {
                    rows.push(section(""));
                    rows.push(section("projects"));
                }
                (
                    Style::default(),
                    p.list[i].cells.iter().map(|c| &c.state).collect(),
                )
            }
            None => continue,
        };
        if r == app.prow {
            selected = rows.len();
        }
        let mut cells = vec![Cell::from(name.clone()).style(label)];
        for (c, state) in states.into_iter().enumerate() {
            let mut style = if *state == State::Missing {
                dim
            } else {
                state_style(state)
            };
            if r == app.prow && c == app.pcol {
                style = style.add_modifier(Modifier::REVERSED);
            }
            cells.push(Cell::from(format!(" {} ", house_word(state))).style(style));
        }
        rows.push(Row::new(cells));
    }
    if p.list.is_empty() {
        rows.push(section(""));
        rows.push(section(if cfg.projects.is_empty() {
            "projects: none yet, add `projects = [\"~/dev\"]` to canonize.toml (e)"
        } else {
            "projects: none under your project folders has a CLAUDE.md or AGENTS.md"
        }));
    }
    let total = rows.len();
    let table = Table::new(rows, widths)
        .header(Row::new(header).bottom_margin(1))
        .block(block);
    render_grid(f, area, table, total, selected, &app.ptop);
}

/// Agent `a`'s import of house file `col` spelled out.
fn house_detail(app: &App, a: usize, col: usize) -> Option<Pane> {
    let (cfg, h) = (app.cfg.as_ref()?, app.houses.as_ref()?);
    let (house, cell, agent) = (
        h.house.get(col)?,
        h.cells.get(col)?.get(a)?,
        cfg.agents.get(a)?,
    );
    let dim = Style::default().add_modifier(Modifier::DIM);
    let field =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<17}"), dim), Span::raw(v)]);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:<17}", "state"), dim),
        Span::styled(
            match cell.state.why() {
                Some(why) => format!("{}: {why}", house_word(&cell.state)),
                None if cell.state == State::Missing => "not read".to_string(),
                None => house_word(&cell.state).to_string(),
            },
            state_style(&cell.state),
        ),
    ])];
    lines.push(field("house", tilde(house)));
    if !matches!(cell.state, State::Na | State::Absent) {
        lines.push(field("at", tilde(&cell.at)));
    }
    let toggle = if cell.state == State::Linked {
        cell.undo.as_ref()
    } else {
        cell.change.as_ref()
    };
    match toggle {
        Some(c) => lines.push(field(&format!("↵ {}", c.verb()), c.describe())),
        None => lines.push(field("↵", "nothing to do".to_string())),
    }
    if cell.state == State::Na {
        lines.push(Line::styled(
            format!("{} has no way to load another file", agent.name),
            dim,
        ));
    } else if !agent.installed() {
        lines.push(Line::styled(
            format!("not installed: {} does not exist", tilde(&agent.home)),
            dim,
        ));
    } else if !agent.enabled {
        lines.push(Line::styled("disabled in canonize.toml", dim));
    }
    Some(Pane {
        title: format!(" {} · {} ", agent.name, crate::cli::house_label(house)),
        lines,
    })
}

/// Project `prow`'s import of house file `pcol` spelled out, and its wiring.
fn project_detail(app: &App, prow: usize, pcol: usize) -> Option<Pane> {
    let (cfg, p) = (app.cfg.as_ref()?, app.projects.as_ref()?);
    let (x, h) = (p.list.get(prow)?, p.house.get(pcol)?);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let field =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<17}"), dim), Span::raw(v)]);
    let cell = &x.cells[pcol];
    let mut lines = vec![field(
        "state",
        match &cell.state {
            State::Linked => "imported".into(),
            State::Broken(why) => format!("broken: {why}"),
            _ => "not imported".into(),
        },
    )];
    lines.push(field("project", tilde(&x.root)));
    // Named by its key, because the import itself lives in CANON.md and a row
    // saying `file` beside it reads as the file the import sits in.
    lines.push(field("o opens", tilde(&x.host)));
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
    Some(Pane { title, lines })
}
