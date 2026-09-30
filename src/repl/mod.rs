pub mod editing;
pub mod highlight;
pub mod prompt;
pub mod session;
pub mod style;

use std::sync::{Arc, RwLock};

use anyhow::Result;
use reedline::{
    DefaultHinter, DescriptionMode, EditCommand, EditMode, Emacs, FileBackedHistory, IdeMenu, KeyCode,
    KeyModifiers, Keybindings, MenuBuilder, Reedline, ReedlineEvent, ReedlineMenu, Signal, Vi,
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    default_vi_visual_keybindings,
};

use editing::{ReplCompleter, ReplValidator, SafeHistory, SharedEdit};
use prompt::QPrompt;
use session::{Flow, Session};

const MENU: &str = "completion_menu";
const HOST: &str = "\u{1}quarry:";

pub enum Exit {
    Quit,
    Tui(Box<Session>),
}

struct Toggles {
    vi: bool,
    complete_while_typing: bool,
}

pub fn run(mut session: Session) -> Result<Exit> {
    let mut toggles = Toggles { vi: session.config.main.vi, complete_while_typing: session.config.main.complete_while_typing };
    let mut editor = build_editor(&session, &toggles)?;
    if !session.config.main.less_chatty {
        print_banner(&session);
    }
    session.refresh_catalog();

    loop {
        let prompt = QPrompt::new(&session.prompt_format, &session.config.main.prompt_continuation, &session.prompt_info(), &session.palette);
        if let Some(buf) = session.pending_buffer.take() {
            editor.run_edit_commands(&[EditCommand::InsertString(buf)]);
        }
        let signal = match editor.read_line(&prompt) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", session.palette.error(&format!("terminal error: {e}")));
                return Ok(Exit::Quit);
            }
        };
        match signal {
            Signal::Success(line) => {
                session.history_snapshot = history_lines(&editor);
                match session.handle_input(&line) {
                    Flow::Continue => {}
                    Flow::Quit => break,
                    Flow::Tui => return Ok(Exit::Tui(Box::new(session))),
                }
            }
            Signal::HostCommand(cmd) => {
                let rest = editor.current_buffer_contents().to_string();
                println!();
                match cmd.strip_prefix(HOST) {
                    Some("smart") => {
                        let mut st = session.edit.write().unwrap();
                        st.smart_completion = !st.smart_completion;
                        let on = st.smart_completion;
                        drop(st);
                        let cat = session.current_catalog();
                        if let Some(cat) = cat {
                            session::install_catalog(&session.edit, &session.config, &session.favorites, (*cat).clone());
                        }
                        println!("{}", session.palette.muted(&format!("Smart completion {}", on_off(on))));
                    }
                    Some("multiline") => {
                        let mut st = session.edit.write().unwrap();
                        st.multi_line = !st.multi_line;
                        println!("{}", session.palette.muted(&format!("Multi-line mode {}", on_off(st.multi_line))));
                    }
                    Some("vi") => {
                        toggles.vi = !toggles.vi;
                        editor = build_editor(&session, &toggles)?;
                        println!("{}", session.palette.muted(if toggles.vi { "Vi mode" } else { "Emacs mode" }));
                    }
                    _ => {}
                }
                session.pending_buffer = Some(rest);
            }
            Signal::CtrlC => continue,
            Signal::CtrlD => break,
            _ => continue,
        }
    }
    if !session.config.main.less_chatty {
        println!("{}", session.palette.muted("Goodbye!"));
    }
    Ok(Exit::Quit)
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn history_lines(editor: &Reedline) -> Vec<String> {
    use reedline::{SearchDirection, SearchQuery};
    let mut q = SearchQuery::everything(SearchDirection::Backward, None);
    q.limit = Some(1000);
    let mut items: Vec<String> =
        editor.history().search(q).map(|v| v.into_iter().map(|h| h.command_line).collect()).unwrap_or_default();
    items.reverse();
    items
}

fn print_banner(s: &Session) {
    let p = &s.palette;
    let info = s.conn.info();
    println!(
        "{} {} {} {}",
        p.accent("quarry"),
        p.muted(env!("CARGO_PKG_VERSION")),
        p.muted("·"),
        p.paint(p.fg(p.theme.accent2), &info.version),
    );
    println!(
        "{}",
        p.muted("Type \\? for help · \\tui for the full-screen interface · Tab to complete · Ctrl-D to quit")
    );
}

pub fn new_edit_state(session_backend: crate::db::Backend, config: &crate::config::Config, palette: style::Palette) -> SharedEdit {
    Arc::new(RwLock::new(editing::EditState {
        backend: session_backend,
        delimiter: ";".into(),
        multi_line: config.main.multi_line,
        palette,
        completer: None,
        smart_completion: config.main.smart_completion,
    }))
}

