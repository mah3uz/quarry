use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::prompt::human_duration;
use super::style::Palette;
use crate::theme::{SPINNER, shimmer};

/// An animated `⠋ text… 1.2 s` line on stderr while something slow runs; erased when stopped.
/// Does nothing when stderr isn't a terminal, so scripts and logs stay clean.
pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Spinner {
    pub fn start(text: String, palette: Palette) -> Spinner {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::io::stderr().is_terminal().then(|| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let th = &palette.theme;
                let started = Instant::now();
                let mut err = std::io::stderr();
                let mut tick = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    let mut line = String::from("\r\x1b[2K");
                    line.push_str(&palette.paint(palette.fg(th.accent2).bold(), SPINNER[tick % SPINNER.len()]));
                    line.push(' ');
                    for (c, color) in text.chars().zip(shimmer(text.chars().count(), tick, th.muted, th.fg)) {
                        line.push_str(&palette.paint(palette.fg(color), &c.to_string()));
                    }
                    line.push_str(&palette.muted(&format!("  {}", human_duration(started.elapsed()))));
                    let _ = err.write_all(line.as_bytes());
                    let _ = err.flush();
                    tick += 1;
                    std::thread::sleep(Duration::from_millis(80));
                }
                let _ = err.write_all(b"\r\x1b[2K");
                let _ = err.flush();
            })
        });
        Spinner { stop, handle }
    }

    pub fn stop(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.finish();
    }
}
