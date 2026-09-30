pub mod widgets;

use anyhow::Result;

use crate::cli::Opened;
use crate::config::Config;

/// Runs the full-screen interface. `initial` is an already-open connection (from args or `\tui`).
pub fn run(_rt: &tokio::runtime::Runtime, _config: Config, _initial: Option<Opened>) -> Result<()> {
    todo!()
}
