use reedline::{Completer, Editor, IdeMenu, Menu, MenuEvent, MenuSettings, Painter, Suggestion};

/// The completion menu, which steps aside for Enter when its highlighted suggestion is already
/// typed out: taking it would change nothing and only cost a second Enter to run the line.
pub struct CompletionMenu {
    inner: IdeMenu,
    /// The first suggestion is what the line already says.
    typed: bool,
    /// The highlight was moved off the first suggestion.
    moved: bool,
}

impl CompletionMenu {
    pub fn new(inner: IdeMenu) -> Self {
        CompletionMenu { inner, typed: false, moved: false }
    }

    fn look(&mut self, editor: &Editor) {
        self.typed = already_typed(self.inner.get_values().first(), editor.get_buffer());
    }
}

fn already_typed(first: Option<&Suggestion>, line: &str) -> bool {
    first.is_some_and(|s| line.get(s.span.start..s.span.end) == Some(s.value.as_str()))
}

impl Menu for CompletionMenu {
    fn settings(&self) -> &MenuSettings {
        self.inner.settings()
    }

    fn is_active(&self) -> bool {
        self.inner.is_active()
    }

    fn set_active(&mut self, active: bool) {
        self.inner.set_active(active);
    }

    fn clear_input(&mut self) {
        self.inner.clear_input();
    }

    fn on_activate(&mut self) {
        self.inner.on_activate();
    }

    fn on_deactivate(&mut self) {
        self.inner.on_deactivate();
    }

    fn handle_menu_event(&mut self, event: &MenuEvent) {
        self.inner.handle_menu_event(event);
    }

    fn menu_event(&mut self, event: MenuEvent) {
        self.moved = !matches!(event, MenuEvent::Activate(_) | MenuEvent::Deactivate | MenuEvent::Edit(_));
        self.inner.menu_event(event);
    }

    fn can_quick_complete(&self) -> bool {
        self.inner.can_quick_complete()
    }

    fn can_partially_complete(&mut self, values_updated: bool, editor: &mut Editor, completer: &mut dyn Completer) -> bool {
        let done = self.inner.can_partially_complete(values_updated, editor, completer);
        self.look(editor);
        done
    }

    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        self.inner.update_values(editor, completer);
        self.look(editor);
    }

    fn reset_position(&mut self) {
        self.moved = false;
        self.inner.reset_position();
    }

    fn reload(&mut self, updated: bool, editor: &mut Editor, completer: &mut dyn Completer) {
        self.moved = false;
        self.inner.reload(updated, editor, completer);
        self.look(editor);
    }

    fn update_working_details(&mut self, editor: &mut Editor, completer: &mut dyn Completer, painter: &Painter) {
        self.inner.update_working_details(editor, completer, painter);
        self.look(editor);
    }

    fn replace_in_buffer(&self, editor: &mut Editor) {
        self.inner.replace_in_buffer(editor);
    }

    fn menu_required_lines(&self, terminal_columns: u16) -> u16 {
        self.inner.menu_required_lines(terminal_columns)
    }

    fn menu_string(&self, available_lines: u16, use_ansi_coloring: bool) -> String {
        self.inner.menu_string(available_lines, use_ansi_coloring)
    }

    fn min_rows(&self) -> u16 {
        self.inner.min_rows()
    }

    /// Empty while taking the highlighted suggestion would change nothing, which is how a menu
    /// tells the line editor that it has no claim on Enter.
    fn get_values(&self) -> &[Suggestion] {
        if self.typed && !self.moved { &[] } else { self.inner.get_values() }
    }

    fn results_are_provisional(&self) -> bool {
        self.inner.results_are_provisional()
    }

    fn is_awaiting_first_answer(&self) -> bool {
        self.inner.is_awaiting_first_answer()
    }

    fn is_visible(&self) -> bool {
        self.inner.is_visible()
    }

    fn set_cursor_pos(&mut self, pos: (u16, u16)) {
        self.inner.set_cursor_pos(pos);
    }
}

#[cfg(test)]
mod tests {
    use reedline::Span;

    use super::*;

    fn suggestion(value: &str, span: (usize, usize)) -> Suggestion {
        Suggestion { value: value.into(), span: Span::new(span.0, span.1), ..Default::default() }
    }

    #[test]
    fn a_suggestion_the_line_already_spells_out_is_not_worth_an_enter() {
        assert!(already_typed(Some(&suggestion("\\d", (0, 2))), "\\d"), "\\d is typed, though \\dt and \\df are offered too");
        assert!(already_typed(Some(&suggestion("users", (14, 19))), "select * from users"));
        assert!(!already_typed(Some(&suggestion("users", (14, 16))), "select * from us"), "there is something to complete");
        assert!(!already_typed(Some(&suggestion("SELECT", (0, 6))), "select"), "accepting would change the case");
        assert!(!already_typed(None, "select"));
    }
}
