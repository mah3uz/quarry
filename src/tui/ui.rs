use ratatui::Frame;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use super::app::{App, Command, Focus, Level, Overlay};
use super::dialogs::ModalLayout;
use super::keymap::Action;
use super::sidebar::compact_count;
use super::tabs::*;
use crate::complete::SuggestionKind;
use crate::db::Backend;
use super::widgets::bar::Pill;
use super::widgets::editor::VimMode;
use crate::icons::{self, Icons};
use crate::repl::prompt::human_duration;
use crate::theme::Theme;

/// Swaps the glyphs quarry draws for their ASCII stand-ins, in the cells `chrome` accepts.
fn to_ascii(buf: &mut Buffer, area: Rect, chrome: impl Fn(Position, &Cell) -> bool) {
    for pos in area.positions() {
        let cell = &mut buf[pos];
        let mut chars = cell.symbol().chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(plain) = crate::icons::ascii_for(c)
            && chrome(pos, cell)
        {
            cell.set_symbol(plain);
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let theme = app.theme.clone();
    let buf = f.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg).fg(theme.fg));
    app.areas = Default::default();
    let transparent = app.config.main.transparent;
    if area.height < 6 || area.width < 30 {
        buf.set_string(area.x, area.y, "Terminal too small", Style::default().fg(theme.warning));
        return;
    }
    let [header, body, status] = Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
    draw_header(buf, header, app, &theme);
    let (side, main) = if app.sidebar_visible {
        let w = app.sidebar_width.min(body.width / 2);
        let [s, m] = Layout::horizontal([Constraint::Length(w), Constraint::Min(10)]).areas(body);
        (Some(s), m)
    } else {
        (None, body)
    };
    if let Some(s) = side {
        draw_sidebar(buf, s, app, &theme);
    }
    app.areas.main = main;
    let mut cursor = draw_main(buf, main, app, &theme);
    draw_status(buf, status, app, &theme);
    // The editor and the grid hold the user's text and data, which stay as they are; the popups
    // drawn over them from here on are found again by what they changed.
    let ascii = crate::icons::current() == crate::icons::IconSet::Ascii;
    let under = ascii.then(|| {
        let (editor, grid) = (app.areas.editor, app.areas.grid);
        to_ascii(buf, area, |pos, _| !editor.contains(pos) && !grid.contains(pos));
        buf.clone()
    });
    if let Some(pos) = draw_completion(buf, area, app, &theme) {
        cursor = Some(pos);
    }
    draw_toasts(buf, area, app, &theme);
    if let Some(c) = draw_overlay(buf, area, app, &theme) {
        cursor = Some(c);
    } else if app.overlay.is_some() {
        cursor = None;
    }
    if let Some(under) = under {
        to_ascii(f.buffer_mut(), area, |pos, cell| *cell != under[pos]);
    }
    if transparent {
        clear_theme_background(f.buffer_mut(), area, &theme);
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

fn draw_header(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let bar = theme.surface;
    buf.set_style(area, Style::default().bg(bar));
    let ic = icons::get();
    let right_edge = area.x + area.width;
    let y = area.y;

    let theme_label = format!("{} {}", ic.palette, theme.name);
    let cmd_label = format!("{} ^P", ic.commands);
    let theme_pill = Pill { text: &theme_label, fg: theme.accent2, bg: theme.highlight, bold: false };
    let cmd_pill = Pill { text: &cmd_label, fg: theme.bg, bg: theme.accent2, bold: true };
    let right_w = theme_pill.width() + cmd_pill.width() + 2;
    let tabs_limit = right_edge.saturating_sub(right_w + 1);

    let logo = format!("{} quarry", ic.logo);
    let r = Pill { text: &logo, fg: theme.bg, bg: theme.accent, bold: true }.draw(buf, area.x, y, bar, right_edge);
    let mut x = r.x + r.width + 1;
    for (i, tab) in app.tabs.iter().enumerate() {
        let active = i == app.active;
        let busy = if tab.is_busy() { format!(" {}", app.spinner()) } else { String::new() };
        let dirty = matches!(&tab.kind, TabKind::Table(t) if t.dirty());
        let label = format!(
            "{} {}{}{} {}",
            tab.icon(),
            truncate(&tab.title, 22),
            if dirty { format!(" {}", ic.dirty) } else { String::new() },
            busy,
            ic.close
        );
        let (fg, bg) = if active { (theme.fg, theme.highlight) } else { (theme.muted, bar) };
        let pill = Pill { text: &label, fg, bg, bold: active };
        if x + pill.width() + 4 > tabs_limit {
            buf.set_string(x, y, "…", Style::default().fg(theme.muted).bg(bar));
            x += 2;
            break;
        }
        let r = pill.draw(buf, x, y, bar, tabs_limit);
        if active {
            buf.set_string(r.x + 1, y, tab.icon(), Style::default().fg(theme.accent).bg(bg));
        }
        let close_x = r.x + r.width - 1 - ic.cap_right.width().max(1) as u16;
        app.areas.buttons.push((Rect { x: close_x, y, width: 1, height: 1 }, Command::CloseTabAt(i)));
        app.areas.header_tabs.push((r, i));
        x += r.width + 1;
    }
    if x + 3 <= tabs_limit {
        buf.set_string(x, y, format!(" {} ", ic.add), Style::default().fg(theme.muted).bg(bar));
        app.areas.buttons.push((Rect { x, y, width: 3, height: 1 }, Command::NewQuery));
    }

    let mut rx = right_edge.saturating_sub(cmd_pill.width() + 1);
    if rx >= x + 3 {
        let r = cmd_pill.draw(buf, rx, y, bar, right_edge);
        app.areas.buttons.push((r, Command::Commands));
        rx = rx.saturating_sub(theme_pill.width() + 1);
        if rx >= x + 3 {
            let r = theme_pill.draw(buf, rx, y, bar, right_edge);
            app.areas.buttons.push((r, Command::Themes));
        }
    }
}

fn draw_sidebar(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let focused = app.focus == Focus::Sidebar;
    buf.set_style(area, Style::default().bg(theme.surface));
    let inner = Rect { width: area.width.saturating_sub(1), ..area };
    let title_style = if focused {
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted).add_modifier(Modifier::BOLD)
    };
    buf.set_string(inner.x + 1, inner.y, "EXPLORER", title_style);
    let loading = app.conns.iter().flatten().any(|c| c.loading_catalog);
    if loading {
        buf.set_string(inner.x + inner.width.saturating_sub(2), inner.y, app.spinner(), Style::default().fg(theme.accent));
    }
    let list = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
    app.areas.sidebar = list;
    if app.sidebar.roots.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(" No connections", Style::default().fg(theme.muted))),
            Line::from(""),
            Line::from(vec![Span::styled(" Ctrl+O", Style::default().fg(theme.accent)), Span::styled(" connect", Style::default().fg(theme.muted))]),
        ]);
        p.render(list, buf);
        return;
    }
    app.sidebar.render(list, buf, theme, focused);
}

