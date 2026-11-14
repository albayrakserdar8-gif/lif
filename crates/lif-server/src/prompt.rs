//! The status-line prompt: `command-prompt` (a small line editor with
//! history) and `confirm-before`.

use lif_core::{KeyCode, PaneId, Prompt};
use lif_render::char_width;
use lif_types::{Color, Key, KeyAction, KeyEvent, Mods, Span, Style, StyledLine};

/// Prompt history kept by the server, oldest first.
pub(crate) const HISTORY_MAX: usize = 100;

pub(crate) enum PromptAction {
    Continue,
    Cancel,
    /// Parse and run this command string.
    Run(String),
    /// A copy-mode prompt's text changed (incremental search).
    CopyChanged(PaneId, String),
    /// C-s (true) or C-r in an incremental copy-mode prompt.
    CopyAgain(PaneId, bool),
    CopyAccept(PaneId, String),
    CopyCancel(PaneId),
}

/// A prompt copy mode asked for (search, goto line).
#[derive(Clone, Copy)]
struct CopyPrompt {
    pane: PaneId,
    incremental: bool,
}

pub(crate) struct PromptState {
    prompt: Prompt,
    /// Index of the prompt being answered (command-prompt with several -p).
    step: usize,
    answers: Vec<String>,
    line: Vec<char>,
    cursor: usize,
    /// Position in the history while browsing it; `None` when editing.
    hist: Option<usize>,
    saved: Vec<char>,
    copy: Option<CopyPrompt>,
}

pub(crate) fn message_style() -> Style {
    Style { fg: Color::Indexed(0), bg: Color::Indexed(3), ..Style::default() }
}

impl PromptState {
    pub(crate) fn new(prompt: Prompt) -> Self {
        let mut s =
            PromptState { prompt, step: 0, answers: Vec::new(), line: Vec::new(), cursor: 0, hist: None, saved: Vec::new(), copy: None };
        s.load_input();
        s
    }

    /// A prompt for copy mode in `pane` (`(search up)` ...); its edits and
    /// answer go back to the mode instead of running a command.
    pub(crate) fn copy(pane: PaneId, label: &str, initial: String, incremental: bool) -> Self {
        let mut s = PromptState::new(Prompt::Command {
            prompts: vec![format!("{label} ")],
            inputs: vec![initial],
            template: String::new(),
            single_key: false,
            key_name: false,
            numeric: false,
        });
        s.copy = Some(CopyPrompt { pane, incremental });
        s
    }

    /// The pane of a copy-mode prompt.
    pub(crate) fn copy_pane(&self) -> Option<PaneId> {
        self.copy.map(|c| c.pane)
    }

    fn load_input(&mut self) {
        self.line.clear();
        if let Prompt::Command { inputs, .. } = &self.prompt
            && let Some(i) = inputs.get(self.step)
        {
            self.line.extend(i.chars());
        }
        self.cursor = self.line.len();
        self.hist = None;
    }

    fn label(&self) -> &str {
        match &self.prompt {
            Prompt::Command { prompts, .. } => prompts.get(self.step).map(|s| s.as_str()).unwrap_or(":"),
            Prompt::Confirm { prompt, .. } => prompt,
        }
    }

    /// The line to show (in `style`) and the column of the cursor in it.
    pub(crate) fn render(&self, out: &mut StyledLine, style: Style) -> u16 {
        out.clear();
        let label = self.label().to_string();
        let mut col: usize = label.chars().map(char_width).sum();
        out.push(Span::new(label, style));
        if matches!(self.prompt, Prompt::Command { .. }) {
            let text: String = self.line.iter().collect();
            col += self.line[..self.cursor].iter().map(|c| char_width(*c)).sum::<usize>();
            out.push(Span::new(text, style));
        }
        col.min(u16::MAX as usize) as u16
    }

