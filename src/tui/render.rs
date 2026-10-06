//! Drawing the frame: the source line, the table, the selected cell explained,
//! the status line, and every box on top.

use ratatui::prelude::*;
use std::cell::Cell as Kept;

use ratatui::widgets::{
    Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap,
};

use super::alert::render_note;
use super::confirm::render_confirm;
use super::line_edit;
use super::scope::render_scope;
use super::server_form::render_server_form;
use super::typed::render_typed;
use super::widgets::*;
use super::wizard::render_wizard;
use super::{App, HouseRow, View};
use crate::config::{RulesMode, SkillsMode, tilde};
use crate::plan::{self, State};
use crate::projects;

const ADD: &str = "a add";
const OPEN: &str = "o open";
const FIX_ALL: &str = "F fix all";
const ON_OFF: &str = "↵ on/off";
const AGENT_KEYS: &[&str] = &["↵ open", FIX_ALL, FIND, REFRESH, QUIT];
const CARD_KEYS: &[&str] = &["↵ fix", FIX_ALL, DEL, BACK, REFRESH, QUIT];
const SKILL_KEYS: &[&str] = &[ON_OFF, ADD, FIX_ALL, DEL, FIND, REFRESH, QUIT];
const UNSET_KEYS: &[&str] = &["s set up", QUIT];

pub(super) fn ui(f: &mut Frame, app: &App) {
    let screen = f.area();
    // Every box is drawn over the rows above the footer, so its key row and
    // bottom border never land on the footer's row.
    let area = Rect {
        height: screen.height.saturating_sub(1),
        ..screen
    };
    let pane = detail_pane(app, screen.width);
    let chunks = Layout::vertical([
        Constraint::Length(3),
        // Three rows of a grid under its header, taken from the detail pane
        // on a short screen, so the selection is never scrolled out of sight.
        Constraint::Min(7),
        Constraint::Length(pane.as_ref().map_or(0, |(_, h)| *h)),
        Constraint::Length(1),
    ])
    .split(screen);
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
        None if app.view == View::Mcp => render_mcp(f, chunks[1], app),
        None => render_agents(f, chunks[1], app),
    }
    render_status(f, chunks[3], app);

    if let Some(w) = &app.wizard {
        render_wizard(f, area, w);
    }
    if let Some(sc) = &app.scope {
        render_scope(f, area, sc);
    }
    if let (Some(form), Some(cfg)) = (&app.server_form, &app.cfg) {
        render_server_form(f, area, form, &tilde(&cfg.source.mcp));
    }
    if let Some(c) = &app.confirm {
        render_confirm(f, area, c);
    }
    if let Some(t) = &app.typed {
        render_typed(f, area, t);
    }
    if app.show_help {
        render_help(f, area, app);
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
                format!("{n} problem{}", if n == 1 { "" } else { "s" }),
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    // The tabs on the left, where the canon is on the right, in one frame.
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" canonize · canon ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let cols = Layout::horizontal([Constraint::Length(42), Constraint::Min(0)]).split(inner);
    let idx = match app.view {
        View::Agents => 0,
        View::Skills => 1,
        View::Houses => 2,
        View::Mcp => 3,
    };
    let tabs = Tabs::new(vec!["Agents", "Skills", "Conventions", "MCPs"])
        .select(idx)
        .divider("│")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, cols[0]);
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        cols[1],
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

    let shown = app.agent_rows();
    let items: Vec<Line> = shown
        .iter()
        .map(|&i| (i, &cfg.agents[i]))
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

    let Some(agent) = cfg.agents.get(app.col).filter(|_| shown.contains(&app.col)) else {
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
                        _ if per_project => "through each project's CLAUDE.local.md".to_string(),
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
    let rows_idx = app.skill_list();
    if plan.skill_rows().is_empty() {
        let para = Paragraph::new(
            "No skills yet: put a folder with a SKILL.md in your canon's skills/, and a skill an agent keeps itself shows here as own, for ↵ to adopt.",
        )
        .style(dim)
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }
    let label_w = plan
        .skill_rows()
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
    render_grid(f, area, table, 2, total, app.srow, &app.stop);
}