fn pane_block<'a>(title: Vec<Span<'a>>, focused: bool, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { theme.border_focus } else { theme.border }))
        .title(Line::from(title))
        .style(Style::default().bg(theme.bg))
}

fn draw_main(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) -> Option<(u16, u16)> {
    let focused = app.focus == Focus::Main;
    let spinner = app.spinner();
    let tick = app.spinner;
    let conn_label = app.active_tab().map(|t| app.tab_conn_label(t)).unwrap_or_default();
    let backend = app.active_tab().and_then(|t| t.conn).and_then(|c| app.conn(c)).map(|c| c.backend());
    let max_rows = app.max_rows;
    let Some(tab) = app.tabs.get_mut(app.active) else {
        draw_welcome(buf, area, theme);
        return None;
    };
    let mut cursor = None;
    match &mut tab.kind {
        TabKind::Query(q) => {
            let editor_h = (area.height as u32 * q.split as u32 / 100) as u16;
            let [ea, ra] = Layout::vertical([Constraint::Length(editor_h.max(4)), Constraint::Min(3)]).areas(area);
            let ef = focused && q.pane == Pane::Editor;
            let (row, col) = q.editor.cursor_pos();
            let mut title = vec![
                Span::styled(format!(" {} ", icons::get().query), Style::default().fg(theme.accent)),
                Span::styled(tab.title.clone(), Style::default().fg(if ef { theme.fg } else { theme.muted }).add_modifier(Modifier::BOLD)),
            ];
            if !conn_label.is_empty() {
                title.push(Span::styled(format!(" · {conn_label} "), Style::default().fg(theme.muted)));
            } else {
                title.push(Span::styled(" · not connected ", Style::default().fg(theme.warning)));
            }
            let block = pane_block(title, ef, theme).title_bottom(
                Line::from(Span::styled(format!(" Ln {}, Col {} ", row + 1, col + 1), Style::default().fg(theme.muted))).right_aligned(),
            );
            let inner = block.inner(ea);
            block.render(ea, buf);
            // set into the border like the title, so it lines up with the frame
            let ic = icons::get();
            let (label, cmd, color) = if q.running.is_some() {
                (format!(" {} Stop ", ic.stop), Command::Cancel, theme.error)
            } else {
                (format!(" {} Run ", ic.run), Command::RunStatement, theme.success)
            };
            let w = label.width() as u16;
            let px = (ea.x + ea.width).saturating_sub(w + 2);
            if px > ea.x + 30 {
                buf.set_string(px, ea.y, &label, Style::default().fg(color).add_modifier(Modifier::BOLD));
                app.areas.buttons.push((Rect { x: px, y: ea.y, width: w, height: 1 }, cmd));
            }
            app.areas.split = Rect { x: ea.x, y: ea.y + ea.height - 1, width: ea.width, height: 2 };
            app.areas.editor = inner;
            q.editor.render(inner, buf, theme, ef);
            if ef && app.overlay.is_none() {
                cursor = q.editor.cursor_screen_position(inner);
            }
            draw_results(buf, ra, q, focused && q.pane == Pane::Results, theme, spinner, tick, max_rows, &mut app.areas);
        }
        TabKind::Table(t) => {
            let [tool, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(area);
            draw_table_toolbar(buf, tool, t, theme, spinner, backend);
            let block = pane_block(vec![], focused, theme);
            let inner = block.inner(rest);
            block.render(rest, buf);
            app.areas.grid = inner;
            if let Some(e) = &t.error
                && t.grid.column_count() == 0 {
                    Paragraph::new(format!("{} {e}", icons::get().error)).style(Style::default().fg(theme.error)).wrap(Wrap { trim: false }).render(inner, buf);
                    return None;
                }
            if t.grid.column_count() == 0 && t.loading {
                center(buf, inner, &format!("{spinner} Loading {}…", t.name), Style::default().fg(theme.muted));
            } else {
                t.grid.render(inner, buf, theme, focused);
            }
        }
        TabKind::Structure(s) => {
            let [tabs_a, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(area);
            let mut x = tabs_a.x + 1;
            buf.set_string(x, tabs_a.y, format!("{} {}.{} ", icons::get().structure, s.schema, s.name), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD));
            x += (s.schema.width() + s.name.width() + 4) as u16;
            for (i, sec) in StructSection::ALL.iter().enumerate() {
                let count = s.section_count(*sec).map(|n| format!(" {n}")).unwrap_or_default();
                let label = format!(" {}{} ", sec.label(), count);
                let active = *sec == s.section;
                let style = if active {
                    Style::default().bg(theme.selection).fg(theme.fg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted)
                };
                let w = label.width() as u16;
                if x + w > tabs_a.x + tabs_a.width {
                    break;
                }
                buf.set_string(x, tabs_a.y, &label, style);
                app.areas.struct_tabs.push((Rect { x, y: tabs_a.y, width: w, height: 1 }, i));
                x += w + 1;
            }
            let block = pane_block(vec![], focused, theme);
            let inner = block.inner(rest);
            block.render(rest, buf);
            app.areas.grid = inner;
            if let Some(e) = &s.error {
                Paragraph::new(format!("{} {e}", icons::get().error)).style(Style::default().fg(theme.error)).render(inner, buf);
            } else if s.section == StructSection::Ddl {
                match &s.ddl {
                    Some(d) => render_text(d, backend, s.text_scroll, inner, buf, theme),
                    None => center(buf, inner, &format!("{spinner} Loading DDL…"), Style::default().fg(theme.muted)),
                }
            } else if s.details.is_none() {
                center(buf, inner, &format!("{spinner} Loading…"), Style::default().fg(theme.muted));
            } else {
                s.grid.render(inner, buf, theme, focused);
            }
        }
        TabKind::Activity(a) => {
            let [tool, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(area);
            let state = if a.paused { "paused".to_string() } else { format!("every {}s", a.interval.as_secs()) };
            let ago = a.last_refresh.map(|t| format!("refreshed {}s ago", t.elapsed().as_secs())).unwrap_or_default();
            let line = Line::from(vec![
                Span::styled(format!(" {} Sessions ", icons::get().activity), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} · {state} · {ago}", a.grid.row_count()), Style::default().fg(theme.muted)),
                Span::styled("   p pause · r refresh · K kill · Enter details", Style::default().fg(theme.muted)),
            ]);
            line.render(tool, buf);
            let block = pane_block(vec![], focused, theme);
            let inner = block.inner(rest);
            block.render(rest, buf);
            app.areas.grid = inner;
            if let Some(e) = &a.error {
                Paragraph::new(format!("{} {e}", icons::get().error)).style(Style::default().fg(theme.error)).wrap(Wrap { trim: false }).render(inner, buf);
            } else {
                a.grid.render(inner, buf, theme, focused);
            }
        }
        TabKind::Text(t) => {
            let block = pane_block(
                vec![Span::styled(format!(" {} {} ", icons::get().text, tab.title), Style::default().fg(theme.accent))],
                focused,
                theme,
            )
            .title_bottom(Line::from(Span::styled(" y copy · e open in editor · j/k scroll ", Style::default().fg(theme.muted))).right_aligned());
            let inner = block.inner(area);
            block.render(area, buf);
            render_text(&t.text, if t.sql { backend } else { None }, t.scroll, inner, buf, theme);
        }
        TabKind::Explain(x) => {
            let elapsed = x.elapsed.map(human_duration).unwrap_or_else(|| spinner.to_string());
            let first = x.sql.lines().next().unwrap_or("").to_string();
            let block = pane_block(
                vec![
                    Span::styled(format!(" {} Explain{} ", icons::get().explain, if x.analyze { " analyze" } else { "" }), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{} · {elapsed} ", truncate(&first, 60)), Style::default().fg(theme.muted)),
                ],
                focused,
                theme,
            );
            let inner = block.inner(area);
            block.render(area, buf);
            app.areas.list = render_explain(x, inner, buf, theme, focused);
        }
        TabKind::History(h) => {
            let block = pane_block(vec![Span::styled(format!(" {} History ", icons::get().history), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))], focused, theme)
                .title_bottom(Line::from(Span::styled(format!(" type to filter · {} open in new query tab ", crate::icons::enter()), Style::default().fg(theme.muted))).right_aligned());
            let inner = block.inner(area);
            block.render(area, buf);
            buf.set_string(inner.x + 1, inner.y, format!("{} ", icons::get().search), Style::default().fg(theme.accent));
            cursor = h.filter.render(Rect { x: inner.x + 3, y: inner.y, width: inner.width.saturating_sub(4), height: 1 }, buf, theme, focused);
            let list = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
            app.areas.list = list;
            let n = list.height as usize;
            if h.selected < h.offset {
                h.offset = h.selected;
            } else if n > 0 && h.selected >= h.offset + n {
                h.offset = h.selected + 1 - n;
            }
            for (row, &i) in h.filtered.iter().enumerate().skip(h.offset).take(n) {
                let y = list.y + (row - h.offset) as u16;
                let sel = row == h.selected;
                if sel {
                    buf.set_style(Rect { y, height: 1, ..list }, Style::default().bg(theme.selection));
                }
                let sql = &h.entries[i].sql;
                let lines = sql.lines().count();
                let first: String = sql.lines().next().unwrap_or("").to_string();
                let mut spans = vec![Span::styled(format!("{:>5} ", i + 1), Style::default().fg(theme.muted))];
                spans.extend(highlight_spans(&truncate(&first, list.width as usize - 14), backend.unwrap_or(Backend::Postgres), theme));
                if lines > 1 {
                    spans.push(Span::styled(format!("  +{} lines", lines - 1), Style::default().fg(theme.muted)));
                }
                Line::from(spans).render(Rect { y, height: 1, ..list }, buf);
            }
            if h.filtered.is_empty() {
                center(buf, list, "No history yet", Style::default().fg(theme.muted));
            }
        }
    }
    cursor
}

#[allow(clippy::too_many_arguments)]
fn draw_results(
    buf: &mut Buffer,
    area: Rect,
    q: &mut QueryTab,
    focused: bool,
    theme: &Theme,
    spinner: &str,
    tick: usize,
    max_rows: usize,
    areas: &mut super::app::Areas,
) {
    let mut title: Vec<Span> = vec![Span::raw(" ")];
    let mut x = area.x + 2;
    let multi = q.results.len() > 1;
    for (i, r) in q.results.iter().enumerate() {
        let n = if i == q.shown && r.stash.is_none() { q.grid.row_count() } else { r.row_count };
        let label = if multi { format!(" Result {} · {} ", i + 1, fmt_count(n)) } else { format!(" Result · {} rows ", fmt_count(n)) };
        let active = i == q.shown;
        let st = if active {
            Style::default().bg(if focused { theme.selection } else { theme.highlight }).fg(theme.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted)
        };
        let w = label.width() as u16;
        areas.result_tabs.push((Rect { x, y: area.y, width: w, height: 1 }, i));
        x += w + 1;
        title.push(Span::styled(label, st));
        title.push(Span::raw(" "));
        if i > 6 {
            title.push(Span::styled("…", Style::default().fg(theme.muted)));
            break;
        }
    }
    let errors = q.messages.iter().rev().take_while(|m| !m.text.starts_with(icons::get().run)).any(|m| matches!(m.kind, MessageKind::Error));
    let mlabel = format!(" Messages{} ", if errors { format!(" {}", icons::get().error) } else { String::new() });
    let mst = if q.showing_messages() {
        Style::default().bg(if focused { theme.selection } else { theme.highlight }).fg(if errors { theme.error } else { theme.fg }).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(if errors { theme.error } else { theme.muted })
    };
    areas.result_tabs.push((Rect { x, y: area.y, width: mlabel.width() as u16, height: 1 }, q.results.len()));
    title.push(Span::styled(mlabel, mst));
    let status = match &q.running {
        Some(r) if q.asking.is_some() => {
            Span::styled(format!(" {spinner} writing SQL · {} ", human_duration(r.started.elapsed())), Style::default().fg(theme.accent2))
        }
        Some(r) => {
            let step = if r.total > 1 { format!(" {}/{}", r.current + 1, r.total) } else { String::new() };
            Span::styled(format!(" {spinner} running{step} · {} · esc cancel ", human_duration(r.started.elapsed())), Style::default().fg(theme.accent))
        }
        None => match (q.results.get(q.shown), q.last_elapsed) {
            (Some(r), _) if r.truncated => Span::styled(format!(" truncated at {} rows ", fmt_count(max_rows)), Style::default().fg(theme.warning)),
            (_, Some(e)) => Span::styled(format!(" {} ", human_duration(e)), Style::default().fg(theme.muted)),
            _ => Span::raw(""),
        },
    };
    let block = pane_block(title, focused, theme).title(Line::from(status).right_aligned());
    let inner = block.inner(area);
    block.render(area, buf);
    areas.grid = inner;
    if let (Some((who, question)), Some(r)) = (&q.asking, &q.running) {
        draw_asking(buf, inner, theme, spinner, tick, who, question, r.started.elapsed());
        return;
    }
    if q.showing_messages() {
        draw_messages(buf, inner, q, theme);
        return;
    }
    if q.results.is_empty() && q.running.is_some() {
        center(buf, inner, &format!("{spinner} Executing…"), Style::default().fg(theme.muted));
        return;
    }
    q.grid.render(inner, buf, theme, focused);
}

fn draw_messages(buf: &mut Buffer, area: Rect, q: &mut QueryTab, theme: &Theme) {
    if q.messages.is_empty() {
        center(buf, area, "Run a query with Ctrl+Enter (or F5 for everything)", Style::default().fg(theme.muted));
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    for m in &q.messages {
        let (icon, color) = match m.kind {
            MessageKind::Info => (icons::get().note, theme.muted),
            MessageKind::Ok => (icons::get().ok, theme.success),
            MessageKind::Notice => (icons::get().notice, theme.warning),
            MessageKind::Error => (icons::get().error, theme.error),
        };
        for (i, l) in m.text.lines().enumerate() {
            let prefix = if i == 0 { format!("{} {icon} ", m.at.format("%H:%M:%S")) } else { " ".repeat(11) };
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(if i == 0 { color } else { theme.muted })),
                Span::styled(l.to_string(), Style::default().fg(if matches!(m.kind, MessageKind::Error) { theme.error } else { theme.fg })),
            ]));
        }
    }
    let h = area.height as usize;
    let max_scroll = lines.len().saturating_sub(h);
    if q.messages_scroll > max_scroll || q.running.is_some() {
        q.messages_scroll = max_scroll;
    }
    Paragraph::new(lines).scroll((q.messages_scroll as u16, 0)).render(Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area }, buf);
}

