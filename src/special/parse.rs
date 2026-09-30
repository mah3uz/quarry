use super::{RelFilter, Special};
use crate::conn::url::expand_tilde;
use crate::db::Backend;
use crate::output::{Expanded, TableFormat};
use crate::sql::lexer::{TokenKind, tokenize};

type Parsed = Result<Special, String>;

const DEFAULT_WATCH_SECONDS: f64 = 2.0;

pub(super) fn parse(input: &str, backend: Backend) -> Option<Parsed> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Some(rest) = input.strip_prefix('\\') {
        return parse_backslash(rest, backend);
    }
    if let Some(rest) = input.strip_prefix('.') {
        return (backend == Backend::Sqlite).then(|| parse_dot(rest));
    }
    parse_word(input, backend).or_else(|| parse_suffix(input, backend))
}

/// Splits `name args`; one-character punctuation commands (`\!ls`, `\|cmd`) need no space.
fn split_name(rest: &str) -> (&str, &str) {
    if let Some(c) = rest.chars().next().filter(|c| "!|?#.".contains(*c)) {
        return (&rest[..c.len_utf8()], rest[c.len_utf8()..].trim());
    }
    match rest.find(char::is_whitespace) {
        Some(i) => (&rest[..i], rest[i..].trim()),
        None => (rest, ""),
    }
}

fn strip_semi(s: &str) -> &str {
    s.trim().trim_end_matches(';').trim_end()
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// First whitespace-delimited token, keeping double quotes (they make psql patterns case-sensitive).
fn pattern(args: &str) -> Option<String> {
    let args = args.trim();
    let mut in_quote = false;
    for (i, c) in args.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => return Some(args[..i].to_string()),
            _ => {}
        }
    }
    opt(args)
}