/// Draws a grid scrolled so the `selected` row is on screen, starting from
/// `top` and writing back where it ended up, with a scrollbar on the right
/// border whenever some of the `total` rows are out of sight. `header_h` is
/// the rows the table's header takes, its margin included.
fn render_grid(
    f: &mut Frame,
    area: Rect,
    table: Table,
    header_h: u16,
    total: usize,
    selected: usize,
    top: &Kept<usize>,
) {
    let mut state = TableState::new()
        .with_offset(top.get())
        .with_selected(Some(selected));
    f.render_stateful_widget(table, area, &mut state);
    top.set(state.offset());
    let viewport = area.height.saturating_sub(2 + header_h) as usize;
    // Measured from below the header, so the bar runs beside the rows alone.
    let rows = Rect {
        y: area.y + header_h,
        height: area.height.saturating_sub(header_h),
        ..area
    };
    vscrollbar(f, rows, total, state.offset(), viewport);
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
        let pane = |r: HouseRow, c: usize| match r {
            HouseRow::Agent(a) => house_detail(app, a, c),
            HouseRow::Project(i) => project_detail(app, i, c),
        };
        let all = app
            .all_places()
            .into_iter()
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .filter_map(|(r, c)| pane(r, c))
            .collect();
        return Some((pane(app.house_row()?, app.pcol)?, tallest(all)));
    }
    if app.view == View::Mcp {
        let cols = app.mcps.as_ref()?.servers.len();
        let all = app
            .all_places()
            .into_iter()
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .filter_map(|(r, c)| mcp_detail(app, r, c))
            .collect();
        let at = app.house_row_at(app.mrow)?;
        return Some((mcp_detail(app, at, app.mcol)?, tallest(all)));
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
    // A filter that hides every row leaves nothing selected to explain.
    let hidden = match app.view {
        View::Skills => app.skill_list().is_empty(),
        _ => !app.agent_rows().contains(&app.col),
    };
    if hidden {
        return None;
    }
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
            "claude" => "per project: `@CANON.md` in its CLAUDE.local.md (conventions tab)".into(),
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
    let dim = Style::default().add_modifier(Modifier::DIM);
    // While `/` is being typed the row belongs to the query, the only place
    // what was typed shows.
    if app.searching {
        let mut spans = vec![Span::styled(
            " /",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
        spans.extend(line_edit::with_cursor(
            &app.query,
            app.query_back,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        // Every letter goes into the query here, so only keys that are not
        // letters are offered.
        spans.push(Span::styled(
            format!("   {} match   ↵ keep{SEP}{BACK}", app.row_count()),
            dim,
        ));
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }
    if let Some(msg) = app.live_status() {
        let colour = if app.status_failed {
            Color::Yellow
        } else {
            Color::Green
        };
        f.render_widget(
            Paragraph::new(format!(" {msg}")).style(Style::default().fg(colour)),
            area,
        );
        return;
    }
    // `o open` only where the row has a file to open.
    let with_open = |file: Option<std::path::PathBuf>, extra: &[&'static str]| {
        let mut keys = vec![ON_OFF, ADD];
        if file.is_some() {
            keys.push(OPEN);
        }
        keys.push(FIX_ALL);
        keys.extend_from_slice(extra);
        keys.extend([DEL, FIND, REFRESH, QUIT]);
        keys
    };
    let keys: Vec<&str> = if app.load_error.is_some() {
        UNSET_KEYS.to_vec()
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
        with_open(file, &[])
    } else if app.view == View::Mcp {
        let file = match app.house_row_at(app.mrow) {
            Some(HouseRow::Agent(a)) => app
                .mcps
                .as_ref()
                .and_then(|m| m.cells.get(app.mcol)?.get(a))
                .map(|c| c.at.clone())
                .filter(|at| {
                    at.is_file() && app.cfg.as_ref().is_some_and(|c| *at != c.claude_state)
                }),
            _ => None,
        };
        with_open(file, &[CREATE, EDIT])
    } else if app.view == View::Skills {
        SKILL_KEYS.to_vec()
    } else if app.in_card {
        CARD_KEYS.to_vec()
    } else {
        AGENT_KEYS.to_vec()
    };
    // A committed filter stays in front of the keys: rows are hidden, and
    // nothing else on screen would say why.
    let lead = match app.query.is_empty() || app.in_card {
        true => Vec::new(),
        false => vec![format!("/{}", app.query), BACK.to_string()],
    };
    f.render_widget(Paragraph::new(key_footer(&lead, &keys, area.width)), area);
}

/// One group of the help panel: a heading, then `(keys, what they do)` rows,
/// where a row with no keys is a note about the group.
type HelpSection = (&'static str, &'static [(&'static str, &'static str)]);

/// Every key the app answers to, grouped by where it works. The panel scrolls,
/// so a new row costs nothing but its line.
const HELP: &[HelpSection] = &[
    (
        "moving",
        &[
            ("j/k ↑↓", "move a row"),
            ("h/l ←→", "move a column"),
            ("g G home end", "the first, last row"),
            ("tab shift-tab", "the next, previous tab"),
            ("space", "the same as ↵ in a grid"),
        ],
    ),
    (
        "every tab",
        &[
            ("/", "find in the list, esc drops it"),
            ("r", "refresh"),
            ("v", "validate"),
            ("E", "edit canonize.toml"),
            ("s", "set up"),
            ("?", "this help"),
            ("q ctrl-c", "quit (ctrl-c is esc in a box)"),
        ],
    ),
    (
        "agents",
        &[("↵", "open the agent's card"), ("F", "fix all")],
    ),
    (
        "in an agent's card",
        &[
            ("↵ f", "fix the line"),
            ("d", "delete it"),
            ("esc", "back to the list"),
        ],
    ),
    (
        "skills",
        &[
            ("↵", "on/off, adopts an own one"),
            ("a", "add"),
            ("d", "delete"),
            ("F", "fix all"),
        ],
    ),
    (
        "conventions",
        &[
            ("↵", "on/off"),
            ("f", "fix"),
            ("o", "open the file the import sits in"),
            ("a", "add"),
            ("d", "delete"),
            ("F", "fix all"),
        ],
    ),
    (
        "mcps",
        &[
            ("↵", "on/off"),
            ("f", "fix"),
            ("o", "open the agent's config"),
            ("a", "add"),
            ("d", "delete, or delete the server from the canon"),
            ("c", "create a server"),
            ("e", "edit it"),
            ("F", "fix all"),
        ],
    ),
    (
        "what a cell says",
        &[
            ("", "linked imported added: in place"),
            ("", "unwired broken: F fixes it"),
            ("", "own: ↵ adopts it"),
            ("", "foreign: yours, left alone"),
            ("", "n/a off -: nothing"),
            (
                "",
                "unapproved: imported, but Claude may not load it there yet",
            ),
        ],
    ),
    (
        "in set up",
        &[
            ("↵ tab", "the next question, ↵ finishes on the last"),
            ("shift-tab", "the previous question"),
            ("j/k ↑↓", "pick an answer from a list"),
            ("esc", "cancel set up"),
        ],
    ),
    (
        "in a form",
        &[
            ("ctrl-j/k ↑↓", "the next, previous field"),
            ("tab shift-tab", "the next, previous field"),
            ("h/l ←→", "step a choice"),
            ("↵", "the next field, and submit on the last"),
            ("esc", "cancel"),
        ],
    ),
    (
        "in a box",
        &[
            ("y n", "answer"),
            ("h/l ←→ tab", "move between the buttons"),
            ("↵", "select, or pick from a list"),
            ("j/k ↑↓", "move in a list or scroll a message"),
            ("esc", "cancel, or close a message"),
        ],
    ),
    (
        "in this help",
        &[
            ("j/k ↑↓", "scroll"),
            ("ctrl-d ctrl-u", "half a page down, up"),
            ("g G", "the top, the bottom"),
            ("esc q ?", "close"),
        ],
    ),
];

/// The width of the key column, so every description starts in one place.
const HELP_KEYS: usize = 16;

pub(super) fn help_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (section, entries) in HELP {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            *section,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        for (keys, what) in *entries {
            if keys.is_empty() {
                lines.push(Line::styled(
                    format!("  {what}"),
                    Style::default().add_modifier(Modifier::DIM),
                ));
                continue;
            }
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {keys:<HELP_KEYS$}"),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(*what),
            ]));
        }
    }
    lines
}