fn draw_table_toolbar(buf: &mut Buffer, area: Rect, t: &TableTab, theme: &Theme, spinner: &str, backend: Option<Backend>) {
    let ic = icons::get();
    let mut spans = vec![Span::styled(format!(" {} {}.{} ", ic.table, t.schema, t.name), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))];
    if t.filter.is_empty() {
        spans.push(Span::styled(format!(" {} no filter (f) ", ic.filter), Style::default().fg(theme.muted)));
    } else {
        spans.push(Span::styled(" WHERE ", Style::default().fg(theme.keyword).add_modifier(Modifier::BOLD)));
        spans.extend(highlight_spans(&truncate(&t.filter, 50), backend.unwrap_or(Backend::Postgres), theme));
        spans.push(Span::raw(" "));
    }
    if let Some((c, asc)) = t.order
        && let Some(col) = t.grid.columns().get(c) {
            spans.push(Span::styled(format!(" {} {} {} ", ic.sort, col.name, if asc { ic.asc } else { ic.desc }), Style::default().fg(theme.accent2)));
        }
    let count = match t.total {
        Some(n) => format!(" {} rows", fmt_count(n as usize)),
        None => " counting…".into(),
    };
    spans.push(Span::styled(count, Style::default().fg(theme.fg)));
    spans.push(Span::styled(format!(" · {} loaded", fmt_count(t.loaded)), Style::default().fg(theme.muted)));
    if let Some(e) = t.last_elapsed {
        spans.push(Span::styled(format!(" · {}", human_duration(e)), Style::default().fg(theme.muted)));
    }
    if t.loading {
        spans.push(Span::styled(format!(" {spinner}"), Style::default().fg(theme.accent)));
    }
    if t.dirty() {
        spans.push(Span::styled(
            format!("  {} {} pending · Ctrl+S review · u discard ", ic.dirty, t.pending_count()),
            Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans).render(area, buf);
}

fn draw_welcome(buf: &mut Buffer, area: Rect, theme: &Theme) {
    let art = [
        "  ██████  ██    ██  █████  ██████  ██████  ██    ██ ",
        " ██    ██ ██    ██ ██   ██ ██   ██ ██   ██  ██  ██  ",
        " ██    ██ ██    ██ ███████ ██████  ██████    ████   ",
        " ██ ▄▄ ██ ██    ██ ██   ██ ██   ██ ██   ██    ██    ",
        "  ██████   ██████  ██   ██ ██   ██ ██   ██    ██    ",
        "     ▀▀                                              ",
    ];
    let mut lines: Vec<Line> = art
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let c = if i < 3 { theme.accent } else { theme.accent2 };
            Line::from(Span::styled(*l, Style::default().fg(c))).centered()
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("PostgreSQL · MySQL/MariaDB · SQLite", Style::default().fg(theme.muted))).centered());
    lines.push(Line::from(""));
    for (k, d) in [("Ctrl+O", "connect"), ("Ctrl+P", "commands"), ("Ctrl+Y", "themes"), ("F1", "help"), ("Ctrl+Q", "quit")] {
        lines.push(
            Line::from(vec![
                Span::styled(format!("{k:>8}  "), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{d:<10}"), Style::default().fg(theme.fg)),
            ])
            .centered(),
        );
    }
    let h = lines.len() as u16;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Paragraph::new(lines).render(Rect { y, height: h.min(area.height), ..area }, buf);
}

