use std::io::{IsTerminal, Read};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser};

use quarry::cli::{self, Args, Opened};
use quarry::config::{Config, SavedConnection};
use quarry::output::{Expanded, OutputOptions, TableFormat};
use quarry::output::sink::Sinks;
use quarry::repl::{self, Exit, style::Palette};
use quarry::repl::session::{Flow, Session};
use quarry::special::favorites::Favorites;
use quarry::theme::{self, ColorDepth, Theme};

fn main() -> ExitCode {
    // When the shell asks for completions (`COMPLETE=<shell> quarry -- …`), answer and exit.
    clap_complete::CompleteEnv::with_factory(Args::command).complete();
    let args = Args::parse();
    if let Some(shell) = &args.completions {
        return print_completion_script(shell);
    }
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

/// The script to `source` in a shell's rc file; it calls back into `quarry` for every completion.
fn print_completion_script(shell: &str) -> ExitCode {
    // SAFETY: nothing else runs yet; the tokio runtime is started after this returns.
    unsafe { std::env::set_var("COMPLETE", shell) };
    match clap_complete::CompleteEnv::with_factory(Args::command).try_complete(["quarry"], None) {
        Ok(_) => {
            if shell == "zsh" {
                // Installed as an autoloaded `_quarry` (site-functions), the first Tab must complete too.
                println!("[[ $funcstack[1] == _quarry ]] && _clap_dynamic_completer_quarry \"$@\"");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("quarry: {e}");
            ExitCode::FAILURE
        }
    }
}

fn real_main(args: Args, rt: &tokio::runtime::Runtime) -> Result<ExitCode> {
    if args.default_config {
        print!("{}", quarry::config::DEFAULT_CONFIG);
        return Ok(ExitCode::SUCCESS);
    }
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
    quarry::icons::set(args.icons.unwrap_or(config.main.icons));

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
        return quarry::tui::run(rt, config, None, tui_overrides(&args, None, None)).map(|_| ExitCode::SUCCESS);
    };

    if let Some(name) = &args.save {
        let url = args.target.clone().filter(|t| t.contains(':') || t.contains('.')).unwrap_or_else(|| resolved.spec.display_url());
        let (url, had_password) = quarry::conn::url::strip_password(&url);
        if had_password {
            eprintln!(
                "quarry: the password was not saved; use password_command in the config, ~/.pgpass or ~/.my.cnf"
            );
        }
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
        let overrides = tui_overrides(&args, None, resolved.saved_name.clone());
        return quarry::tui::run(rt, config, Some(opened), overrides).map(|_| ExitCode::SUCCESS);
    }

    let format = match &args.format {
        Some(f) => TableFormat::parse(f).with_context(|| format!("unknown format '{f}'"))?,
        None if batch && !std::io::stdout().is_terminal() => TableFormat::Tsv,
        None => TableFormat::parse(&config.main.table_format).unwrap_or(TableFormat::Rounded),
    };
    for w in &config.warnings {
        eprintln!("quarry: warning: {w}");
    }
    let mut session = make_session(rt, opened, config, theme, depth, format, !batch);
    session.saved_name = resolved.saved_name.clone();
    session.continue_on_error = args.continue_on_error;

    if batch {
        return Ok(run_batch(&mut session, &args));
    }
    match repl::run(session)? {
        Exit::Quit => Ok(ExitCode::SUCCESS),
        Exit::Tui(s) => {
            let s = *s;
            let overrides = tui_overrides(&args, Some(s.opts.theme.name.clone()), s.saved_name.clone());
            let opened = Opened { conn: s.conn, spec: s.spec, tunnel: s.tunnel };
            quarry::tui::run(rt, s.config, Some(opened), overrides).map(|_| ExitCode::SUCCESS)
        }
    }
}

/// `theme` is the REPL's current theme when switching with `\tui`, so the TUI keeps it.
fn tui_overrides(args: &Args, theme: Option<String>, connection_name: Option<String>) -> quarry::tui::Overrides {
    quarry::tui::Overrides { theme: theme.or_else(|| args.theme.clone()), no_color: args.no_color, connection_name }
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
        row_lines: config.main.row_lines,
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
        saved_name: None,
        errors: Default::default(),
        config,
    }
}

fn run_batch(session: &mut Session, args: &Args) -> ExitCode {
    let status = |ok: bool| if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    let mut ok = true;
    for sql in &args.execute {
        let backend = session.conn.backend();
        if quarry::special::parse(sql.trim(), backend).is_some() {
            let (cmd_ok, flow) = session.handle_input_checked(sql);
            ok &= cmd_ok;
            if let Flow::Quit = flow {
                return status(ok);
            }
        } else {
            ok &= session.run_sql(sql, None);
        }
        if !ok && !session.continue_on_error {
            return ExitCode::FAILURE;
        }
    }
    let script = match (&args.file, args.execute.is_empty()) {
        (Some(path), _) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("quarry: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        },
        (None, true) => {
            let mut text = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut text) {
                eprintln!("quarry: reading stdin: {e}");
                return ExitCode::FAILURE;
            }
            text
        }
        (None, false) => return status(ok),
    };
    let (script_ok, _) = session.run_script(&script);
    status(ok && script_ok)
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
