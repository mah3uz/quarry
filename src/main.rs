use std::io::{IsTerminal, Read};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;

use quarry::cli::{self, Args, Opened};
use quarry::config::{Config, SavedConnection};
use quarry::output::{Expanded, OutputOptions, TableFormat};
use quarry::output::sink::Sinks;
use quarry::repl::{self, Exit, session::Session, style::Palette};
use quarry::special::favorites::Favorites;
use quarry::theme::{self, ColorDepth, Theme};

fn main() -> ExitCode {
    let args = Args::parse();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("quarry: cannot start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = match real_main(args, &rt) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("quarry: {e:#}");
            ExitCode::FAILURE
        }
    };
    rt.shutdown_timeout(std::time::Duration::from_millis(200));
    code
}

fn real_main(args: Args, rt: &tokio::runtime::Runtime) -> Result<ExitCode> {
    let mut config = Config::load(args.config.clone())?;
    if args.list {
        list_connections(&config);
        return Ok(ExitCode::SUCCESS);
    }
    if args.setup_llm {
        return quarry::llm::setup::run(&mut config).map(|_| ExitCode::SUCCESS);
    }
    if let Some(p) = &args.prompt {
        config.main.prompt = p.clone();
    }
    if args.less_chatty {
        config.main.less_chatty = true;
    }
    if let Some(n) = args.row_limit {
        config.main.row_limit = n;
    }

    let batch = !args.execute.is_empty() || args.file.is_some() || !std::io::stdin().is_terminal();
    let depth = if args.no_color || (batch && !std::io::stdout().is_terminal()) {
        ColorDepth::None
    } else {
        ColorDepth::detect()
    };
    let theme_name = args.theme.clone().unwrap_or_else(|| config.main.theme.clone());
    let theme = theme::load(&theme_name, &config.themes_dir()).unwrap_or_else(|e| {
        eprintln!("quarry: {e}; using the default theme");
        Theme::default()
    });

    let Some(resolved) = cli::resolve(&args, &config)? else {
        if batch {
            bail!("no connection given (pass a URL, a saved connection name or a SQLite file)");
        }
        return quarry::tui::run(rt, config, None).map(|_| ExitCode::SUCCESS);
    };

    if let Some(name) = &args.save {
        let url = args.target.clone().filter(|t| t.contains(':') || t.contains('.')).unwrap_or_else(|| resolved.spec.display_url());
        config.connections.insert(
            name.clone(),
            SavedConnection { url, readonly: resolved.spec.readonly, ssh: args.ssh.clone(), ..Default::default() },
        );
        config.save().context("saving connection")?;
        eprintln!("Saved connection '{name}'.");
    }

    let opened = rt.block_on(cli::open(
        resolved.spec,
        resolved.password_command.as_deref(),
        args.force_password,
        !args.no_password,
    ))?;

    if args.tui && !batch {
        return quarry::tui::run(rt, config, Some(opened)).map(|_| ExitCode::SUCCESS);
    }

    let format = match &args.format {
        Some(f) => TableFormat::parse(f).with_context(|| format!("unknown format '{f}'"))?,
        None if batch && !std::io::stdout().is_terminal() => TableFormat::Tsv,
        None => TableFormat::parse(&config.main.table_format).unwrap_or(TableFormat::Rounded),
    };
    let mut session = make_session(rt, opened, config, theme, depth, format, !batch);
    session.continue_on_error = args.continue_on_error;

    if batch {
        return Ok(run_batch(&mut session, &args));
    }
    match repl::run(session)? {
        Exit::Quit => Ok(ExitCode::SUCCESS),
        Exit::Tui(s) => {
            let s = *s;
            let opened = Opened { conn: s.conn, spec: s.spec, tunnel: s.tunnel };
            quarry::tui::run(rt, s.config, Some(opened)).map(|_| ExitCode::SUCCESS)
        }
    }
}