fn draw_status(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let bar = theme.surface;
    buf.set_style(area, Style::default().bg(bar).fg(theme.fg));
    let ic = icons::get();
    let y = area.y;
    let right_edge = area.x + area.width;
    let (mode, mode_bg) = match (app.focus, app.active_tab().map(|t| &t.kind)) {
        _ if app.vim_mode().is_some() => {
            let m = app.vim_mode().unwrap_or_default();
            let color = match m {
                VimMode::Normal => theme.accent,
                VimMode::Insert => theme.success,
                VimMode::Visual | VimMode::VisualLine => theme.accent2,
            };
            (m.label(), color)
        }
        (Focus::Sidebar, _) => ("EXPLORER", theme.accent2),
        (_, Some(TabKind::Query(q))) if q.pane == Pane::Editor => ("EDITOR", theme.accent),
        (_, Some(TabKind::Query(_))) => ("RESULTS", theme.success),
        (_, Some(TabKind::Table(_))) => ("TABLE", theme.success),
        (_, Some(TabKind::Structure(_))) => ("STRUCTURE", theme.info),
        (_, Some(TabKind::Activity(_))) => ("ACTIVITY", theme.warning),
        (_, Some(TabKind::Explain(_))) => ("EXPLAIN", theme.info),
        (_, Some(TabKind::History(_))) => ("HISTORY", theme.info),
        (_, Some(TabKind::Text(_))) => ("VIEW", theme.info),
        (_, None) => ("READY", theme.muted),
    };
    let r = Pill { text: mode, fg: theme.bg, bg: mode_bg, bold: true }.draw(buf, area.x, y, bar, right_edge);
    let mut x = r.x + r.width + 1;

    let conn = app.active_tab().and_then(|t| t.conn).and_then(|c| app.conn(c));
    let mut buttons = Vec::new();
    match conn {
        Some(c) => {
            let tag = match c.backend() {
                Backend::Postgres => "PG",
                Backend::MySql if c.info.is_mariadb => "MDB",
                Backend::MySql => "MY",
                Backend::Sqlite => "SQ",
            };
            let logo = ic.backend(c.backend(), c.info.is_mariadb);
            let place = app.active_tab().map(|t| app.tab_conn_label(t)).unwrap_or_else(|| c.short_label());
            let label = if logo.is_empty() { format!("{tag} {place}") } else { format!("{logo} {place}") };
            let (fg, bg) = match c.color {
                Some(color) => (theme.bg, color),
                None => (theme.fg, theme.highlight),
            };
            let r = Pill { text: &label, fg, bg, bold: true }.draw(buf, x, y, bar, right_edge);
            buttons.push((r, Command::Connections));
            x += r.width + 1;
            let mut badge = |text: &str, fg: Color, bg: Color, x: &mut u16| {
                let r = Pill { text, fg, bg, bold: true }.draw(buf, *x, y, bar, right_edge);
                *x += r.width + 1;
            };
            if c.in_tx {
                badge(Icons::badge(ic.tx, "TX").trim(), theme.bg, theme.warning, &mut x);
            }
            if c.readonly {
                badge(Icons::badge(ic.ro, "READ-ONLY").trim(), theme.bg, theme.info, &mut x);
            }
            if c.spec.ssh.is_some() {
                badge(Icons::badge(ic.ssh, "ssh").trim(), theme.accent2, bar, &mut x);
            }
            if c.info.tls {
                badge(Icons::badge(ic.tls, "TLS").trim(), theme.success, bar, &mut x);
            }
        }
        None => {
            let r = Pill { text: "not connected", fg: theme.muted, bg: theme.highlight, bold: false }.draw(buf, x, y, bar, right_edge);
            buttons.push((r, Command::Connections));
            x += r.width + 1;
        }
    }
    app.areas.buttons.extend(buttons);

    let clock = chrono::Local::now().format("%H:%M").to_string();
    let clock = if ic.clock.is_empty() { clock } else { format!("{} {clock}", ic.clock) };
    let position = match app.active_tab().map(|t| &t.kind) {
        Some(TabKind::Query(q)) if q.pane == Pane::Editor => None,
        Some(TabKind::Query(q)) if !q.showing_messages() => grid_position(&q.grid),
        Some(TabKind::Table(t)) => grid_position(&t.grid),
        _ => None,
    };
    let clock_pill = Pill { text: &clock, fg: theme.bg, bg: mode_bg, bold: true };
    let mut rx = right_edge.saturating_sub(clock_pill.width());
    clock_pill.draw(buf, rx, y, bar, right_edge);
    if let Some(pos) = &position {
        let p = Pill { text: pos, fg: theme.fg, bg: theme.highlight, bold: false };
        rx = rx.saturating_sub(p.width() + 1);
        p.draw(buf, rx, y, bar, right_edge);
    }

    let k = |a: Action| app.keymap.short(a);
    let hints = match (app.focus, app.active_tab().map(|t| &t.kind)) {
        (Focus::Sidebar, _) => format!("{} open  c console  s structure  g script  / filter", crate::icons::enter()),
        (_, Some(TabKind::Query(q))) if q.pane == Pane::Editor => {
            format!("{} run  {} all  {} explain  {} help", k(Action::RunStatement), k(Action::RunAll), k(Action::Explain), k(Action::Help))
        }
        (_, Some(TabKind::Query(_))) => format!("{} view  y copy  / search  [ ] results  m messages", crate::icons::enter()),
        (_, Some(TabKind::Table(_))) => "f filter  s sort  e edit  o insert  D delete  ^S apply".to_string(),
        _ => format!("{} help", k(Action::Help)),
    };
    let hints = hints.as_str();
    let hw = hints.width() as u16;
    if rx > x + hw + 2 {
        buf.set_string(rx - hw - 2, y, hints, Style::default().fg(theme.muted).bg(bar));
    }
}