/// The help reader: the body scrolls under a key row that never moves, with a
/// scrollbar on the right border once it is taller than the box.
fn render_help(f: &mut Frame, area: Rect, app: &App) {
    let lines = help_lines();
    let width = box_width(area.width);
    // The body, then a blank and the key row.
    let rect = box_area(area, width, box_height(lines.len() as u16 + 2, area.height));
    f.render_widget(Clear, rect);
    let block = box_block(Color::Cyan, "help");
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let shown = inner.height.saturating_sub(2) as usize;
    // Clamped here, where the height is known, so scrolling past the end never
    // piles up presses that then take as many to undo.
    let top = app.help_scroll.get().min(lines.len().saturating_sub(shown));
    app.help_scroll.set(top);
    let body = Rect {
        height: shown as u16,
        ..inner
    };
    f.render_widget(Paragraph::new(lines[top..].to_vec()), body);
    let keys = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(box_hint(READER_KEYS)), keys);
    if lines.len() > shown {
        vscrollbar(f, rect, lines.len(), top, shown);
    }
}

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
/// imported, every agent (for every project) above every project.
fn render_houses(f: &mut Frame, area: Rect, app: &App) {
    let (Some(h), Some(p)) = (&app.houses, &app.projects) else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" conventions · who reads which ");
    if h.house.is_empty() {
        let para = Paragraph::new(
            "No conventions yet: put a CONVENTIONS-<NAME>.md in your canon's conventions/, then import it here into an agent, for every project, or into a project.",
        )
        .style(Style::default().add_modifier(Modifier::DIM))
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }
    let cols = h.house.iter().map(|x| crate::cli::house_label(x)).collect();
    let states = |row: HouseRow| -> Vec<(&'static str, Style)> {
        match row {
            HouseRow::Agent(a) => h
                .cells
                .iter()
                .map(|r| look(house_word, &r[a].state))
                .collect(),
            HouseRow::Project(i) => {
                let x = &p.list[i];
                x.cells
                    .iter()
                    .map(|c| project_look(&c.state, x.unread()))
                    .collect()
            }
        }
    };
    let grid = Places {
        block,
        cols,
        at: (app.prow, app.pcol),
        top: &app.ptop,
    };
    render_places(f, area, app, grid, states);
}

