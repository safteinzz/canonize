//! Box furniture with no idea what it is drawing: centering, the wrapped-row
//! count a box needs to size itself, and the house box every overlay is built
//! from.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState};

/// Rough count of how many rows `text` needs when word-wrapped to `width`,
/// matching how ratatui's `Wrap` breaks on spaces. Used to size modal boxes.
pub(super) fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut rows = 1usize;
    let mut col = 0usize;
    // Split on single spaces rather than runs of whitespace: the indent and the
    // gap after "runs" are real columns, and collapsing them undercounted the
    // rows enough to clip the line off the bottom of a wizard.
    for (i, word) in text.split(' ').enumerate() {
        let w = word.chars().count();
        // Only the very first token has no space in front of it. Keying this off
        // `col == 0` instead swallowed a column for every leading space, which
        // undercounted the rows and clipped the last line off a box.
        let need = if i == 0 { w } else { col + 1 + w };
        if need <= width {
            col = need;
        } else {
            rows += 1;
            col = w;
        }
        // A word longer than the whole line wraps across several rows.
        while col > width {
            rows += 1;
            col -= width;
        }
    }
    rows
}

/// Narrowest a box may be, so a two-word message still reads as a box.
pub(super) const BOX_MIN_W: u16 = 24;
/// Widest, so one long line does not stretch a box across a 200-column screen.
pub(super) const BOX_MAX_W: u16 = 88;
/// Rows the chrome costs: two borders plus the single top padding row.
pub(super) const BOX_CHROME_H: u16 = 3;
/// Columns the chrome costs: two borders plus two columns of padding a side.
pub(super) const BOX_CHROME_W: u16 = 6;

/// How wide a box is on a screen this wide.
pub(super) fn box_width(screen_w: u16) -> u16 {
    screen_w.saturating_sub(4).clamp(BOX_MIN_W, BOX_MAX_W)
}

/// The columns a body actually gets, which is what it must be wrapped to.
pub(super) fn box_inner_width(width: u16) -> usize {
    width.saturating_sub(BOX_CHROME_W).max(1) as usize
}

/// How tall a box holding `body_rows` *wrapped* rows is, capped at the screen.
/// Pass the wrapped count, never the line count: measuring the unwrapped text
/// is what clips a modal's last row off and makes it look unanswerable.
///
/// The cap is the whole screen and not some fraction of it. A box is drawn over
/// a `Clear`, so it owns the screen while it is up anyway, and a fraction only
/// decides in advance that a long one gets cut off.
pub(super) fn box_height(body_rows: u16, screen_h: u16) -> u16 {
    let floor = BOX_CHROME_H + 1;
    body_rows
        .saturating_add(BOX_CHROME_H)
        .clamp(floor, screen_h.max(floor))
}

/// A box of at most `w` by `h`, centered in `area`.
pub(super) fn box_area(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// The bordered block every box wears: a spaced title on the top border, and
/// otherwise an unbroken frame in the colour that says what kind it is.
///
/// Nothing else is written on the border. Keys go in the body, through
/// `box_hint`: a frame with a sentence along the bottom of it stops reading as
/// a frame, and the title then has to compete with it.
pub(super) fn box_block(colour: Color, title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colour))
        // One blank row above the content and none below it: the key line is
        // the last thing in the box, and a blank under it is a wasted row that
        // makes the frame look loose.
        .padding(Padding::new(2, 2, 1, 0))
        .title(format!(" {} ", title.trim()))
}

// The words every footer and key row is built from, so the same key reads the
// same on every screen and a hand-typed legend stands out.
pub(super) const CREATE: &str = "c create";
pub(super) const EDIT: &str = "e edit";
pub(super) const DEL: &str = "d del";
pub(super) const FIND: &str = "/ find";
pub(super) const REFRESH: &str = "r refresh";
pub(super) const QUIT: &str = "q quit";
pub(super) const HELP: &str = "? help";
pub(super) const SELECT: &str = "↵ select";
pub(super) const PICK: &str = "↵ pick";
pub(super) const NEXT_SUBMIT: &str = "↵ next/submit";
pub(super) const BACK: &str = "esc back";
pub(super) const CANCEL: &str = "esc cancel";
pub(super) const CLOSE: &str = "esc close";
pub(super) const REQUIRED: &str = "* required";
pub(super) const SEP: &str = " · ";