    pub(crate) fn key(&mut self, key: &KeyEvent, history: &mut Vec<String>) -> PromptAction {
        if key.action == KeyAction::Release || matches!(key.key, Key::Modifier(_)) {
            return PromptAction::Continue;
        }
        if let Some(c) = self.copy {
            return self.copy_key(c, key, history);
        }
        let (single_key, key_name, numeric) = match &self.prompt {
            Prompt::Confirm { command, default_yes, .. } => {
                let yes = matches!(key.key, Key::Char('y') | Key::Char('Y')) && !key.mods.contains(Mods::CTRL)
                    || (*default_yes && key.key == Key::Enter);
                return if yes { PromptAction::Run(command.clone()) } else { PromptAction::Cancel };
            }
            Prompt::Command { single_key, key_name, numeric, .. } => (*single_key, *key_name, *numeric),
        };
        let ctrl = key.mods.contains(Mods::CTRL);
        if key_name {
            let name = KeyCode::from_event(key).map(|k| k.to_string()).unwrap_or_default();
            return self.answer(name, history);
        }
        if single_key {
            if key.key == Key::Escape || (ctrl && key.key == Key::Char('c')) {
                return PromptAction::Cancel;
            }
            let text = key_text(key);
            if text.is_empty() {
                return PromptAction::Continue;
            }
            return self.answer(text, history);
        }
        match key.key {
            Key::Escape => return PromptAction::Cancel,
            Key::Char('c') | Key::Char('g') if ctrl => return PromptAction::Cancel,
            Key::Enter => {
                let line: String = self.line.iter().collect();
                return self.answer(line, history);
            }
            Key::Char('m') | Key::Char('j') if ctrl => {
                let line: String = self.line.iter().collect();
                return self.answer(line, history);
            }
            Key::Backspace => self.backspace(),
            Key::Char('h') if ctrl => self.backspace(),
            Key::Delete => {
                if self.cursor < self.line.len() {
                    self.line.remove(self.cursor);
                }
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Char('b') if ctrl => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.line.len()),
            Key::Char('f') if ctrl => self.cursor = (self.cursor + 1).min(self.line.len()),
            Key::Home => self.cursor = 0,
            Key::Char('a') if ctrl => self.cursor = 0,
            Key::End => self.cursor = self.line.len(),
            Key::Char('e') if ctrl => self.cursor = self.line.len(),
            Key::Char('u') if ctrl => {
                self.line.drain(..self.cursor);
                self.cursor = 0;
            }
            Key::Char('k') if ctrl => self.line.truncate(self.cursor),
            Key::Char('w') if ctrl => {
                let mut i = self.cursor;
                while i > 0 && self.line[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && self.line[i - 1] != ' ' {
                    i -= 1;
                }
                self.line.drain(i..self.cursor);
                self.cursor = i;
            }
            Key::Up => self.history_move(history, true),
            Key::Char('p') if ctrl => self.history_move(history, true),
            Key::Down => self.history_move(history, false),
            Key::Char('n') if ctrl => self.history_move(history, false),
            _ => {
                let text = key_text(key);
                if numeric && !text.chars().all(|c| c.is_ascii_digit()) {
                    return PromptAction::Continue;
                }
                self.insert(&text);
            }
        }
        PromptAction::Continue
    }

    fn copy_key(&mut self, c: CopyPrompt, key: &KeyEvent, history: &mut Vec<String>) -> PromptAction {
        let ctrl = key.mods.contains(Mods::CTRL);
        let line = |s: &Self| s.line.iter().collect::<String>();
        match key.key {
            Key::Escape => return PromptAction::CopyCancel(c.pane),
            Key::Char('c') | Key::Char('g') if ctrl => return PromptAction::CopyCancel(c.pane),
            Key::Enter => return PromptAction::CopyAccept(c.pane, line(self)),
            Key::Char('m') | Key::Char('j') if ctrl => return PromptAction::CopyAccept(c.pane, line(self)),
            Key::Char('s') if ctrl && c.incremental => return PromptAction::CopyAgain(c.pane, true),
            Key::Char('r') if ctrl && c.incremental => return PromptAction::CopyAgain(c.pane, false),
            _ => {}
        }
        let before = self.line.clone();
        // The usual line editing (search prompts keep no history).
        let mut none = Vec::new();
        let _ = history;
        let copy = self.copy.take();
        let _ = self.key(key, &mut none);
        self.copy = copy;
        if c.incremental && self.line != before {
            return PromptAction::CopyChanged(c.pane, line(self));
        }
        PromptAction::Continue
    }

    /// Pasted text goes into the line (control characters dropped).
    pub(crate) fn insert(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            self.line.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.line.remove(self.cursor);
        }
    }