/// The MCP tab: a column per server in the canon, over the houses tab's rows.
fn render_mcp(f: &mut Frame, area: Rect, app: &App) {
    let (Some(cfg), Some(m)) = (&app.cfg, &app.mcps) else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" mcps · who has which server ");
    if m.servers.is_empty() {
        let (text, style) = match &m.error {
            Some(e) => (
                format!("{e}\n\nFix it, then r refreshes."),
                Style::default().fg(Color::Yellow),
            ),
            None if cfg.source.mcp.as_os_str().is_empty() => (
                "MCP servers are turned off: `mcp` is \"\" in canonize.toml (E).".to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ),
            None => (
                format!(
                    "No MCP servers yet: c describes one and writes it into {}, then ↵ adds it here to an agent, for every project, or to a project.",
                    tilde(&cfg.source.mcp)
                ),
                Style::default().add_modifier(Modifier::DIM),
            ),
        };
        let para = Paragraph::new(text)
            .style(style)
            .block(block)
            .wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }
    let cols = m.servers.iter().map(|s| s.name.clone()).collect();
    let word = crate::cli::mcp_word;
    let states = |row: HouseRow| -> Vec<(&'static str, Style)> {
        match row {
            HouseRow::Agent(a) => m.cells.iter().map(|r| look(word, &r[a].state)).collect(),
            HouseRow::Project(i) => m.projects[i].iter().map(|c| look(word, &c.state)).collect(),
        }
    };
    let grid = Places {
        block,
        cols,
        at: (app.mrow, app.mcol),
        top: &app.mtop,
    };
    render_places(f, area, app, grid, states);
}