/// The card shown while a model writes SQL: a spinner, a shimmering headline, the question, the time.
#[allow(clippy::too_many_arguments)]
fn draw_asking(buf: &mut Buffer, area: Rect, theme: &Theme, spinner: &str, tick: usize, who: &str, question: &str, elapsed: std::time::Duration) {
    let headline = format!("Asking {who}…");
    let w = area.width.saturating_sub(4) as usize;
    let quoted = truncate(&format!("“{question}”"), w.saturating_sub(4).max(8));
    let time = human_duration(elapsed);
    let width = (headline.width() + 2).max(quoted.width()).min(w) as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(3) / 2;
    buf.set_string(x, y, spinner, Style::default().fg(theme.accent2).add_modifier(Modifier::BOLD));
    let colors = crate::theme::shimmer(headline.chars().count(), tick, theme.muted, theme.fg);
    let mut hx = x + 2;
    for (c, color) in headline.chars().zip(colors) {
        if hx >= area.x + area.width {
            break;
        }
        buf.set_string(hx, y, c.to_string(), Style::default().fg(color).add_modifier(Modifier::BOLD));
        hx += c.to_string().width() as u16;
    }
    if area.height >= 3 {
        buf.set_stringn(x + 2, y + 1, &quoted, w, Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC));
        buf.set_string(x + 2, y + 2, &time, Style::default().fg(theme.muted));
    }
}