fn build_editor(session: &Session, toggles: &Toggles) -> Result<Reedline> {
    let p = session.palette.clone();
    let th = &p.theme;
    let hl = HighlighterBridge { edit: session.edit.clone() };

    let menu = IdeMenu::default()
        .with_name(MENU)
        .with_marker(&p.paint(p.fg(th.accent).bold(), "❯ "))
        .with_word_chars("_$")
        .with_default_border()
        .with_description_mode(DescriptionMode::PreferRight)
        .with_min_completion_width(18)
        .with_max_completion_height(12)
        .with_padding(1)
        .with_text_style(p.fg(th.fg))
        .with_selected_text_style(p.on(th.fg, th.selection).bold())
        .with_description_text_style(p.fg(th.muted))
        .with_match_text_style(p.fg(th.accent).underline())
        .with_selected_match_text_style(p.on(th.accent, th.selection).bold().underline());

    let edit_mode: Box<dyn EditMode> = if toggles.vi {
        let mut insert = default_vi_insert_keybindings();
        let mut normal = default_vi_normal_keybindings();
        add_bindings(&mut insert, toggles.complete_while_typing);
        add_bindings(&mut normal, false);
        Box::new(Vi::new(insert, normal, default_vi_visual_keybindings()))
    } else {
        let mut kb = default_emacs_keybindings();
        add_bindings(&mut kb, toggles.complete_while_typing);
        Box::new(Emacs::new(kb))
    };

    let history_path = session::history_path(&session.config);
    session::ensure_dir(&history_path)?;
    let history = FileBackedHistory::with_file(session.config.main.history_size.max(100), history_path.clone())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&history_path, std::fs::Permissions::from_mode(0o600));
    }

    let mut editor = Reedline::create()
        .with_history(Box::new(SafeHistory { inner: history }))
        .with_highlighter(Box::new(hl))
        .with_completer(Box::new(ReplCompleter { state: session.edit.clone() }))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_validator(Box::new(ReplValidator { state: session.edit.clone() }))
        .with_edit_mode(edit_mode)
        .with_quick_completions(false)
        .with_partial_completions(false)
        .with_ansi_colors(p.depth != crate::theme::ColorDepth::None);
    if session.config.main.auto_suggest {
        editor = editor.with_hinter(Box::new(DefaultHinter::default().with_style(p.fg(th.muted).italic())));
    }
    Ok(editor)
}

fn add_bindings(kb: &mut Keybindings, complete_while_typing: bool) {
    // Tab takes the highlighted (best) match or opens the menu; Enter always runs, so a menu
    // that popped up while typing never swallows the submit.
    kb.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuAccept, ReedlineEvent::Menu(MENU.into())]),
    );
    kb.add_binding(KeyModifiers::NONE, KeyCode::Enter, ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Enter]));
    kb.add_binding(KeyModifiers::SHIFT, KeyCode::BackTab, ReedlineEvent::MenuPrevious);
    kb.add_binding(KeyModifiers::NONE, KeyCode::Down, ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuDown, ReedlineEvent::Down]));
    kb.add_binding(KeyModifiers::NONE, KeyCode::Up, ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuUp, ReedlineEvent::Up]));
    kb.add_binding(KeyModifiers::CONTROL, KeyCode::Char(' '), ReedlineEvent::Menu(MENU.into()));
    kb.add_binding(KeyModifiers::ALT, KeyCode::Enter, ReedlineEvent::Submit);
    kb.add_binding(KeyModifiers::NONE, KeyCode::F(2), ReedlineEvent::ExecuteHostCommand(format!("{HOST}smart")));
    kb.add_binding(KeyModifiers::NONE, KeyCode::F(3), ReedlineEvent::ExecuteHostCommand(format!("{HOST}multiline")));
    kb.add_binding(KeyModifiers::NONE, KeyCode::F(4), ReedlineEvent::ExecuteHostCommand(format!("{HOST}vi")));
    if complete_while_typing {
        let chars = ('a'..='z').chain('A'..='Z').chain(['_', '.']);
        for c in chars {
            let modifier = if c.is_ascii_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
            kb.add_binding(
                modifier,
                KeyCode::Char(c),
                ReedlineEvent::Multiple(vec![
                    ReedlineEvent::Edit(vec![EditCommand::InsertChar(c)]),
                    ReedlineEvent::Menu(MENU.into()),
                ]),
            );
        }
    }
}

/// Reads backend + palette from the shared edit state so `\theme` / `\c` take effect immediately.
struct HighlighterBridge {
    edit: SharedEdit,
}

impl reedline::Highlighter for HighlighterBridge {
    fn highlight(&self, line: &str, cursor: usize) -> reedline::StyledText {
        let st = self.edit.read().unwrap();
        highlight::highlight(line, cursor, st.backend, &st.palette)
    }
}
