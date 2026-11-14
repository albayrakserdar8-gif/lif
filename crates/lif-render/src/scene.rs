//! What to draw: borrowed from the server for one frame.

use lif_types::{Color, Cursor, PaneFrame, Pos, Rect, Size, Style, StyledLine};

/// Styles of lif's own decorations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub border: Style,
    /// Border cells next to the active pane.
    pub active_border: Style,
    /// Selected cells in copy mode replace their style with this one.
    pub selection: Style,
    /// The position indicator (`[12/340]`) on a pane's first row.
    pub indicator: Style,
    /// Search matches shown in copy mode (tmux's `copy-mode-match-style`),
    /// drawn under the selection.
    pub search_match: Style,
    /// The match the cursor is on (`copy-mode-current-match-style`).
    pub current_match: Style,
}

impl Default for Theme {
    /// tmux's defaults: green active border, yellow mode style, cyan
    /// matches and a magenta current match.
    fn default() -> Self {
        let on = |bg: u8| Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(bg),
            ..Style::default()
        };
        let mode = on(3);
        Theme {
            border: Style::default(),
            active_border: Style {
                fg: Color::Indexed(2),
                ..Style::default()
            },
            selection: mode,
            indicator: mode,
            search_match: on(6),
            current_match: on(5),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusPosition {
    Top,
    Bottom,
}

/// The status line. Text is laid out left to right and cut at the screen
/// width; the rest of the row is filled with blanks in `fill`.
#[derive(Clone, Copy, Debug)]
pub struct StatusBar<'a> {
    pub line: &'a StyledLine,
    pub fill: Style,
    pub position: StatusPosition,
}

/// A one-line message or prompt. It replaces the status line, or covers the
/// bottom row when there is no status line. Where the prompt's cursor goes
/// is up to the server, through [`Scene::cursor`].
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub line: &'a StyledLine,
    pub fill: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Character-wise, wrapping across lines (the usual copy-mode selection).
    Stream,
    /// Whole lines.
    Lines,
    /// A rectangle.
    Rect,
}

/// A copy-mode selection in pane coordinates; `start` and `end` are both
/// inclusive and may be in either order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: Pos,
    pub end: Pos,
    pub kind: SelectionKind,
}

impl Selection {
    /// Rows the selection touches, inclusive.
    pub fn rows(&self) -> (u16, u16) {
        (self.start.y.min(self.end.y), self.start.y.max(self.end.y))
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        let (s, e) = if (self.start.y, self.start.x) <= (self.end.y, self.end.x) {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        };
        match self.kind {
            SelectionKind::Lines => y >= s.y && y <= e.y,
            SelectionKind::Rect => {
                let (x0, x1) = (self.start.x.min(self.end.x), self.start.x.max(self.end.x));
                y >= s.y && y <= e.y && x >= x0 && x <= x1
            }
            SelectionKind::Stream => {
                if y < s.y || y > e.y {
                    false
                } else if s.y == e.y {
                    x >= s.x && x <= e.x
                } else if y == s.y {
                    x >= s.x
                } else if y == e.y {
                    x <= e.x
                } else {
                    true
                }
            }
        }
    }
}

/// One highlighted run of a search match on one row of a pane, in pane
/// coordinates, columns `x0..=x1`. A wide character is highlighted when the
/// column of its first cell is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    pub y: u16,
    pub x0: u16,
    pub x1: u16,
    /// Drawn in [`Theme::current_match`] instead of [`Theme::search_match`].
    pub current: bool,
}

impl MatchSpan {
    /// The style `spans` give the cell at `(x, y)`: the current match's if
    /// any current span covers it, else the match style if any span does.
    pub fn style_at(spans: &[MatchSpan], theme: &Theme, x: u16, y: u16) -> Option<Style> {
        let mut found = None;
        for s in spans {
            if s.y == y && s.x0 <= x && x <= s.x1 {
                if s.current {
                    return Some(theme.current_match);
                }
                found = Some(theme.search_match);
            }
        }
        found
    }
}

/// One pane of the scene.
#[derive(Clone, Copy, Debug)]
pub struct ScenePane<'a> {
    /// Stable identity of the pane, so the renderer can tell a pane that
    /// stayed in place from a different pane that took its place.
    pub id: u64,
    /// Where the pane goes on the client screen. Normally the same size as
    /// the frame; if they differ (a resize in flight) the frame is cut or
    /// padded with blanks.
    pub rect: Rect,
    pub frame: &'a PaneFrame,
    pub active: bool,
    pub selection: Option<Selection>,
    /// Search matches to highlight (under the selection), any order.
    pub matches: &'a [MatchSpan],
    /// Right-aligned on the pane's first row, e.g. `[12/340]`.
    pub indicator: Option<&'a str>,
}

/// Turns a (pane index, hyperlink id) found in a frame cell into its URI.
pub type LinkLookup<'a> = &'a dyn Fn(usize, u32) -> Option<&'a str>;

/// Everything one client sees in one frame. A zoomed window is a scene
/// with one pane.
#[derive(Clone, Copy)]
pub struct Scene<'a> {
    pub size: Size,
    pub panes: &'a [ScenePane<'a>],
    pub status: Option<StatusBar<'a>>,
    pub message: Option<Message<'a>>,
    /// The cursor on the client screen: the active pane's (see
    /// [`pane_cursor`]) or the prompt's.
    pub cursor: Cursor,
    pub theme: Theme,
    /// Hyperlink resolution; without it cells are drawn without links.
    pub links: Option<LinkLookup<'a>>,
}

impl<'a> Scene<'a> {
    pub fn new(size: Size, panes: &'a [ScenePane<'a>]) -> Self {
        Scene {
            size,
            panes,
            status: None,
            message: None,
            cursor: Cursor::default(),
            theme: Theme::default(),
            links: None,
        }
    }

    /// The row the status line or message occupies, if any.
    pub fn chrome_row(&self) -> Option<u16> {
        if self.size.rows == 0 {
            return None;
        }
        match (&self.status, &self.message) {
            (Some(s), _) => Some(match s.position {
                StatusPosition::Top => 0,
                StatusPosition::Bottom => self.size.rows - 1,
            }),
            (None, Some(_)) => Some(self.size.rows - 1),
            (None, None) => None,
        }
    }
}

/// The cursor of a pane's frame, moved to client-screen coordinates; hidden
/// when it lies outside the pane's rectangle.
pub fn pane_cursor(rect: Rect, frame: &PaneFrame) -> Cursor {
    let mut c = frame.cursor;
    c.pos = c.pos.and_then(|p| {
        (p.x < rect.cols && p.y < rect.rows).then(|| Pos {
            x: rect.x + p.x,
            y: rect.y + p.y,
        })
    });
    c
}
