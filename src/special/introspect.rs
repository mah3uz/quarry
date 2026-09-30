use super::{Special, Titled};
use crate::db::{Connection, DbResult};

/// Runs an introspection command (`\dt`, `\d t`, `.schema`, `\l`, …) against `conn`.
/// Returns Ok(None) when `cmd` is not an introspection command.
pub async fn run(_conn: &mut Connection, _cmd: &Special) -> DbResult<Option<Vec<Titled>>> {
    todo!()
}
