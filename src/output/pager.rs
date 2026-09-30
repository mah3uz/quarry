use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};

use super::strip_ansi;
use super::table::str_width;

/// Prints `text`, through a pager when stdout is a terminal and the text does not fit on screen.
/// Pager: `pager_cmd`, else `$PAGER`, else `less -SRXF` when installed.
pub fn page_or_print(text: &str, pager_cmd: Option<&str>, enabled: bool) {
    if enabled && io::stdout().is_terminal() && exceeds_screen(text) {
        if let Some(cmd) = pager_command(pager_cmd) {
            if run_pager(&cmd, text).is_ok() {
                return;
            }
        }
    }
    print_text(text);
}

fn exceeds_screen(text: &str) -> bool {
    let Ok((cols, rows)) = crossterm::terminal::size() else {
        return false;
    };
    let (cols, rows) = (cols as usize, rows as usize);
    let mut lines = 0usize;
    for line in text.lines() {
        lines += 1;
        if lines >= rows || str_width(&strip_ansi(line)) > cols {
            return true;
        }
    }
    false
}

fn pager_command(explicit: Option<&str>) -> Option<String> {
    if let Some(p) = explicit.map(str::trim).filter(|p| !p.is_empty()) {
        return Some(p.to_string());
    }
    if let Ok(p) = std::env::var("PAGER") {
        if !p.trim().is_empty() {
            return Some(p);
        }
    }
    which::which("less").ok().map(|_| "less -SRXF".to_string())
}

fn run_pager(cmd: &str, text: &str) -> io::Result<()> {
    let mut command = Command::new("sh");
    command.arg("-c").arg(cmd).stdin(Stdio::piped());
    if std::env::var_os("LESS").is_none() {
        command.env("LESS", "-SRXF");
    }
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(text.as_bytes()) {
            Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
                let _ = child.wait();
                return Err(e);
            }
            _ => {}
        }
    }
    child.wait()?;
    Ok(())
}

/// Writes to stdout, silently stopping when the reader went away (`quarry … | head`).
pub fn print_text(text: &str) {
    let mut out = io::stdout().lock();
    // BrokenPipe or closed stdout: nothing useful left to do.
    let _ = out.write_all(text.as_bytes()).and_then(|_| out.flush());
}