fn grid_position(g: &super::widgets::grid::GridState) -> Option<String> {
    let (row, _) = g.selected_cell()?;
    Some(format!("{}/{}", fmt_count(row + 1), fmt_count(g.row_count())))
}

fn kind_glyph(kind: SuggestionKind, theme: &Theme) -> (&'static str, Color) {
    let color = match kind {
        SuggestionKind::Keyword => theme.keyword,
        SuggestionKind::Table => theme.accent,
        SuggestionKind::View => theme.info,
        SuggestionKind::Column => theme.identifier,
        SuggestionKind::Schema | SuggestionKind::Database | SuggestionKind::Special => theme.accent2,
        SuggestionKind::Function => theme.function,
        SuggestionKind::DataType => theme.datatype,
        SuggestionKind::Alias | SuggestionKind::User => theme.parameter,
        SuggestionKind::Join | SuggestionKind::JoinCondition => theme.success,
        SuggestionKind::Favorite => theme.warning,
        SuggestionKind::File => theme.string,
    };
    (icons::get().kind(kind), color)
}

fn draw_completion(buf: &mut Buffer, screen: Rect, app: &mut App, theme: &Theme) -> Option<(u16, u16)> {
    let editor_area = app.areas.editor;
    let popup = app.completion.as_mut()?;
    let Some(Tab { kind: TabKind::Query(q), .. }) = app.tabs.get(app.active) else { return None };
    let (cx, cy) = q.editor.cursor_screen_position(editor_area)?;
    let visible = popup.items.len().min(10);
    let label_w = popup.items.iter().map(|s| s.display.width()).max().unwrap_or(4).min(40);
    let detail_w = popup.items.iter().map(|s| s.detail.as_deref().map(|d| d.width()).unwrap_or(0)).max().unwrap_or(0).min(28);
    let width = (label_w + detail_w + 7) as u16;
    let height = visible as u16 + 2;
    let below = cy + 1 + height <= screen.y + screen.height;
    let y = if below { cy + 1 } else { cy.saturating_sub(height) };
    let x = cx.saturating_sub(1).min(screen.x + screen.width.saturating_sub(width));
    let area = Rect { x, y, width: width.min(screen.width), height };
    let inner = super::dialogs::modal(area, buf, theme, None, theme.border_focus);
    app.areas.completion = inner;
    if popup.selected < popup.offset {
        popup.offset = popup.selected;
    } else if popup.selected >= popup.offset + visible {
        popup.offset = popup.selected + 1 - visible;
    }
    for (i, item) in popup.items.iter().enumerate().skip(popup.offset).take(visible) {
        let ry = inner.y + (i - popup.offset) as u16;
        let sel = i == popup.selected;
        let row = Rect { y: ry, height: 1, ..inner };
        if sel {
            buf.set_style(row, Style::default().bg(theme.selection));
        }
        let (g, color) = kind_glyph(item.kind, theme);
        buf.set_string(inner.x + 1, ry, g, Style::default().fg(color).add_modifier(Modifier::BOLD));
        let st = if sel { Style::default().fg(theme.fg).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.fg) };
        buf.set_stringn(inner.x + 3, ry, &item.display, label_w, st);
        if let Some(d) = &item.detail {
            let dx = inner.x + inner.width.saturating_sub(detail_w as u16 + 1);
            buf.set_stringn(dx, ry, d, detail_w, Style::default().fg(theme.muted));
        }
    }
    if popup.items.len() > visible {
        let n = format!("{}/{}", popup.selected + 1, popup.items.len());
        buf.set_string(area.x + area.width.saturating_sub(n.width() as u16 + 2), area.y + area.height - 1, n, Style::default().fg(theme.muted).bg(theme.surface));
    }
    None
}