/// First word, unquoting `"…"` / `'…'`, and the raw remainder.
fn take_word(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let first = s.chars().next()?;
    if first == '"' || first == '\'' {
        let body = &s[1..];
        let end = body.find(first)?;
        return Some((body[..end].to_string(), body[end + 1..].trim_start()));
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    Some((s[..end].to_string(), s[end..].trim_start()))
}

fn words(args: &str) -> Result<Vec<String>, String> {
    shell_words::split(args).map_err(|e| format!("cannot parse arguments: {e}"))
}

fn path_arg(args: &str, usage: &str) -> Result<String, String> {
    match words(args)?.as_slice() {
        [p] => Ok(expand_tilde(p).to_string_lossy().into_owned()),
        [] => Err(format!("missing file name (usage: {usage})")),
        _ => Err(format!("expected one file name; quote names with spaces (usage: {usage})")),
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn bool_opt(args: &str, usage: &str) -> Result<Option<bool>, String> {
    match args.trim() {
        "" => Ok(None),
        a => parse_bool(a).map(Some).ok_or_else(|| format!("expected on or off (usage: {usage})")),
    }
}

fn expanded_opt(args: &str) -> Result<Option<Expanded>, String> {
    match args.trim().to_ascii_lowercase().as_str() {
        "" => Ok(None),
        "auto" => Ok(Some(Expanded::Auto)),
        a => match parse_bool(a) {
            Some(true) => Ok(Some(Expanded::On)),
            Some(false) => Ok(Some(Expanded::Off)),
            None => Err("expected on, off or auto (usage: \\x [on|off|auto])".into()),
        },
    }
}

fn format_names() -> String {
    TableFormat::ALL.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}

fn table_format(name: &str) -> Result<TableFormat, String> {
    TableFormat::parse(name).ok_or_else(|| format!("unknown table format '{name}'; available: {}", format_names()))
}

fn format_opt(args: &str) -> Result<Option<TableFormat>, String> {
    match args.trim() {
        "" => Ok(None),
        a => table_format(a).map(Some),
    }
}

/// sqlite3 `.mode` names mapped onto ours.
fn mode_opt(args: &str) -> Result<Option<TableFormat>, String> {
    let mapped = match args.trim() {
        "line" => "vertical",
        "list" => "plain",
        "tabs" => "tsv",
        "column" => "simple",
        "table" => "ascii",
        "insert" => "sql-insert",
        "qbox" => "unicode",
        other => other,
    };
    format_opt(mapped)
}

fn tee(args: &str, usage: &str) -> Parsed {
    let w = words(args)?;
    let (overwrite, rest) = match w.split_first() {
        Some((flag, rest)) if flag == "-o" => (true, rest),
        _ => (false, &w[..]),
    };
    match rest {
        [p] => Ok(Special::Tee { path: expand_tilde(p).to_string_lossy().into_owned(), overwrite }),
        [] => Err(format!("missing file name (usage: {usage})")),
        _ => Err(format!("expected one file name; quote names with spaces (usage: {usage})")),
    }
}

fn once(args: &str, usage: &str) -> Parsed {
    match tee(args, usage)? {
        Special::Tee { path, overwrite } => Ok(Special::Once { path, overwrite }),
        other => Ok(other),
    }
}

/// `[seconds] [-c] [query]`, also psql's `i=N` / `interval=N`.
fn watch(args: &str) -> Parsed {
    let mut seconds = None;
    let mut clear = false;
    let mut rest = args.trim();
    loop {
        let (word, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if word.is_empty() {
            break;
        }
        let num = word.strip_prefix("interval=").or_else(|| word.strip_prefix("i=")).unwrap_or(word);
        if word == "-c" || word == "--clear" {
            clear = true;
        } else if let Ok(n) = num.parse::<f64>() {
            if !(n > 0.0 && n.is_finite()) {
                return Err("watch interval must be a positive number of seconds".into());
            }
            seconds = Some(n);
        } else {
            break;
        }
        rest = after.trim_start();
    }
    let query = strip_semi(rest);
    Ok(Special::Watch {
        seconds: seconds.unwrap_or(DEFAULT_WATCH_SECONDS),
        clear,
        query: (!query.is_empty()).then(|| query.to_string()),
    })
}

fn explain(args: &str) -> Parsed {
    let args = strip_semi(args);
    let (analyze, query) = match args.split_once(char::is_whitespace) {
        Some((w, q)) if w.eq_ignore_ascii_case("analyze") || w.eq_ignore_ascii_case("analyse") => (true, q.trim()),
        _ if args.eq_ignore_ascii_case("analyze") => (true, ""),
        _ => (false, args),
    };
    if query.is_empty() {
        return Err("missing query (usage: \\explain [analyze] query)".into());
    }
    Ok(Special::Explain { analyze, query: query.to_string() })
}

fn export(args: &str) -> Parsed {
    const USAGE: &str = "usage: \\export format file query";
    let (fmt, rest) = take_word(args).ok_or_else(|| format!("missing format ({USAGE})"))?;
    let format = table_format(&fmt)?;
    let (path, rest) = take_word(rest).filter(|(p, _)| !p.is_empty()).ok_or_else(|| format!("missing file ({USAGE})"))?;
    let query = strip_semi(rest);
    if query.is_empty() {
        return Err(format!("missing query ({USAGE})"));
    }
    Ok(Special::Export { format, path: expand_tilde(&path).to_string_lossy().into_owned(), query: query.to_string() })
}

fn favorite(args: &str) -> Parsed {
    let mut w = words(args)?;
    if w.is_empty() {
        return Ok(Special::Favorite { name: None, args: Vec::new() });
    }
    let name = w.remove(0);
    Ok(Special::Favorite { name: Some(name), args: w })
}

fn favorite_save(args: &str) -> Parsed {
    const USAGE: &str = "usage: \\fs name query";
    let (name, query) = take_word(args).filter(|(n, _)| !n.is_empty()).ok_or_else(|| format!("missing name ({USAGE})"))?;
    let query = strip_semi(query);
    if query.is_empty() {
        return Err(format!("missing query ({USAGE})"));
    }
    Ok(Special::FavoriteSave { name, query: query.to_string() })
}

/// Quoting keeps trailing spaces: `\R '\d> '`.
fn prompt_arg(args: &str) -> Option<String> {
    match take_word(args) {
        Some((p, rest)) if args.starts_with(['"', '\'']) && rest.is_empty() => Some(p),
        _ => opt(args),
    }
}

fn connect_target(args: &str) -> Option<String> {
    let args = args.trim();
    if args.starts_with(['"', '\'', '`']) {
        let q = args.chars().next().unwrap_or('"');
        let body = &args[1..];
        return Some(body.split(q).next().unwrap_or(body).to_string()).filter(|s| !s.is_empty());
    }
    opt(args)
}

fn parse_backslash(rest: &str, backend: Backend) -> Option<Parsed> {
    let (name, raw_args) = split_name(rest);
    let args = strip_semi(raw_args);
    let (base, verbose) = match name.strip_suffix('+') {
        Some(b) if !b.is_empty() => (b, true),
        _ => (name, false),
    };
    // psql's `S` modifier (include system objects) is accepted and ignored.
    let base = match base.strip_suffix('S') {
        Some(b) if b.starts_with('d') && !b.is_empty() => b,
        _ => base,
    };
    let rel = |kind| Special::ListRelations { kind, pattern: pattern(args), verbose };
    Some(Ok(match base {
        "g" | "G" => return None,
        "?" | "h" | "help" => Special::Help(opt(args)),
        "q" | "quit" => Special::Quit,
        "l" | "list" => Special::ListDatabases { pattern: pattern(args), verbose },
        "d" | "describe" => Special::Describe { pattern: pattern(args), verbose },
        "dt" => rel(RelFilter::Tables),
        "dv" => rel(RelFilter::Views),
        "dm" => rel(RelFilter::MaterializedViews),
        "dE" => rel(RelFilter::ForeignTables),
        "di" => Special::ListIndexes { pattern: pattern(args), verbose },
        "ds" => Special::ListSequences { pattern: pattern(args) },
        "df" => Special::ListFunctions { pattern: pattern(args), verbose },
        "dn" => Special::ListSchemas { pattern: pattern(args) },
        "du" | "dg" => Special::ListRoles { pattern: pattern(args) },
        "dT" => Special::ListTypes { pattern: pattern(args) },
        "dx" => Special::ListExtensions { pattern: pattern(args) },
        "dp" | "z" => Special::ListPrivileges { pattern: pattern(args) },
        "sf" | "sv" => {
            let kind = if base == "sf" { "function" } else { "view" };
            match opt(args) {
                Some(name) => Special::ShowSource { name, kind: kind.into() },
                None => return Some(Err(format!("missing {kind} name (usage: \\{base}[+] name)"))),
            }
        }
        "schema" => Special::Schema { pattern: pattern(args) },
        "s" | "status" => Special::Status,
        "conninfo" => Special::ConnInfo,
        "c" | "connect" | "u" | "use" => Special::Connect { target: connect_target(args) },
        "x" | "expanded" => return Some(expanded_opt(args).map(Special::Expanded)),
        "timing" | "t" => return Some(bool_opt(args, "\\timing [on|off]").map(Special::Timing)),
        "T" | "tableformat" => return Some(format_opt(args).map(Special::TableFormat)),
        "pager" | "P" => Special::Pager(opt(args)),
        "nopager" => Special::NoPager,
        "tee" => return Some(tee(args, "tee [-o] file")),
        "notee" => Special::NoTee,
        "o" | "once" => return Some(once(args, "\\o [-o] file")),
        "|" | "pipe_once" => match opt(raw_args) {
            Some(command) => Special::PipeOnce { command },
            None => return Some(Err("missing command (usage: \\| command)".into())),
        },
        "e" | "edit" => match path_arg(args, "\\e [file]") {
            Ok(file) => Special::Edit { file: Some(file), query: None },
            Err(_) if args.is_empty() => Special::Edit { file: None, query: None },
            Err(e) => return Some(Err(e)),
        },
        "i" | "ir" | "include" | "." => return Some(path_arg(args, "\\i file").map(|path| Special::Source { path })),
        "clip" => Special::Clip { query: None },
        "watch" => return Some(watch(args)),
        "f" | "n" => return Some(favorite(args)),
        "fs" | "ns" => return Some(favorite_save(args)),
        "fd" | "nd" => match take_word(args).filter(|(n, _)| !n.is_empty()) {
            Some((name, _)) => Special::FavoriteDelete { name },
            None => return Some(Err("missing name (usage: \\fd name)".into())),
        },
        "refresh" | "rehash" | "#" => Special::Refresh,
        "!" => match opt(raw_args) {
            Some(command) => Special::System { command },
            None => return Some(Err("missing command (usage: \\! command)".into())),
        },
        "echo" => Special::Echo(raw_args.to_string()),
        "R" | "prompt" => Special::Prompt(prompt_arg(raw_args)),
        "delimiter" => return Some(delimiter(args)),
        "theme" => Special::Theme(opt(args)),
        "llm" | "ai" => match opt(raw_args) {
            Some(question) => Special::Llm { question },
            None => return Some(Err("missing question (usage: \\llm show the ten newest orders)".into())),
        },
        "format" => Special::Format { query: opt(args) },
        "explain" => return Some(explain(args)),
        "readonly" => return Some(bool_opt(args, "\\readonly [on|off]").map(Special::ReadOnly)),
        "tui" => Special::Tui,
        "export" => return Some(export(args)),
        "history" => match args {
            "" => Special::History(None),
            n => match n.parse() {
                Ok(n) => Special::History(Some(n)),
                Err(_) => return Some(Err("expected a number (usage: \\history [n])".into())),
            },
        },
        "load" if backend == Backend::Sqlite => {
            return Some(path_arg(args, ".load path").map(|path| Special::LoadExtension { path }));
        }
        other => return Some(Err(format!("unknown command \\{other}; type \\? for help"))),
    }))
}

fn delimiter(args: &str) -> Parsed {
    match args.split_whitespace().next() {
        Some(d) => Ok(Special::Delimiter(d.to_string())),
        None => Err("missing delimiter (usage: delimiter string)".into()),
    }
}

fn parse_dot(rest: &str) -> Parsed {
    let (name, raw_args) = split_name(rest);
    let args = strip_semi(raw_args);
    Ok(match name {
        "tables" => Special::ListRelations { kind: RelFilter::All, pattern: pattern(args), verbose: false },
        "views" => Special::ListRelations { kind: RelFilter::Views, pattern: pattern(args), verbose: false },
        "schema" => Special::Schema { pattern: pattern(args) },
        "indexes" | "indices" => Special::ListIndexes { pattern: pattern(args), verbose: false },
        "databases" => Special::ListDatabases { pattern: None, verbose: false },
        "open" => Special::Connect { target: Some(path_arg(args, ".open file")?) },
        "mode" => Special::TableFormat(mode_opt(args)?),
        "read" => Special::Source { path: path_arg(args, ".read file")? },
        "output" if args.is_empty() || args == "stdout" => Special::NoTee,
        "output" => match tee(args, ".output file")? {
            Special::Tee { path, .. } => Special::Tee { path, overwrite: true },
            other => other,
        },
        "once" => match once(args, ".once file")? {
            Special::Once { path, .. } => Special::Once { path, overwrite: true },
            other => other,
        },
        "load" => Special::LoadExtension { path: path_arg(args, ".load path")? },
        "status" => Special::Status,
        "exit" | "quit" => Special::Quit,
        "help" => Special::Help(opt(args)),
        "timer" => Special::Timing(bool_opt(args, ".timer on|off")?),
        other => return Err(format!("unknown command .{other}; type .help for help")),
    })
}

fn parse_word(input: &str, backend: Backend) -> Option<Parsed> {
    let (word, raw_args) = match input.find(char::is_whitespace) {
        Some(i) => (&input[..i], input[i..].trim()),
        None => (input, ""),
    };
    let lower = word.trim_end_matches(';').to_ascii_lowercase();
    let args = strip_semi(raw_args);
    let bare = args.is_empty() && raw_args.trim_start_matches(';').trim().is_empty();
    Some(match lower.as_str() {
        "help" if backend == Backend::MySql && args.starts_with('\'') => return None,
        "help" => Ok(Special::Help(opt(args))),
        "quit" | "exit" if bare => Ok(Special::Quit),
        "use" if backend != Backend::Sqlite => match connect_target(args) {
            Some(db) => Ok(Special::Connect { target: Some(db) }),
            None => Err("missing database name (usage: use database)".into()),
        },
        "source" => path_arg(args, "source file").map(|path| Special::Source { path }),
        "tee" => tee(args, "tee [-o] file"),
        "notee" if bare => Ok(Special::NoTee),
        "pager" => Ok(Special::Pager(opt(args))),
        "nopager" if bare => Ok(Special::NoPager),
        "status" if bare => Ok(Special::Status),
        "rehash" if bare => Ok(Special::Refresh),
        "delimiter" => delimiter(args),
        "system" => match opt(raw_args) {
            Some(command) => Ok(Special::System { command }),
            None => Err("missing command (usage: system command)".into()),
        },
        "prompt" => Ok(Special::Prompt(prompt_arg(raw_args))),
        "watch" => watch(args),
        "describe" | "desc" => return describe_word(args, backend),
        _ => return None,
    })
}

/// `describe t` / `desc schema.t` → Describe; anything else (`DESCRIBE SELECT …`, `DESC t col`) is SQL.
fn describe_word(args: &str, backend: Backend) -> Option<Parsed> {
    if args.is_empty() {
        return Some(Ok(Special::Describe { pattern: None, verbose: false }));
    }
    let toks: Vec<_> = tokenize(args, backend).into_iter().filter(|t| !t.is_trivia()).collect();
    let mut expect_name = true;
    for t in &toks {
        let ok = if expect_name { t.is_word() || t.kind == TokenKind::QuotedIdent } else { t.kind == TokenKind::Dot };
        if !ok {
            return None;
        }
        expect_name = !expect_name;
    }
    if expect_name {
        return None;
    }
    let first = toks[0].text(args).to_ascii_uppercase();
    if toks[0].is_word()
        && matches!(
            first.as_str(),
            "SELECT" | "WITH" | "INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "TABLE" | "VALUES" | "ANALYZE" | "FORMAT"
                | "EXTENDED" | "PARTITIONS"
        )
    {
        return None;
    }
    Some(Ok(Special::Describe { pattern: Some(args.to_string()), verbose: false }))
}

/// `query \e`, `query \clip`, `query \watch [sec]`.
fn parse_suffix(input: &str, backend: Backend) -> Option<Parsed> {
    let toks = tokenize(input, backend);
    let idx = toks.iter().rposition(|t| t.kind == TokenKind::Backslash)?;
    let name_tok = toks.get(idx + 1).filter(|n| n.start == toks[idx].end && n.is_word())?;
    let after = input[name_tok.end..].trim();
    let query = strip_semi(&input[..toks[idx].start]);
    if query.is_empty() {
        return None;
    }
    let query = Some(query.to_string());
    match name_tok.text(input) {
        "e" | "edit" if after.is_empty() => Some(Ok(Special::Edit { file: None, query })),
        "clip" if after.is_empty() => Some(Ok(Special::Clip { query })),
        "watch" => Some(match after {
            "" => Ok(Special::Watch { seconds: DEFAULT_WATCH_SECONDS, clear: false, query }),
            a => match a.parse::<f64>() {
                Ok(n) if n > 0.0 && n.is_finite() => Ok(Special::Watch { seconds: n, clear: false, query }),
                _ => Err("watch interval must be a positive number of seconds (usage: query \\watch [sec])".into()),
            },
        }),
        _ => None,
    }
}
