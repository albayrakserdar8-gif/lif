//! Copy-mode commands, named as tmux names them for `send-keys -X`.

macro_rules! commands {
    ($($variant:ident = $name:literal),* $(,)?) => {
        /// A copy-mode command.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Command {
            $($variant,)*
        }

        impl Command {
            /// Every command, in tmux's manual order.
            pub const ALL: &'static [Command] = &[$(Command::$variant,)*];

            /// The tmux name (`send-keys -X <name>`).
            pub const fn name(self) -> &'static str {
                match self {
                    $(Command::$variant => $name,)*
                }
            }
        }
    };
}

commands! {
    AppendSelection = "append-selection",
    AppendSelectionAndCancel = "append-selection-and-cancel",
    BackToIndentation = "back-to-indentation",
    BeginSelection = "begin-selection",
    BottomLine = "bottom-line",
    Cancel = "cancel",
    ClearSelection = "clear-selection",
    CopyEndOfLine = "copy-end-of-line",
    CopyEndOfLineAndCancel = "copy-end-of-line-and-cancel",
    CopyLine = "copy-line",
    CopyLineAndCancel = "copy-line-and-cancel",
    CopySelection = "copy-selection",
    CopySelectionNoClear = "copy-selection-no-clear",
    CopySelectionAndCancel = "copy-selection-and-cancel",
    CursorDown = "cursor-down",
    CursorDownAndCancel = "cursor-down-and-cancel",
    CursorLeft = "cursor-left",
    CursorRight = "cursor-right",
    CursorUp = "cursor-up",
    CursorCentreVertical = "cursor-centre-vertical",
    CursorCentreHorizontal = "cursor-centre-horizontal",
    EndOfLine = "end-of-line",
    GotoLine = "goto-line",
    HalfpageDown = "halfpage-down",
    HalfpageDownAndCancel = "halfpage-down-and-cancel",
    HalfpageUp = "halfpage-up",
    HistoryBottom = "history-bottom",
    HistoryTop = "history-top",
    JumpAgain = "jump-again",
    JumpBackward = "jump-backward",
    JumpForward = "jump-forward",
    JumpReverse = "jump-reverse",
    JumpToBackward = "jump-to-backward",
    JumpToForward = "jump-to-forward",
    JumpToMark = "jump-to-mark",
    MiddleLine = "middle-line",
    NextMatchingBracket = "next-matching-bracket",
    NextParagraph = "next-paragraph",
    NextPrompt = "next-prompt",
    NextWord = "next-word",
    NextWordEnd = "next-word-end",
    NextSpace = "next-space",
    NextSpaceEnd = "next-space-end",
    OtherEnd = "other-end",
    PageDown = "page-down",
    PageDownAndCancel = "page-down-and-cancel",
    PageUp = "page-up",
    PreviousMatchingBracket = "previous-matching-bracket",
    PreviousParagraph = "previous-paragraph",
    PreviousPrompt = "previous-prompt",
    PreviousWord = "previous-word",
    PreviousSpace = "previous-space",
    RectangleOn = "rectangle-on",
    RectangleOff = "rectangle-off",
    RectangleToggle = "rectangle-toggle",
    RecentreTopBottom = "recentre-top-bottom",
    RefreshNow = "refresh-now",
    ScrollBottom = "scroll-bottom",
    ScrollDown = "scroll-down",
    ScrollDownAndCancel = "scroll-down-and-cancel",
    ScrollExitOn = "scroll-exit-on",
    ScrollExitOff = "scroll-exit-off",
    ScrollExitToggle = "scroll-exit-toggle",
    ScrollMiddle = "scroll-middle",
    ScrollTop = "scroll-top",
    ScrollUp = "scroll-up",
    SearchAgain = "search-again",
    SearchBackward = "search-backward",
    SearchBackwardIncremental = "search-backward-incremental",
    SearchBackwardText = "search-backward-text",
    SearchForward = "search-forward",
    SearchForwardIncremental = "search-forward-incremental",
    SearchForwardText = "search-forward-text",
    SearchReverse = "search-reverse",
    SelectLine = "select-line",
    SelectWord = "select-word",
    SetMark = "set-mark",
    StartOfLine = "start-of-line",
    StopSelection = "stop-selection",
    TogglePosition = "toggle-position",
    TopLine = "top-line",
}

impl Command {
    /// Look a command up by its tmux name. The `copy-pipe*` family maps to
    /// the matching copy command: running the pipe command is the server's
    /// business (it gets the copied text either way).
    pub fn from_name(name: &str) -> Option<Command> {
        let alias = match name {
            "copy-pipe" => Some(Command::CopySelection),
            "copy-pipe-no-clear" => Some(Command::CopySelectionNoClear),
            "copy-pipe-and-cancel" => Some(Command::CopySelectionAndCancel),
            "copy-pipe-line" => Some(Command::CopyLine),
            "copy-pipe-line-and-cancel" => Some(Command::CopyLineAndCancel),
            "copy-pipe-end-of-line" => Some(Command::CopyEndOfLine),
            "copy-pipe-end-of-line-and-cancel" => Some(Command::CopyEndOfLineAndCancel),
            _ => None,
        };
        alias.or_else(|| Command::ALL.iter().copied().find(|c| c.name() == name))
    }

    /// Commands that need an argument (a character, a pattern or a line).
    pub const fn takes_argument(self) -> bool {
        matches!(
            self,
            Command::GotoLine
                | Command::JumpBackward
                | Command::JumpForward
                | Command::JumpToBackward
                | Command::JumpToForward
                | Command::SearchBackward
                | Command::SearchBackwardIncremental
                | Command::SearchBackwardText
                | Command::SearchForward
                | Command::SearchForwardIncremental
                | Command::SearchForwardText
        )
    }

    /// Jumps take a single character, read from the next key press.
    pub const fn is_jump(self) -> bool {
        matches!(
            self,
            Command::JumpBackward | Command::JumpForward | Command::JumpToBackward | Command::JumpToForward
        )
    }

    /// Commands that only scroll, which keep `copy-mode -e` (exit at the
    /// bottom) armed; any other command disarms it, as in tmux.
    pub const fn is_scroll(self) -> bool {
        matches!(
            self,
            Command::PageUp
                | Command::PageDown
                | Command::PageDownAndCancel
                | Command::HalfpageUp
                | Command::HalfpageDown
                | Command::HalfpageDownAndCancel
                | Command::ScrollUp
                | Command::ScrollDown
                | Command::ScrollDownAndCancel
                | Command::CursorUp
                | Command::CursorDown
                | Command::CursorDownAndCancel
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for c in Command::ALL {
            assert_eq!(Command::from_name(c.name()), Some(*c));
        }
        assert_eq!(Command::from_name("copy-pipe-and-cancel"), Some(Command::CopySelectionAndCancel));
        assert_eq!(Command::from_name("nope"), None);
    }
}