fn draw_toasts(buf: &mut Buffer, screen: Rect, app: &App, theme: &Theme) {
    let mut y = (screen.y + screen.height).saturating_sub(3);
    for t in app.toasts.iter().rev() {
        let (icon, color) = match t.level {
            Level::Info => (icons::get().info, theme.info),
            Level::Success => (icons::get().ok, theme.success),
            Level::Warning => (icons::get().warning, theme.warning),
            Level::Error => (icons::get().error, theme.error),
        };
        let text = truncate(&t.text.replace('\n', " "), (screen.width as usize).saturating_sub(12).min(70));
        let w = text.width() as u16 + 5;
        let x = screen.x + screen.width.saturating_sub(w + 2);
        let area = Rect { x, y, width: w, height: 1 };
        Clear.render(area, buf);
        buf.set_style(area, Style::default().bg(theme.surface).fg(theme.fg));
        buf.set_string(x, y, "▌", Style::default().fg(color).bg(theme.surface));
        buf.set_string(x + 1, y, format!(" {icon} "), Style::default().fg(color).bg(theme.surface).add_modifier(Modifier::BOLD));
        buf.set_string(x + 4, y, &text, Style::default().fg(theme.fg).bg(theme.surface));
        if y <= screen.y + screen.height / 2 {
            break;
        }
        y -= 1;
    }
}

