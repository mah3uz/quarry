pub mod app;
pub mod dialogs;
pub mod keymap;
pub mod palette;
pub mod sidebar;
pub mod tabs;
pub mod ui;
pub mod widgets;
pub mod worker;

use std::io::{self, Stdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::cursor::SetCursorStyle;
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::cli::Opened;
use crate::config::Config;

#[allow(clippy::large_enum_variant)]
pub enum Event {
    Term(event::Event),
    App(worker::AppEvent),
}

type Term = Terminal<CrosstermBackend<Stdout>>;

struct TerminalGuard {
    enhanced: bool,
}

impl TerminalGuard {
    fn enter(mouse: bool) -> Result<(Term, TerminalGuard)> {
        terminal::enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
        if mouse {
            execute!(out, EnableMouseCapture)?;
        }
        let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
        if enhanced {
            let _ = execute!(
                out,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                )
            );
        }
        let mut term = Terminal::new(CrosstermBackend::new(out))?;
        // The first diff assumes a blank screen; after the REPL that is not guaranteed.
        term.clear()?;
        Ok((term, TerminalGuard { enhanced }))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal(self.enhanced);
    }
}

fn restore_terminal(enhanced: bool) {
    let mut out = io::stdout();
    if enhanced {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        SetCursorStyle::DefaultUserShape,
        crossterm::cursor::Show
    );
    let _ = terminal::disable_raw_mode();
}

/// Runs the full-screen interface. `initial` is an already-open connection (from args or `\tui`).
/// Choices for this session only (from the command line, or the REPL's current theme) that win over
/// the saved TUI state and the config.
#[derive(Debug, Default)]
pub struct Overrides {
    pub theme: Option<String>,
    pub no_color: bool,
    /// The saved connection the initial connection came from, for its name and color.
    pub connection_name: Option<String>,
}

pub fn run(rt: &tokio::runtime::Runtime, config: Config, initial: Option<Opened>, overrides: Overrides) -> Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal(true);
        prev_hook(info);
    }));

    let (tx, rx) = mpsc::channel::<Event>();
    let (mut term, _guard) = TerminalGuard::enter(config.main.mouse)?;
    let input_tx = tx.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.send(Event::Term(ev)).is_err() {
                break;
            }
        }
    });

    let name = overrides.connection_name.clone();
    let mut app = app::App::new(rt.handle().clone(), config, tx, overrides);
    match initial {
        Some(opened) => app.adopt_connection(opened, name, None),
        None => app.open_connection_manager(),
    }

    let mut cursor_style = None;
    let frame_budget = Duration::from_millis(16);
    let mut last_draw = Instant::now() - frame_budget;
    let mut dirty = true;
    loop {
        if dirty && last_draw.elapsed() >= frame_budget {
            term.draw(|f| ui::draw(f, &mut app))?;
            // vim modes read at a glance: a block cursor in normal and visual mode, a bar while inserting
            let want = app.vim_mode().map(|m| match m {
                widgets::editor::VimMode::Insert => SetCursorStyle::SteadyBar,
                _ => SetCursorStyle::SteadyBlock,
            });
            if want != cursor_style {
                let _ = execute!(io::stdout(), want.unwrap_or(SetCursorStyle::DefaultUserShape));
                cursor_style = want;
            }
            last_draw = Instant::now();
            dirty = false;
        }
        if app.should_quit() {
            break;
        }
        let timeout = if app.is_animating() { Duration::from_millis(80) } else { Duration::from_millis(500) };
        let wait = if dirty { frame_budget.saturating_sub(last_draw.elapsed()) } else { timeout };
        match rx.recv_timeout(wait) {
            Ok(ev) => {
                app.handle(ev);
                // drain bursts (streamed rows, fast typing) before repainting
                let burst_start = Instant::now();
                while burst_start.elapsed() < Duration::from_millis(12) {
                    match rx.try_recv() {
                        Ok(ev) => app.handle(ev),
                        Err(_) => break,
                    }
                }
                dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                app.tick();
                dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    app.shutdown();
    let _ = std::panic::take_hook();
    Ok(())
}