fn make_session(
    rt: &tokio::runtime::Runtime,
    opened: Opened,
    config: Config,
    theme: Theme,
    depth: ColorDepth,
    format: TableFormat,
    interactive: bool,
) -> Session {
    let backend = opened.conn.backend();
    let palette = Palette::new(theme.clone(), depth);
    let expanded = match config.main.expanded.to_ascii_lowercase().as_str() {
        "on" | "true" => Expanded::On,
        "off" | "false" => Expanded::Off,
        _ => Expanded::Auto,
    };
    let opts = OutputOptions {
        format,
        expanded: if interactive { expanded } else { Expanded::Off },
        null_string: config.main.null_string.clone(),
        max_field_width: if interactive { config.main.max_field_width } else { None },
        terminal_width: repl::session::terminal_width(),
        color: depth,
        theme: Arc::new(theme),
        backend,
        table_name: None,
        align_numbers: true,
    };
    let favorites = Favorites::load(config.favorites_path()).unwrap_or_else(|e| {
        eprintln!("quarry: could not load favorites: {e:#}");
        Favorites::default()
    });
    let readonly = opened.spec.readonly;
    Session {
        rt: rt.handle().clone(),
        edit: repl::new_edit_state(backend, &config, palette.clone()),
        spec: opened.spec,
        conn: opened.conn,
        tunnel: opened.tunnel,
        palette,
        opts,
        timing: config.main.timing && interactive,
        pager_enabled: config.main.enable_pager,
        pager_cmd: config.main.pager.clone(),
        sinks: Sinks::default(),
        favorites,
        readonly,
        interactive,
        prompt_format: config.main.prompt.clone(),
        last: None,
        last_query: None,
        pending_buffer: None,
        continue_on_error: false,
        history_snapshot: Vec::new(),
        config,
    }
}

fn run_batch(session: &mut Session, args: &Args) -> ExitCode {
    let mut ok = true;
    for sql in &args.execute {
        let backend = session.conn.backend();
        if quarry::special::parse(sql.trim(), backend).is_some() {
            session.handle_input(sql);
            continue;
        }
        ok &= session.run_sql(sql, None);
        if !ok && !session.continue_on_error {
            return ExitCode::FAILURE;
        }
    }
    if let Some(path) = &args.file {
        match std::fs::read_to_string(path) {
            Ok(text) => ok &= run_script(session, &text),
            Err(e) => {
                eprintln!("quarry: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if args.execute.is_empty() && args.file.is_none() {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("quarry: reading stdin: {e}");
            return ExitCode::FAILURE;
        }
        ok &= run_script(session, &text);
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Scripts may mix SQL with special commands (one per line), like mysql/psql input files.
fn run_script(session: &mut Session, text: &str) -> bool {
    let backend = session.conn.backend();
    let mut sql = String::new();
    let mut ok = true;
    let flush = |session: &mut Session, sql: &mut String| -> bool {
        let r = sql.trim().is_empty() || session.run_sql(sql, None);
        sql.clear();
        r
    };
    for line in text.lines() {
        let t = line.trim();
        let at_boundary = sql.trim().is_empty() || quarry::sql::split::ends_with_terminator(&sql, backend, ";");
        let is_cmd = at_boundary
            && !t.is_empty()
            && (t.starts_with('\\') || t.starts_with('.') || t.to_ascii_lowercase().starts_with("delimiter "))
            && quarry::special::submits_immediately(t, backend);
        if is_cmd || t.to_ascii_lowercase().starts_with("delimiter ") {
            ok &= flush(session, &mut sql);
            session.handle_input(t);
            continue;
        }
        sql.push_str(line);
        sql.push('\n');
    }
    ok &= flush(session, &mut sql);
    ok
}

fn list_connections(config: &Config) {
    if config.connections.is_empty() {
        println!("No saved connections. Add one with: quarry <url> --save <name>");
        return;
    }
    let width = config.connections.keys().map(|k| k.len()).max().unwrap_or(0);
    for (name, c) in &config.connections {
        let url = quarry::conn::ConnSpec::parse(&c.url).map(|s| s.display_url()).unwrap_or_else(|_| c.url.clone());
        let ro = if c.readonly { " (read-only)" } else { "" };
        println!("{name:<width$}  {url}{ro}");
    }
}
