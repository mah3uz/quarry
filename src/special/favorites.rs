use std::collections::BTreeMap;
use std::path::PathBuf;

/// Named queries persisted in `~/.config/quarry/favorites.toml`.
/// Placeholders: `$1`, `$2`… positional, `$*` all args, `${name}` from `--name=value` args.
#[derive(Clone, Debug, Default)]
pub struct Favorites {
    pub path: Option<PathBuf>,
    pub queries: BTreeMap<String, String>,
}

impl Favorites {
    pub fn load(_path: PathBuf) -> anyhow::Result<Favorites> {
        todo!()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        todo!()
    }

    /// Substitutes placeholders; errors when positional args are missing.
    pub fn expand(&self, _name: &str, _args: &[String]) -> Result<String, String> {
        todo!()
    }
}