/// A grid cell's word and style for `state`, a missing one dimmed.
fn look(word: fn(&State) -> &'static str, state: &State) -> (&'static str, Style) {
    let style = if *state == State::Missing {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        state_style(state)
    };
    (word(state), style)
}

/// A project's convention cell, which says why the agents cannot read an
/// import yet in place of `imported`.
fn project_look(state: &State, unread: Option<projects::Unread>) -> (&'static str, Style) {
    match (state, unread) {
        (State::Linked, Some(projects::Unread::Wiring(s))) => (s.word(), state_style(s)),
        (State::Linked, Some(u)) => (u.word(), Style::default().fg(Color::Yellow)),
        _ => look(house_word, state),
    }
}

/// What a grid of places shows: its frame, a column per `cols`, with `at`
/// (row, column) selected and `top` the first row on screen.
struct Places<'t> {
    block: Block<'static>,
    cols: Vec<String>,
    at: (usize, usize),
    top: &'t Kept<usize>,
}

/// A grid of places something can go: a row per agent (for every project)
/// above a row per project, with `states` giving a row's cells.
fn render_places(
    f: &mut Frame,
    area: Rect,
    app: &App,
    grid: Places,
    states: impl Fn(HouseRow) -> Vec<(&'static str, Style)>,
) {
    let Places {
        block,
        cols,
        at,
        top,
    } = grid;
    let (Some(cfg), Some(p)) = (&app.cfg, &app.projects) else {
        return;
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    // The agents' section title sits on the header row, as in `canon status`.
    let title = "every project";
    // Sized from every row, so typing a filter never shifts the columns.
    let label_w = app
        .all_places()
        .into_iter()
        .map(|r| app.place_name(r).chars().count() + 2)
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 1) as u16;
    let mut header = vec![Cell::from(title).style(dim)];
    let mut widths = vec![Constraint::Length(label_w + 1)];
    for label in cols {
        widths.push(Constraint::Length(label.chars().count().max(10) as u16 + 2));
        header.push(Cell::from(label).style(Style::default().add_modifier(Modifier::BOLD)));
    }
    let section = |text: &str| Row::new(vec![Cell::from(text.to_string()).style(dim)]);
    let mut rows = Vec::new();
    // The display row of the selection, past the section lines above it.
    let mut selected = 0;
    let mut projects_seen = false;
    for (r, row) in app.place_rows().into_iter().enumerate() {
        if matches!(row, HouseRow::Project(_)) && !projects_seen {
            projects_seen = true;
            if !rows.is_empty() {
                rows.push(section(""));
            }
            rows.push(section("projects"));
        }
        if r == at.0 {
            selected = rows.len();
        }
        let mut cells = vec![Cell::from(format!("  {}", app.place_name(row)))];
        for (c, (word, mut style)) in states(row).into_iter().enumerate() {
            if r == at.0 && c == at.1 {
                style = style.add_modifier(Modifier::REVERSED);
            }
            cells.push(Cell::from(format!(" {word} ")).style(style));
        }
        rows.push(Row::new(cells));
    }
    if p.list.is_empty() {
        rows.push(section(""));
        rows.push(section(if cfg.projects.is_empty() {
            "projects: none yet, add `projects = [\"~/dev\"]` to canonize.toml (E)"
        } else {
            "projects: none under your project folders has a CLAUDE.md or AGENTS.md"
        }));
    }
    let total = rows.len();
    let table = Table::new(rows, widths)
        .header(Row::new(header))
        .block(block);
    render_grid(f, area, table, 1, total, selected, top);
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
    lines.push(field("file", tilde(house)));
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
        match (&cell.state, x.unread()) {
            (State::Linked, Some(u)) => {
                format!("{}: in CANON.md, not read here yet", u.word())
            }
            (State::Linked, None) => "imported".into(),
            (State::Broken(why), _) => format!("broken: {why}"),
            _ => "not imported".into(),
        },
    )];
    lines.push(field("project", tilde(&x.root)));
    // Named by its key, because the import itself lives in CANON.md and a row
    // saying `file` beside it reads as the file the import sits in.
    lines.push(field("o opens", tilde(&x.host)));
    lines.push(field("file", tilde(h)));
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
    if let Some(a) = x.claude {
        let (text, style) = match a.advice(cfg, x) {
            None => ("yes".to_string(), state_style(&State::Linked)),
            Some(advice) => (format!("no · {advice}"), Style::default().fg(Color::Yellow)),
        };
        lines.push(Line::from(vec![
            Span::styled("Claude allowed to load them: ", dim),
            Span::styled(text, style),
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

/// Server `col` in row `row` of the MCP tab spelled out: what the server is,
/// where the entry lives, and what Enter would do.
fn mcp_detail(app: &App, row: HouseRow, col: usize) -> Option<Pane> {
    let (cfg, m, p) = (
        app.cfg.as_ref()?,
        app.mcps.as_ref()?,
        app.projects.as_ref()?,
    );
    let server = m.servers.get(col)?;
    let dim = Style::default().add_modifier(Modifier::DIM);
    let field =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<17}"), dim), Span::raw(v)]);
    let (who, state, change, undo, at) = match row {
        HouseRow::Agent(a) => {
            let c = m.cells.get(col)?.get(a)?;
            let at = (!matches!(c.state, State::Na | State::Absent)).then(|| tilde(&c.at));
            (
                cfg.agents.get(a)?.name.clone(),
                &c.state,
                &c.change,
                &c.undo,
                at,
            )
        }
        HouseRow::Project(i) => {
            let c = m.projects.get(i)?.get(col)?;
            let x = p.list.get(i)?;
            (
                projects::short(cfg, &x.root),
                &c.state,
                &c.change,
                &c.undo,
                Some(format!("{}, every agent", tilde(&x.root))),
            )
        }
    };
    let shared = match row {
        HouseRow::Project(i) => m.shared.get(i) == Some(&true),
        HouseRow::Agent(_) => false,
    };
    let word = crate::cli::mcp_word(state);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:<17}", "state"), dim),
        Span::styled(
            match state.why() {
                Some(why) => format!("{word}: {why}"),
                None if *state == State::Missing => "not added".to_string(),
                None if *state == State::Na => {
                    "this agent has no MCP config canonize knows".to_string()
                }
                None => word.to_string(),
            },
            state_style(state),
        ),
    ])];
    lines.push(field("server", server.summary()));
    let vars = server.vars();
    if !vars.is_empty() {
        let unset: Vec<&str> = vars
            .iter()
            .copied()
            .filter(|v| std::env::var_os(v).is_none_or(|x| x.is_empty()))
            .collect();
        let mut text = vars.join(", ");
        if !unset.is_empty() {
            text.push_str(&format!(" (not set here: {})", unset.join(", ")));
        }
        lines.push(field("reads", text));
    }
    if let Some(kept) = server.kept() {
        lines.push(field(
            "token",
            if kept.is_file() {
                "kept outside your canon · e replaces it".to_string()
            } else {
                "none yet · e sets one".to_string()
            },
        ));
    }
    if let Some(at) = at {
        lines.push(field("at", at));
    }
    if shared {
        lines.push(Line::styled(
            "not for Claude: it keeps one list for the whole git repo, on the repo's own row",
            dim,
        ));
    }
    let toggle = if *state == State::Linked {
        undo
    } else {
        change
    };
    match toggle {
        Some(c) => lines.push(field(&format!("↵ {}", c.verb()), c.describe())),
        None => lines.push(field("↵", "nothing to do".to_string())),
    }
    Some(Pane {
        title: format!(" {who} · {} ", server.name),
        lines,
    })
}
