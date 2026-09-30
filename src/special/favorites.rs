use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Named queries persisted in `~/.config/quarry/favorites.toml`.
/// Placeholders: `$1`, `$2`… positional, `$*` all args, `${name}` from `--name=value` args.
#[derive(Clone, Debug, Default)]
pub struct Favorites {
    pub path: Option<PathBuf>,
    pub queries: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct FavoritesFile {
    queries: BTreeMap<String, String>,
}

impl Favorites {
    /// A missing file is an empty list (created on first save).
    pub fn load(path: PathBuf) -> anyhow::Result<Favorites> {
        let queries = match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str::<FavoritesFile>(&text).with_context(|| format!("{}: invalid favorites file", path.display()))?.queries
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Ok(Favorites { path: Some(path), queries })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = self.path.as_ref().context("favorites have no file path")?;
        let text = toml::to_string_pretty(&FavoritesFile { queries: self.queries.clone() })?;
        crate::config::write_private(path, &text)
    }

    /// Substitutes placeholders; errors when positional args are missing.
    /// Values are inserted verbatim (not quoted) so they can be identifiers, numbers or SQL fragments.
    pub fn expand(&self, name: &str, args: &[String]) -> Result<String, String> {
        let query = self.queries.get(name).ok_or_else(|| format!("no favorite named '{name}'; \\f lists them"))?;
        let mut positional = Vec::new();
        let mut named = HashMap::new();
        for a in args {
            match a.strip_prefix("--").and_then(|kv| kv.split_once('=')) {
                Some((k, v)) => {
                    named.insert(k, v);
                }
                None => positional.push(a.as_str()),
            }
        }
        substitute(name, query, &positional, &named)
    }
}

fn substitute(fav: &str, query: &str, positional: &[&str], named: &HashMap<&str, &str>) -> Result<String, String> {
    let mut out = String::with_capacity(query.len());
    let mut highest = 0usize;
    let mut star = false;
    let mut missing_names = Vec::new();
    let mut rest = query;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.as_bytes().first() {
            Some(d @ b'1'..=b'9') => {
                let n = (d - b'0') as usize;
                highest = highest.max(n);
                out.push_str(positional.get(n - 1).copied().unwrap_or(""));
                rest = &after[1..];
            }
            Some(b'*') => {
                star = true;
                out.push_str(&positional.join(", "));
                rest = &after[1..];
            }
            Some(b'{') if after.find('}').is_some_and(|e| is_name(&after[1..e])) => {
                let end = after.find('}').unwrap_or(0);
                let key = &after[1..end];
                match named.get(key) {
                    Some(v) => out.push_str(v),
                    None => missing_names.push(format!("--{key}=…")),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if positional.len() < highest {
        return Err(format!(
            "favorite '{fav}' needs {highest} positional argument{} ($1…${highest}), got {}",
            if highest == 1 { "" } else { "s" },
            positional.len()
        ));
    }
    if !missing_names.is_empty() {
        return Err(format!("favorite '{fav}' needs {}", missing_names.join(" ")));
    }
    if !star && positional.len() > highest {
        return Err(format!(
            "favorite '{fav}' takes {highest} positional argument{}, got {}",
            if highest == 1 { "" } else { "s" },
            positional.len()
        ));
    }
    Ok(out)
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favs(pairs: &[(&str, &str)]) -> Favorites {
        Favorites { path: None, queries: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn positional_star_and_named() {
        let f = favs(&[
            ("top", "select * from $1 limit $2"),
            ("ids", "select * from t where id in ($*)"),
            ("by", "select * from t where owner = '${user}' and kind = $1"),
        ]);
        assert_eq!(f.expand("top", &args(&["users", "10"])).unwrap(), "select * from users limit 10");
        assert_eq!(f.expand("ids", &args(&["1", "2", "3"])).unwrap(), "select * from t where id in (1, 2, 3)");
        assert_eq!(f.expand("by", &args(&["--user=bob", "7"])).unwrap(), "select * from t where owner = 'bob' and kind = 7");
    }

    #[test]
    fn missing_or_extra_arguments_are_errors_not_silent_blanks() {
        let f = favs(&[("top", "select * from $1 limit $2"), ("plain", "select 1"), ("by", "select '${user}'")]);
        let e = f.expand("top", &args(&["users"])).unwrap_err();
        assert!(e.contains("needs 2 positional arguments"), "{e}");
        assert!(f.expand("plain", &args(&["x"])).unwrap_err().contains("takes 0"));
        assert!(f.expand("by", &[]).unwrap_err().contains("--user="));
        assert!(f.expand("nope", &[]).unwrap_err().contains("no favorite named 'nope'"));
    }

    #[test]
    fn dollar_quoting_and_money_are_left_alone() {
        let f = favs(&[("fn", "select $$a$$, '$', $0, ${not valid}")]);
        assert_eq!(f.expand("fn", &[]).unwrap(), "select $$a$$, '$', $0, ${not valid}");
    }

    #[test]
    fn save_and_load_round_trip_with_private_permissions() {
        let path = std::env::temp_dir().join(format!("quarry-fav-{}/favorites.toml", std::process::id()));
        let mut f = Favorites { path: Some(path.clone()), ..Default::default() };
        f.queries.insert("q".into(), "select \"x\"\nfrom t".into());
        f.save().unwrap();
        let loaded = Favorites::load(path.clone()).unwrap();
        assert_eq!(loaded.queries, f.queries);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        assert!(Favorites::load(std::env::temp_dir().join("quarry-no-such-fav.toml")).unwrap().queries.is_empty());
    }
}