fn draw_overlay(buf: &mut Buffer, screen: Rect, app: &mut App, theme: &Theme) -> Option<(u16, u16)> {
    let overlay = app.overlay.as_mut()?;
    dim(buf, screen);
    let (layout, cursor) = match overlay {
        Overlay::Commands(p) => {
            let c = p.render(screen, buf, theme);
            (ModalLayout { area: p.area(), ..Default::default() }, c)
        }
        Overlay::Themes(p, _) => {
            let c = p.render(screen, buf, theme);
            (ModalLayout { area: p.area(), ..Default::default() }, c)
        }
        Overlay::Help(h) => h.render(screen, buf, theme),
        Overlay::Confirm(c) => (c.render(screen, buf, theme), None),
        Overlay::Prompt(p) => p.render(screen, buf, theme),
        Overlay::Text(t) => (t.render(screen, buf, theme), None),
        Overlay::Connect(c) => c.render(screen, buf, theme),
    };
    app.areas.modal = layout;
    cursor
}

/// Lets the terminal's background through wherever the theme's background or panel colour was
/// painted; text colours, selections and pills are untouched.
fn clear_theme_background(buf: &mut Buffer, area: Rect, theme: &Theme) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            if cell.bg == theme.bg || cell.bg == theme.surface {
                cell.bg = Color::Reset;
            }
        }
    }
}

/// Darkens the background behind a modal so it reads as a layer.
fn dim(buf: &mut Buffer, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_style(Style::default().add_modifier(Modifier::DIM));
        }
    }
}

fn center(buf: &mut Buffer, area: Rect, text: &str, style: Style) {
    let w = text.width() as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height / 2;
    if y < area.y + area.height {
        buf.set_stringn(x, y, text, area.width as usize, style);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn fmt_count(n: usize) -> String {
    if n < 10_000 {
        let s = n.to_string();
        if n >= 1000 {
            return format!("{},{}", &s[..s.len() - 3], &s[s.len() - 3..]);
        }
        return s;
    }
    compact_count(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A translucent terminal must show through the app's own background and panels, while
    /// selections keep their colour so the cursor row stays visible.
    #[test]
    fn transparent_mode_clears_only_theme_backgrounds() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        buf[(0, 0)].set_bg(theme.bg);
        buf[(1, 0)].set_bg(theme.surface);
        buf[(2, 0)].set_bg(theme.selection).set_fg(theme.bg);
        clear_theme_background(&mut buf, area, &theme);
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
        assert_eq!(buf[(1, 0)].bg, Color::Reset);
        assert_eq!(buf[(2, 0)].bg, theme.selection);
        assert_eq!(buf[(2, 0)].fg, theme.bg, "text drawn in the background colour keeps it");
    }

    #[test]
    fn the_ascii_set_redraws_quarrys_glyphs_but_not_the_users_data() {
        let area = Rect::new(0, 0, 12, 2);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, "╭─ Query ──╮", Style::default());
        buf.set_string(0, 1, "│ a — b… é │", Style::default());
        let data = Rect::new(1, 1, 10, 1);
        to_ascii(&mut buf, area, |pos, _| !data.contains(pos));
        let row = |y: u16| (0..12).map(|x| buf[(x, y)].symbol()).collect::<String>();
        assert_eq!(row(0), "+- Query --+");
        assert_eq!(row(1), "| a — b… é |", "a dash in a value must not be shown as a different character");
    }
}