    fn history_move(&mut self, history: &[String], up: bool) {
        if history.is_empty() {
            return;
        }
        let next = match (self.hist, up) {
            (None, true) => {
                self.saved = self.line.clone();
                Some(history.len() - 1)
            }
            (None, false) => return,
            (Some(0), true) => Some(0),
            (Some(i), true) => Some(i - 1),
            (Some(i), false) if i + 1 < history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.hist = next;
        self.line = match next {
            Some(i) => history[i].chars().collect(),
            None => std::mem::take(&mut self.saved),
        };
        self.cursor = self.line.len();
    }

    fn answer(&mut self, text: String, history: &mut Vec<String>) -> PromptAction {
        let Prompt::Command { prompts, template, single_key, key_name, .. } = &self.prompt else {
            return PromptAction::Cancel;
        };
        if !text.is_empty() && !*single_key && !*key_name && history.last() != Some(&text) {
            history.push(text.clone());
            if history.len() > HISTORY_MAX {
                history.remove(0);
            }
        }
        self.answers.push(text);
        if self.step + 1 < prompts.len() {
            self.step += 1;
            self.load_input();
            return PromptAction::Continue;
        }
        if self.answers.len() == 1 && self.answers[0].is_empty() && prompts.len() <= 1 && !*single_key {
            // tmux runs nothing for an empty answer.
            return PromptAction::Cancel;
        }
        let refs: Vec<&str> = self.answers.iter().map(|s| s.as_str()).collect();
        PromptAction::Run(lif_core::expand_prompt_template(template, &refs))
    }
}

fn key_text(key: &KeyEvent) -> String {
    if key.mods.contains(Mods::CTRL) || key.mods.contains(Mods::ALT) || key.mods.contains(Mods::SUPER) {
        return String::new();
    }
    if !key.text.is_empty() {
        return key.text.clone();
    }
    match key.key {
        Key::Char(c) if !c.is_control() => {
            if key.mods.contains(Mods::SHIFT) {
                key.shifted.unwrap_or_else(|| c.to_uppercase().next().unwrap_or(c)).to_string()
            } else {
                c.to_string()
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_prompt() -> PromptState {
        PromptState::new(Prompt::Command {
            prompts: vec![":".into()],
            inputs: vec![String::new()],
            template: "%1".into(),
            single_key: false,
            key_name: false,
            numeric: false,
        })
    }

    fn k(key: Key) -> KeyEvent {
        KeyEvent::new(key, Mods::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(Key::Char(c), Mods::CTRL)
    }

    fn typed(p: &mut PromptState, s: &str, h: &mut Vec<String>) {
        for c in s.chars() {
            assert!(matches!(p.key(&k(Key::Char(c)), h), PromptAction::Continue));
        }
    }

    #[test]
    fn edits_and_runs() {
        let mut h = Vec::new();
        let mut p = cmd_prompt();
        typed(&mut p, "new-windw", &mut h);
        p.key(&k(Key::Left), &mut h);
        typed(&mut p, "o", &mut h);
        p.key(&k(Key::End), &mut h);
        typed(&mut p, " -d", &mut h);
        match p.key(&k(Key::Enter), &mut h) {
            PromptAction::Run(s) => assert_eq!(s, "new-window -d"),
            _ => panic!("expected run"),
        }
        assert_eq!(h, vec!["new-window -d".to_string()]);
    }

    #[test]
    fn kill_word_line_and_history() {
        let mut h = vec!["first".to_string(), "second".to_string()];
        let mut p = cmd_prompt();
        typed(&mut p, "abc def", &mut h);
        p.key(&ctrl('w'), &mut h);
        let mut line = StyledLine::new();
        p.render(&mut line, message_style());
        assert_eq!(line[1].text, "abc ");
        p.key(&ctrl('u'), &mut h);
        p.render(&mut line, message_style());
        assert_eq!(line[1].text, "");
        p.key(&k(Key::Up), &mut h);
        p.render(&mut line, message_style());
        assert_eq!(line[1].text, "second");
        p.key(&k(Key::Up), &mut h);
        p.render(&mut line, message_style());
        assert_eq!(line[1].text, "first");
        p.key(&k(Key::Down), &mut h);
        p.key(&k(Key::Down), &mut h);
        p.render(&mut line, message_style());
        assert_eq!(line[1].text, "");
        typed(&mut p, "xyz", &mut h);
        p.key(&k(Key::Home), &mut h);
        p.key(&ctrl('k'), &mut h);
        p.render(&mut line, message_style());
        assert_eq!(line[1].text, "");
        assert!(matches!(p.key(&k(Key::Escape), &mut h), PromptAction::Cancel));
    }

    #[test]
    fn confirm() {
        let mut h = Vec::new();
        let mut p = PromptState::new(Prompt::Confirm { prompt: "kill? (y/n)".into(), command: "kill-pane".into(), default_yes: false });
        assert!(matches!(p.key(&k(Key::Char('y')), &mut h), PromptAction::Run(s) if s == "kill-pane"));
        let mut p = PromptState::new(Prompt::Confirm { prompt: "x".into(), command: "c".into(), default_yes: false });
        assert!(matches!(p.key(&k(Key::Enter), &mut h), PromptAction::Cancel));
    }
}