/// The key rows of the box kinds, each drawn by that kind's render function.
pub(super) const GATE_KEYS: &[&str] = &[SELECT, CANCEL];
pub(super) const TYPED_KEYS: &[&str] = &["↵ del", CANCEL];
pub(super) const PICKER_KEYS: &[&str] = &[PICK, CANCEL];
pub(super) const FORM_KEYS: &[&str] = &[NEXT_SUBMIT, CANCEL];
pub(super) const READER_KEYS: &[&str] = &[CLOSE];

/// The line of keys a box ends with, as the last row of its body. Every kind
/// puts it in the same place, so it is where the eye already is.
///
/// Quieter than anything else in the box, deliberately: an unfocused field
/// label is the default foreground dimmed, so this goes a step below that with
/// `DarkGray` dimmed again. Separation is the blank row above it and its fixed
/// place at the bottom, not brightness. Colouring it only made a guideline look
/// like something worth reading.
pub(super) fn box_hint(keys: &[&str]) -> Line<'static> {
    Line::from(Span::styled(
        keys.join(SEP),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    ))
}

/// The row under a view: `lead` (what a committed filter is, say) and then as
/// many of `keys` as fit in `width`, whole and in order, with `? help` pinned
/// at the right edge however narrow it gets, since help is the way to every key
/// that fell off.
pub(super) fn key_footer(lead: &[String], keys: &[&str], width: u16) -> Line<'static> {
    let width = width as usize;
    // One column of margin at each end, and a gap before `? help` as wide as a
    // separator, so it never reads as part of the last key.
    let room = width.saturating_sub(HELP.chars().count() + 2 + SEP.chars().count());
    let mut left = String::new();
    let entries = lead.iter().map(String::as_str).chain(keys.iter().copied());
    for entry in entries {
        let next = if left.is_empty() {
            entry.to_string()
        } else {
            format!("{left}{SEP}{entry}")
        };
        if next.chars().count() > room {
            break;
        }
        left = next;
    }
    let pad = width.saturating_sub(left.chars().count() + HELP.chars().count() + 2);
    let dim = Style::default().add_modifier(Modifier::DIM);
    Line::from(vec![
        Span::styled(format!(" {left}"), dim),
        Span::raw(" ".repeat(pad.max(1))),
        Span::styled(format!("{HELP} "), dim),
    ])
}

/// The Yes/No row a gate and an offer share. The labels carry their keys, so
/// the hint line does not have to teach them twice, and the picked one is
/// filled with the border colour rather than merely reversed: a reversed
/// button reads as "selected", a filled one reads as "this is what Enter does".
pub(super) fn box_buttons(colour: Color, yes: bool) -> Line<'static> {
    let button = |label: &str, picked: bool| {
        let style = if picked {
            Style::default()
                .fg(Color::Black)
                .bg(colour)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        Span::styled(format!(" {label} "), style)
    };
    Line::from(vec![
        button("Yes (y)", yes),
        Span::raw("  "),
        button("No (n)", !yes),
    ])
}

/// Wrapped rows for a body of several lines, each wrapped on its own the way
/// the paragraph will draw it.
pub(super) fn wrapped_rows(text: &str, width: usize) -> u16 {
    text.lines()
        .map(|l| wrapped_line_count(l, width))
        .sum::<usize>()
        .max(1) as u16
}

/// A scrollbar on `area`'s right border for `total` rows, `view` of them on
/// screen from `top`, where `area` is the bordered rect it sits on. Nothing is
/// drawn when every row fits.
pub(super) fn vscrollbar(f: &mut Frame, area: Rect, total: usize, top: usize, view: usize) {
    if total <= view {
        return;
    }
    let mut state = ScrollbarState::new(total - view).position(top);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None);
    f.render_stateful_widget(bar, area.inner(Margin::new(0, 1)), &mut state);
}
