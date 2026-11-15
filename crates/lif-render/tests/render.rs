//! The renderer against a real terminal emulator: after feeding the bytes
//! of every frame to libghostty-vt, the screen must equal what a simple
//! reference compositor computes from the scene.

mod common;

use common::{Rng, Vt, diff, escape, reference};
use lif_render::{
    BorderLabel, MatchSpan, Message, Renderer, Scene, ScenePane, Selection, SelectionKind, StatusBar, StatusPosition,
    Theme,
};
use lif_types::{
    Attrs, Cell, CellWidth, Color, Cursor, CursorShape, OuterCaps, PaneFrame, Pos, Rect, Size,
    Span, Style, StyledLine, Underline,
};

const URIS: [&str; 5] = [
    "https://a.example/1",
    "https://b.example/x?y=2",
    "file:///tmp/f",
    "https://c.example/",
    "mailto:x@y.z",
];

fn lookup(pane: usize, id: u32) -> Option<&'static str> {
    Some(URIS[(pane + id as usize) % URIS.len()])
}

fn full_caps() -> OuterCaps {
    OuterCaps {
        truecolor: true,
        sync_output: true,
        styled_underline: true,
        underline_color: true,
        hyperlinks: true,
        overline: true,
        ..OuterCaps::default()
    }
}

/// Render, feed, compare; panics with details on a mismatch.
fn check(r: &mut Renderer, vt: &mut Vt, scene: &Scene<'_>, caps: OuterCaps, what: &str) -> Vec<u8> {
    let mut out = Vec::new();
    r.render(scene, &mut out);
    vt.feed(&out);
    let got = vt.shot();
    let want = reference(scene, caps);
    if let Some(d) = diff(&got, &want) {
        panic!(
            "{what}: screen differs from the scene\n{d}\n  bytes: {}",
            escape(&out)
        );
    }
    out
}

fn put(f: &mut PaneFrame, x: usize, y: usize, s: &str, style: Style) {
    let mut x = x;
    for ch in s.chars() {
        let w = lif_render::char_width(ch);
        let row = &mut f.rows[y];
        if w == 2 {
            row.cells[x] = Cell {
                ch,
                width: CellWidth::Wide,
                style,
                ..Cell::BLANK
            };
            row.cells[x + 1] = Cell {
                width: CellWidth::WideTail,
                style,
                ..Cell::BLANK
            };
        } else {
            row.cells[x] = Cell {
                ch,
                style,
                ..Cell::BLANK
            };
        }
        x += w;
    }
    f.dirty[y] = true;
}

fn single<'a>(size: Size, panes: &'a [ScenePane<'a>]) -> Scene<'a> {
    Scene::new(size, panes)
}

fn pane(id: u64, rect: Rect, frame: &PaneFrame, active: bool) -> ScenePane<'_> {
    ScenePane {
        id,
        rect,
        frame,
        active,
        selection: None,
        matches: &[],
        indicator: None,
        base: Default::default(),
    }
}

#[test]
fn first_frame_then_nothing() {
    let size = Size::new(20, 5);
    let mut f = PaneFrame::new(size);
    put(&mut f, 0, 0, "hello", Style::default());
    put(
        &mut f,
        3,
        2,
        "wörld 中文",
        Style {
            fg: Color::Indexed(1),
            attrs: Attrs::BOLD,
            ..Style::default()
        },
    );
    let panes = [pane(1, Rect::new(0, 0, 20, 5), &f, true)];
    let scene = single(size, &panes);
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(20, 5);
    check(&mut r, &mut vt, &scene, full_caps(), "first");
    // Nothing changed: zero bytes, even with dirty flags still set.
    let out = check(&mut r, &mut vt, &scene, full_caps(), "again");
    assert!(out.is_empty(), "{}", escape(&out));
    f.clear_dirty();
    let panes = [pane(1, Rect::new(0, 0, 20, 5), &f, true)];
    let out = check(&mut r, &mut vt, &single(size, &panes), full_caps(), "clean");
    assert!(out.is_empty());
}

#[test]
fn one_cell_change_is_small() {
    let size = Size::new(40, 10);
    let mut f = PaneFrame::new(size);
    for y in 0..10 {
        put(
            &mut f,
            0,
            y,
            "the quick brown fox jumps over the dog",
            Style::default(),
        );
    }
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(40, 10);
    {
        let panes = [pane(1, Rect::new(0, 0, 40, 10), &f, true)];
        check(&mut r, &mut vt, &single(size, &panes), full_caps(), "first");
    }
    f.clear_dirty();
    put(&mut f, 10, 4, "X", Style::default());
    let panes = [pane(1, Rect::new(0, 0, 40, 10), &f, true)];
    let out = check(
        &mut r,
        &mut vt,
        &single(size, &panes),
        full_caps(),
        "one cell",
    );
    // Sync on/off (16) + CUP (6) + 'X' (1).
    assert!(out.len() <= 24, "{} bytes: {}", out.len(), escape(&out));
}

#[test]
fn bottom_right_cell_does_not_scroll() {
    let size = Size::new(10, 3);
    let mut f = PaneFrame::new(size);
    put(&mut f, 0, 0, "top", Style::default());
    put(&mut f, 0, 2, "0123456789", Style::default());
    let panes = [pane(1, Rect::new(0, 0, 10, 3), &f, true)];
    for caps in [full_caps(), OuterCaps::default()] {
        let mut r = Renderer::new(caps);
        let mut vt = Vt::new(10, 3);
        check(&mut r, &mut vt, &single(size, &panes), caps, "bottom right");
    }
    // A wide character ending in the bottom-right corner.
    let mut f = PaneFrame::new(size);
    put(&mut f, 0, 0, "top", Style::default());
    put(&mut f, 8, 2, "中", Style::default());
    let panes = [pane(1, Rect::new(0, 0, 10, 3), &f, true)];
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(10, 3);
    check(
        &mut r,
        &mut vt,
        &single(size, &panes),
        full_caps(),
        "wide bottom right",
    );
}

#[test]
fn wide_char_cut_by_pane_edge() {
    // The frame is wider than the pane's rectangle (a resize in flight): the
    // wide character straddling the edge becomes a blank.
    let size = Size::new(11, 2);
    let mut f = PaneFrame::new(Size::new(8, 2));
    put(
        &mut f,
        0,
        0,
        "abcd中xy",
        Style {
            bg: Color::Indexed(4),
            ..Style::default()
        },
    );
    put(&mut f, 0, 1, "abc中xy", Style::default());
    let g = PaneFrame::new(Size::new(5, 2));
    let panes = [
        pane(1, Rect::new(0, 0, 5, 2), &f, true),
        pane(2, Rect::new(6, 0, 5, 2), &g, false),
    ];
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(11, 2);
    check(&mut r, &mut vt, &single(size, &panes), full_caps(), "cut");
    let shot = vt.shot();
    assert_eq!(shot.row_text(0), "abcd │     ");
    assert_eq!(shot.cells[4].style.bg, Color::Indexed(4));
}

fn grid(rects: &[Rect], cols: u16, rows: u16, active: usize) -> Vec<String> {
    let frames: Vec<PaneFrame> = rects.iter().map(|r| PaneFrame::new(r.size())).collect();
    let panes: Vec<ScenePane> = rects
        .iter()
        .zip(&frames)
        .enumerate()
        .map(|(i, (r, f))| pane(i as u64, *r, f, i == active))
        .collect();
    let mut scene = Scene::new(Size::new(cols, rows), &panes);
    scene.theme = Theme::default();
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(cols, rows);
    check(&mut r, &mut vt, &scene, full_caps(), "grid");
    let shot = vt.shot();
    (0..rows as usize).map(|y| shot.row_text(y)).collect()
}

#[test]
fn border_junctions() {
    assert_eq!(
        grid(&[Rect::new(0, 0, 5, 3)], 5, 3, 0),
        ["     ", "     ", "     "]
    );
    assert_eq!(
        grid(&[Rect::new(0, 0, 2, 2), Rect::new(3, 0, 2, 2)], 5, 2, 0),
        ["  │  ", "  │  "]
    );
    let three = [
        Rect::new(0, 0, 2, 5),
        Rect::new(3, 0, 2, 2),
        Rect::new(3, 3, 2, 2),
    ];
    assert_eq!(
        grid(&three, 5, 5, 0),
        ["  │  ", "  │  ", "  ├──", "  │  ", "  │  "]
    );
    let four = [
        Rect::new(0, 0, 2, 2),
        Rect::new(3, 0, 2, 2),
        Rect::new(0, 3, 2, 2),
        Rect::new(3, 3, 2, 2),
    ];
    assert_eq!(
        grid(&four, 5, 5, 0),
        ["  │  ", "  │  ", "──┼──", "  │  ", "  │  "]
    );
}

#[test]
fn active_border_color() {
    let size = Size::new(5, 4);
    let (f, g) = (
        PaneFrame::new(Size::new(2, 4)),
        PaneFrame::new(Size::new(2, 4)),
    );
    let panes = [
        pane(1, Rect::new(0, 0, 2, 4), &f, true),
        pane(2, Rect::new(3, 0, 2, 4), &g, false),
    ];
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(5, 4);
    check(
        &mut r,
        &mut vt,
        &single(size, &panes),
        full_caps(),
        "active",
    );
    let shot = vt.shot();
    let green = Color::Indexed(2);
    let fg: Vec<Color> = (0..4).map(|y| shot.cells[y * 5 + 2].style.fg).collect();
    assert_eq!(fg, [green, green, Color::Default, Color::Default]);
    // Switching the active pane moves the highlight to the bottom half.
    let panes = [
        pane(1, Rect::new(0, 0, 2, 4), &f, false),
        pane(2, Rect::new(3, 0, 2, 4), &g, true),
    ];
    check(
        &mut r,
        &mut vt,
        &single(size, &panes),
        full_caps(),
        "active 2",
    );
    let shot = vt.shot();
    let fg: Vec<Color> = (0..4).map(|y| shot.cells[y * 5 + 2].style.fg).collect();
    assert_eq!(fg, [Color::Default, Color::Default, green, green]);
}

fn scroll_frame(f: &mut PaneFrame, k: isize, mut fill: impl FnMut(usize) -> String) {
    let rows = f.rows.len();
    if k > 0 {
        f.rows.rotate_left(k as usize);
        for y in rows - k as usize..rows {
            f.rows[y].cells.fill(Cell::BLANK);
            f.rows[y].clusters.clear();
            let s = fill(y);
            put(f, 0, y, &s, Style::default());
        }
    } else {
        f.rows.rotate_right((-k) as usize);
        for y in 0..(-k) as usize {
            f.rows[y].cells.fill(Cell::BLANK);
            f.rows[y].clusters.clear();
            let s = fill(y);
            put(f, 0, y, &s, Style::default());
        }
    }
    f.mark_all_dirty();
}

#[test]
fn scrolling_uses_scroll_region() {
    // Full-width pane above a status line: DECSTBM + SU.
    let size = Size::new(30, 8);
    let mut f = PaneFrame::new(Size::new(30, 7));
    for y in 0..7 {
        put(
            &mut f,
            0,
            y,
            &format!("line {y} with some text"),
            Style {
                fg: Color::Indexed(y as u8 + 1),
                ..Style::default()
            },
        );
    }
    let status: StyledLine = vec![Span::plain("[lif] 0:sh*")];
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(30, 8);
    let scene_of = |f: &PaneFrame, r: &mut Renderer, vt: &mut Vt| {
        let panes = [pane(1, Rect::new(0, 0, 30, 7), f, true)];
        let mut s = Scene::new(size, &panes);
        s.status = Some(StatusBar {
            line: &status,
            fill: Style::default(),
            position: StatusPosition::Bottom,
        });
        check(r, vt, &s, full_caps(), "scroll")
    };
    scene_of(&f, &mut r, &mut vt);
    let mut n = 7;
    for k in [1isize, 3, -2, 1] {
        f.clear_dirty();
        scroll_frame(&mut f, k, |_| {
            n += 1;
            format!("line {n} new")
        });
        let out = scene_of(&f, &mut r, &mut vt);
        let s = escape(&out);
        assert!(s.contains("\\e[1;7r"), "k={k}: {s}");
        assert!(out.len() < 80, "k={k}: {} bytes: {s}", out.len());
    }
}

#[test]
fn scrolling_narrow_pane_with_lr_margins() {
    let size = Size::new(21, 6);
    let mut f = PaneFrame::new(Size::new(10, 6));
    let g = PaneFrame::new(Size::new(10, 6));
    for y in 0..6 {
        put(&mut f, 0, y, &format!("row {y}"), Style::default());
    }
    for lr in [false, true] {
        let mut f = f.clone();
        let caps = OuterCaps { lr_margins: lr, ..full_caps() };
        let mut r = Renderer::new(caps);
        let mut vt = Vt::new(21, 6);
        let run = |f: &PaneFrame, r: &mut Renderer, vt: &mut Vt| {
            let panes = [
                pane(1, Rect::new(11, 0, 10, 6), f, true),
                pane(2, Rect::new(0, 0, 10, 6), &g, false),
            ];
            check(r, vt, &Scene::new(size, &panes), full_caps(), "lr")
        };
        run(&f, &mut r, &mut vt);
        f.clear_dirty();
        scroll_frame(&mut f, 2, |y| format!("new {y}"));
        let out = run(&f, &mut r, &mut vt);
        assert_eq!(escape(&out).contains("?69h"), lr, "{}", escape(&out));
    }
}

#[test]
fn skipped_frames_are_caught_up() {
    // Two clients share one pane; the slow one skips frames but must end up
    // with the same screen once it renders again.
    let size = Size::new(16, 4);
    let mut f = PaneFrame::new(size);
    let mut fast = Renderer::new(full_caps());
    let mut slow = Renderer::new(full_caps());
    let (mut vf, mut vs) = (Vt::new(16, 4), Vt::new(16, 4));
    let rect = Rect::new(0, 0, 16, 4);
    {
        let panes = [pane(1, rect, &f, true)];
        let s = single(size, &panes);
        check(&mut fast, &mut vf, &s, full_caps(), "fast");
        check(&mut slow, &mut vs, &s, full_caps(), "slow");
    }
    f.clear_dirty();
    for (i, y) in [0usize, 2, 3, 1].into_iter().enumerate() {
        put(&mut f, i, y, "changed", Style::default());
        let panes = [pane(1, rect, &f, true)];
        let s = single(size, &panes);
        check(&mut fast, &mut vf, &s, full_caps(), "fast tick");
        slow.skip(&s);
        // The server clears the flags after every tick.
        f.clear_dirty();
    }
    let panes = [pane(1, rect, &f, true)];
    let out = check(
        &mut slow,
        &mut vs,
        &single(size, &panes),
        full_caps(),
        "slow catches up",
    );
    assert!(!out.is_empty());
}

#[test]
fn hyperlinks_and_downgrade() {
    let size = Size::new(12, 2);
    let mut f = PaneFrame::new(size);
    put(
        &mut f,
        0,
        0,
        "link",
        Style {
            fg: Color::Rgb(250, 10, 10),
            ..Style::default()
        },
    );
    for x in 0..4 {
        f.rows[0].cells[x].link = 1;
    }
    put(
        &mut f,
        0,
        1,
        "curly",
        Style {
            underline: Underline::Curly,
            underline_color: Color::Rgb(0, 0, 255),
            ..Style::default()
        },
    );
    let panes = [pane(1, Rect::new(0, 0, 12, 2), &f, true)];
    let lk = |p: usize, id: u32| -> Option<&str> { lookup(p, id) };
    for caps in [full_caps(), OuterCaps::default()] {
        let mut s = single(size, &panes);
        s.links = Some(&lk);
        let mut r = Renderer::new(caps);
        let mut vt = Vt::new(12, 2);
        let out = check(&mut r, &mut vt, &s, caps, "links");
        assert_eq!(escape(&out).contains("]8;;https"), caps.hyperlinks);
        let shot = vt.shot();
        let want = if caps.truecolor {
            Color::Rgb(250, 10, 10)
        } else {
            Color::Indexed(196)
        };
        assert_eq!(shot.cells[0].style.fg, want);
    }
}

#[test]
fn status_message_overlays_and_cursor() {
    let size = Size::new(24, 6);
    let mut f = PaneFrame::new(Size::new(24, 5));
    for y in 0..5 {
        put(&mut f, 0, y, "abcdefghijklmnopqrstuvwx", Style::default());
    }
    put(&mut f, 20, 0, "中", Style::default());
    let status: StyledLine = vec![
        Span::new(
            "[0] ",
            Style {
                bg: Color::Indexed(2),
                ..Style::default()
            },
        ),
        Span::plain("vim 中文 e\u{301}"),
    ];
    let msg: StyledLine = vec![Span::plain(":kill-pane")];
    // Matches, one current, one under the selections and one over the
    // wide character's tail only (which does not highlight it).
    let spans = [
        MatchSpan { y: 1, x0: 0, x1: 4, current: false },
        MatchSpan { y: 3, x0: 2, x1: 3, current: true },
        MatchSpan { y: 0, x0: 21, x1: 23, current: false },
    ];
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(24, 6);
    let sels = [
        None,
        Some(Selection {
            start: Pos { x: 3, y: 1 },
            end: Pos { x: 5, y: 3 },
            kind: SelectionKind::Stream,
        }),
        Some(Selection {
            start: Pos { x: 3, y: 1 },
            end: Pos { x: 5, y: 3 },
            kind: SelectionKind::Rect,
        }),
        Some(Selection {
            start: Pos { x: 9, y: 4 },
            end: Pos { x: 2, y: 2 },
            kind: SelectionKind::Lines,
        }),
        None,
    ];
    for (i, sel) in sels.into_iter().enumerate() {
        let panes = [ScenePane {
            selection: sel,
            matches: if i % 2 == 0 { &spans } else { &[] },
            indicator: (i % 2 == 1).then_some("[12/340]"),
            ..pane(1, Rect::new(0, 1, 24, 5), &f, true)
        }];
        let mut s = Scene::new(size, &panes);
        s.status = Some(StatusBar {
            line: &status,
            fill: Style {
                bg: Color::Indexed(2),
                ..Style::default()
            },
            position: StatusPosition::Top,
        });
        if i == 3 {
            s.message = Some(Message {
                line: &msg,
                fill: Style::default(),
            });
            s.cursor = Cursor {
                pos: Some(Pos { x: 10, y: 0 }),
                visible: true,
                shape: CursorShape::Bar,
                blinking: true,
            };
        } else {
            s.cursor = Cursor {
                pos: Some(Pos { x: i as u16, y: 3 }),
                visible: i != 2,
                shape: CursorShape::Block,
                blinking: false,
            };
        }
        check(&mut r, &mut vt, &s, full_caps(), &format!("overlay {i}"));
    }
}

// ---------------------------------------------------------------------------
// Randomized differential test.

struct P {
    id: u64,
    rect: Rect,
    frame: PaneFrame,
    sel: Option<Selection>,
    matches: Vec<MatchSpan>,
    ind: Option<String>,
}

struct World {
    rng: Rng,
    cols: u16,
    rows: u16,
    panes: Vec<P>,
    active: usize,
    status: Option<(StyledLine, Style, StatusPosition)>,
    msg: Option<(StyledLine, Style)>,
    cursor: Cursor,
    theme: Theme,
    next_id: u64,
}

fn rand_color(rng: &mut Rng) -> Color {
    match rng.below(5) {
        0 | 1 => Color::Default,
        2 => Color::Indexed(rng.below(16) as u8),
        3 => Color::Indexed(rng.below(256) as u8),
        _ => Color::Rgb(
            rng.below(256) as u8,
            rng.below(256) as u8,
            rng.below(256) as u8,
        ),
    }
}

fn rand_style(rng: &mut Rng) -> Style {
    if rng.chance(0.3) {
        return Style::default();
    }
    let mut s = Style {
        fg: rand_color(rng),
        bg: if rng.chance(0.5) {
            Color::Default
        } else {
            rand_color(rng)
        },
        ..Style::default()
    };
    for bit in [
        Attrs::BOLD,
        Attrs::FAINT,
        Attrs::ITALIC,
        Attrs::BLINK,
        Attrs::INVERSE,
        Attrs::STRIKETHROUGH,
        Attrs::OVERLINE,
    ] {
        if rng.chance(0.12) {
            s.attrs.insert(bit);
        }
    }
    if rng.chance(0.2) {
        s.underline = [
            Underline::Single,
            Underline::Double,
            Underline::Curly,
            Underline::Dotted,
            Underline::Dashed,
        ][rng.below(5) as usize];
        if rng.chance(0.5) {
            s.underline_color = rand_color(rng);
        }
    }
    s
}

const WIDE: [char; 6] = ['中', '文', '字', '😀', '🚀', '한'];
const NARROW: [char; 6] = ['é', '─', 'ß', 'λ', '→', '¥'];

fn rand_text(rng: &mut Rng, n: usize) -> String {
    (0..n)
        .map(|_| match rng.below(10) {
            0 => WIDE[rng.below(6) as usize],
            1 => NARROW[rng.below(6) as usize],
            2 => ' ',
            _ => (b'a' + rng.below(26) as u8) as char,
        })
        .collect()
}

/// Write one cell into a frame keeping wide pairs consistent, the way a
/// terminal does.
fn set_cell(f: &mut PaneFrame, x: usize, y: usize, c: Cell) {
    let cols = f.size.cols as usize;
    let row = &mut f.rows[y];
    let span = if c.width == CellWidth::Wide { 2 } else { 1 };
    for i in x..(x + span).min(cols) {
        if row.cells[i].width == CellWidth::WideTail && i > 0 && i == x {
            row.cells[i - 1] = Cell::BLANK;
        }
        if row.cells[i].width == CellWidth::Wide && i + 1 < cols {
            row.cells[i + 1] = Cell::BLANK;
        }
    }
    row.cells[x] = c;
    if span == 2 {
        row.cells[x + 1] = Cell {
            ch: ' ',
            extra: 0,
            width: CellWidth::WideTail,
            ..c
        };
    }
    f.dirty[y] = true;
}

fn write_run(rng: &mut Rng, f: &mut PaneFrame) {
    let (cols, rows) = (f.size.cols as usize, f.size.rows as usize);
    if cols == 0 || rows == 0 {
        return;
    }
    let y = rng.below(rows as u64) as usize;
    let mut x = rng.below(cols as u64) as usize;
    let len = rng.range(1, cols as u64) as usize;
    let style = rand_style(rng);
    let link = if rng.chance(0.1) {
        rng.range(1, 3) as u32
    } else {
        0
    };
    for _ in 0..len {
        if x >= cols {
            break;
        }
        let k = rng.below(20);
        let mut c = Cell {
            style,
            link,
            ..Cell::BLANK
        };
        if rng.chance(0.2) {
            c.style = rand_style(rng);
        }
        match k {
            0..=1 if x + 1 < cols => {
                c.ch = WIDE[rng.below(6) as usize];
                c.width = CellWidth::Wide;
            }
            2 => {
                let base = (b'a' + rng.below(26) as u8) as char;
                let mark = ['\u{301}', '\u{308}', '\u{323}'][rng.below(3) as usize];
                c.ch = base;
                f.rows[y].clusters.push(format!("{base}{mark}").into());
                c.extra = f.rows[y].clusters.len() as u16;
            }
            3 => c.ch = NARROW[rng.below(6) as usize],
            4..=6 => c.ch = ' ',
            _ => c.ch = (b'!' + rng.below(94) as u8) as char,
        }
        set_cell(f, x, y, c);
        x += if c.width == CellWidth::Wide { 2 } else { 1 };
    }
    // Rarely, what a broken engine might produce: a wide head in the last
    // column, or a lone tail.
    if rng.chance(0.03) {
        let y = rng.below(rows as u64) as usize;
        let x = if rng.chance(0.5) {
            cols - 1
        } else {
            rng.below(cols as u64) as usize
        };
        let w = if rng.chance(0.5) {
            CellWidth::Wide
        } else {
            CellWidth::WideTail
        };
        f.rows[y].cells[x] = Cell {
            ch: '中',
            width: w,
            style: rand_style(rng),
            ..Cell::BLANK
        };
        f.dirty[y] = true;
    }
}

fn scroll_rand(rng: &mut Rng, f: &mut PaneFrame) {
    let rows = f.size.rows as usize;
    if rows < 2 {
        return;
    }
    let k = rng.range(1, (rows as u64 - 1).min(6)) as usize;
    let up = rng.chance(0.7);
    if up {
        f.rows.rotate_left(k);
    } else {
        f.rows.rotate_right(k);
    }
    let fresh: Vec<usize> = if up {
        (rows - k..rows).collect()
    } else {
        (0..k).collect()
    };
    for y in fresh {
        f.rows[y].cells.fill(Cell::BLANK);
        f.rows[y].clusters.clear();
    }
    f.mark_all_dirty();
    if rng.chance(0.7) {
        write_run(rng, f);
    }
}

impl World {
    fn new(seed: u64) -> Self {
        let mut w = World {
            rng: Rng(seed | 1),
            cols: 0,
            rows: 0,
            panes: Vec::new(),
            active: 0,
            status: None,
            msg: None,
            cursor: Cursor::default(),
            theme: Theme::default(),
            next_id: 1,
        };
        w.cols = w.rng.range(6, 50) as u16;
        w.rows = w.rng.range(3, 16) as u16;
        if w.rng.chance(0.6) {
            w.new_status();
        }
        w.relayout();
        w
    }

    fn new_status(&mut self) {
        let mut line = Vec::new();
        for _ in 0..self.rng.range(1, 4) {
            let n = self.rng.range(1, 12) as usize;
            let mut t = rand_text(&mut self.rng, n);
            if self.rng.chance(0.2) {
                t.push('\u{301}');
            }
            line.push(Span::new(t, rand_style(&mut self.rng)));
        }
        let pos = if self.rng.chance(0.3) {
            StatusPosition::Top
        } else {
            StatusPosition::Bottom
        };
        self.status = Some((line, rand_style(&mut self.rng), pos));
    }

    fn area(&self) -> Rect {
        match &self.status {
            Some((_, _, StatusPosition::Top)) => Rect::new(0, 1, self.cols, self.rows - 1),
            Some(_) => Rect::new(0, 0, self.cols, self.rows - 1),
            None => Rect::new(0, 0, self.cols, self.rows),
        }
    }

    fn split(&mut self, r: Rect, depth: u32, out: &mut Vec<Rect>) {
        if depth == 0 || self.rng.chance(0.3) || (r.cols < 3 && r.rows < 3) {
            out.push(r);
            return;
        }
        let vertical = if r.cols < 3 {
            false
        } else if r.rows < 3 {
            true
        } else {
            self.rng.chance(0.5)
        };
        if vertical {
            let s = self.rng.range(1, r.cols as u64 - 2) as u16;
            self.split(Rect::new(r.x, r.y, s, r.rows), depth - 1, out);
            self.split(
                Rect::new(r.x + s + 1, r.y, r.cols - s - 1, r.rows),
                depth - 1,
                out,
            );
        } else {
            let s = self.rng.range(1, r.rows as u64 - 2) as u16;
            self.split(Rect::new(r.x, r.y, r.cols, s), depth - 1, out);
            self.split(
                Rect::new(r.x, r.y + s + 1, r.cols, r.rows - s - 1),
                depth - 1,
                out,
            );
        }
    }

    fn relayout(&mut self) {
        let area = self.area();
        let mut rects = Vec::new();
        if area.rows > 0 {
            let depth = self.rng.range(0, 3) as u32;
            self.split(area, depth, &mut rects);
        }
        let old = std::mem::take(&mut self.panes);
        let mut old = old.into_iter();
        for rect in rects {
            let p = match old.next() {
                Some(mut p) if self.rng.chance(0.7) => {
                    p.rect = rect;
                    // Mostly the engine follows the new size; sometimes the
                    // frame lags behind and is cut or padded.
                    if self.rng.chance(0.8) {
                        p.frame.reset(rect.size());
                        for _ in 0..rect.rows {
                            write_run(&mut self.rng, &mut p.frame);
                        }
                    }
                    p.sel = None;
                    p
                }
                _ => {
                    let mut frame = PaneFrame::new(rect.size());
                    for _ in 0..rect.rows {
                        write_run(&mut self.rng, &mut frame);
                    }
                    self.next_id += 1;
                    P {
                        id: self.next_id,
                        rect,
                        frame,
                        sel: None,
                        matches: Vec::new(),
                        ind: None,
                    }
                }
            };
            self.panes.push(p);
        }
        self.active = self.rng.below(self.panes.len().max(1) as u64) as usize;
    }

    fn mutate(&mut self, clients: &mut [Client]) {
        let rng = &mut self.rng;
        let n = self.panes.len();
        let pi = rng.below(n.max(1) as u64) as usize;
        match rng.below(100) {
            0..=44 if n > 0 => write_run(rng, &mut self.panes[pi].frame),
            45..=54 if n > 0 => scroll_rand(rng, &mut self.panes[pi].frame),
            55..=57 if n > 0 => {
                let f = &mut self.panes[pi].frame;
                let s = f.size;
                f.reset(s);
            }
            58..=60 => self.relayout(),
            61..=62 => {
                self.cols = self.rng.range(6, 50) as u16;
                self.rows = self.rng.range(3, 16) as u16;
                for c in clients.iter_mut() {
                    c.vt.resize(self.cols, self.rows);
                    // The server invalidates on every resize of the client's
                    // terminal: a resize and back between two rendered frames
                    // leaves the size unchanged but the screen reflowed.
                    c.r.invalidate();
                }
                self.relayout();
            }
            63..=67 => {
                if self.rng.chance(0.3) {
                    self.status = None;
                } else {
                    self.new_status();
                }
                self.relayout();
            }
            68..=69 => {
                self.status = self.status.take().map(|(mut l, s, p)| {
                    if let Some(sp) = l.first_mut() {
                        sp.text.push('x');
                    }
                    (l, s, p)
                });
            }
            70..=72 => {
                self.msg = if self.msg.is_some() {
                    None
                } else {
                    let t = rand_text(&mut self.rng, 10);
                    Some((
                        vec![Span::new(t, rand_style(&mut self.rng))],
                        rand_style(&mut self.rng),
                    ))
                };
            }
            73..=77 if n > 0 => {
                let p = &mut self.panes[pi];
                p.sel = if self.rng.chance(0.25) {
                    None
                } else {
                    let r = p.rect;
                    let mut pos = || Pos {
                        x: self.rng.below(r.cols as u64 + 1) as u16,
                        y: self.rng.below(r.rows as u64 + 1) as u16,
                    };
                    let (start, end) = (pos(), pos());
                    let kind = [
                        SelectionKind::Stream,
                        SelectionKind::Lines,
                        SelectionKind::Rect,
                    ][self.rng.below(3) as usize];
                    Some(Selection { start, end, kind })
                };
            }
            78..=81 if n > 0 => {
                let v = self.rng.below(400);
                self.panes[pi].ind = match self.rng.below(4) {
                    0 => None,
                    1 => Some(format!("[{v}/中{}]", v * 3)),
                    _ => Some(format!("[{v}/{}]", v * 3)),
                };
            }
            82..=91 => {
                let r = &mut self.rng;
                self.cursor = Cursor {
                    pos: if r.chance(0.9) {
                        Some(Pos {
                            x: r.below(self.cols as u64 + 1) as u16,
                            y: r.below(self.rows as u64) as u16,
                        })
                    } else {
                        None
                    },
                    visible: r.chance(0.8),
                    shape: [CursorShape::Block, CursorShape::Bar, CursorShape::Underline]
                        [r.below(3) as usize],
                    blinking: r.chance(0.5),
                };
            }
            92..=94 => self.active = self.rng.below(n.max(1) as u64) as usize,
            95 => {
                self.theme = Theme {
                    border: rand_style(&mut self.rng),
                    active_border: rand_style(&mut self.rng),
                    ..Theme::default()
                };
                self.theme.selection = rand_style(&mut self.rng);
                self.theme.search_match = rand_style(&mut self.rng);
                self.theme.current_match = rand_style(&mut self.rng);
            }
            98..=99 if n > 0 => {
                // Search matches: a few spans, possibly overlapping each
                // other, the selection, wide characters and the pane edge.
                let p = &mut self.panes[pi];
                let r = p.rect;
                p.matches.clear();
                for _ in 0..self.rng.below(5) {
                    let x0 = self.rng.below(r.cols as u64 + 1) as u16;
                    p.matches.push(MatchSpan {
                        y: self.rng.below(r.rows as u64 + 1) as u16,
                        x0,
                        x1: x0 + self.rng.below(8) as u16,
                        current: self.rng.chance(0.3),
                    });
                }
            }
            96 if !clients.is_empty() => {
                let i = self.rng.below(clients.len() as u64) as usize;
                clients[i].caps = rand_caps(&mut self.rng);
                let caps = clients[i].caps;
                clients[i].r.set_caps(caps);
            }
            97 if !clients.is_empty() => {
                let i = self.rng.below(clients.len() as u64) as usize;
                clients[i].r.invalidate();
            }
            _ => {}
        }
    }
}

fn rand_caps(rng: &mut Rng) -> OuterCaps {
    OuterCaps {
        truecolor: rng.chance(0.5),
        sync_output: rng.chance(0.5),
        styled_underline: rng.chance(0.5),
        underline_color: rng.chance(0.5),
        hyperlinks: rng.chance(0.5),
        overline: rng.chance(0.5),
        grapheme_clusters: rng.chance(0.3),
        lr_margins: rng.chance(0.5),
        ..OuterCaps::default()
    }
}

struct Client {
    r: Renderer,
    vt: Vt,
    caps: OuterCaps,
    skip: f64,
    bytes: usize,
    frames: usize,
}

static SCROLLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn run_seed(seed: u64, steps: usize) -> (usize, usize) {
    let mut w = World::new(seed);
    let mut clients: Vec<Client> = (0..2)
        .map(|i| {
            let caps = rand_caps(&mut w.rng);
            let r = Renderer::new(caps);
            Client {
                r,
                vt: Vt::new(w.cols, w.rows),
                caps,
                skip: if i == 0 { 0.0 } else { 0.6 },
                bytes: 0,
                frames: 0,
            }
        })
        .collect();
    let mut out = Vec::new();
    for step in 0..steps {
        for _ in 0..w.rng.range(1, 4) {
            w.mutate(&mut clients);
        }
        let status_line;
        let msg_line;
        let panes: Vec<ScenePane> = w
            .panes
            .iter()
            .enumerate()
            .map(|(i, p)| ScenePane {
                id: p.id,
                rect: p.rect,
                frame: &p.frame,
                active: i == w.active,
                selection: p.sel,
                matches: &p.matches,
                indicator: p.ind.as_deref(),
                base: Default::default(),
            })
            .collect();
        let mut scene = Scene::new(Size::new(w.cols, w.rows), &panes);
        if let Some((l, fill, pos)) = &w.status {
            status_line = l.clone();
            scene.status = Some(StatusBar {
                line: &status_line,
                fill: *fill,
                position: *pos,
            });
        }
        if let Some((l, fill)) = &w.msg {
            msg_line = l.clone();
            scene.message = Some(Message {
                line: &msg_line,
                fill: *fill,
            });
        }
        scene.cursor = w.cursor;
        scene.theme = w.theme;
        let lk = |p: usize, id: u32| -> Option<&str> { lookup(p, id) };
        scene.links = Some(&lk);
        for (ci, c) in clients.iter_mut().enumerate() {
            if w.rng.chance(c.skip) {
                c.r.skip(&scene);
                continue;
            }
            out.clear();
            c.r.render(&scene, &mut out);
            c.vt.feed(&out);
            c.bytes += out.len();
            if out.windows(2).any(|w| w == b"S\x1b" || w == b"T\x1b") || out.ends_with(b"S") {
                SCROLLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            c.frames += 1;
            let got = c.vt.shot();
            let want = reference(&scene, c.caps);
            let (bx, by) = c.r.believed_cursor();
            if bx.is_some_and(|x| x != got.raw_cursor.0)
                || by.is_some_and(|y| y != got.raw_cursor.1)
            {
                panic!(
                    "seed {seed} step {step} client {ci}: cursor believed at {:?}, is at {:?}\n  bytes: {}",
                    (bx, by),
                    got.raw_cursor,
                    escape(&out)
                );
            }
            if let Some(d) = diff(&got, &want) {
                panic!(
                    "seed {seed} step {step} client {ci} caps {:?}\n{d}\n  bytes: {}",
                    c.caps,
                    escape(&out)
                );
            }
            if w.rng.chance(0.1) {
                out.clear();
                c.r.render(&scene, &mut out);
                assert!(
                    out.is_empty(),
                    "seed {seed} step {step}: unchanged scene drew {}",
                    escape(&out)
                );
            }
        }
        for p in &mut w.panes {
            p.frame.clear_dirty();
        }
    }
    (
        clients.iter().map(|c| c.bytes).sum(),
        clients.iter().map(|c| c.frames).sum(),
    )
}

#[test]
fn differential_random() {
    let mut total = (0, 0);
    let seeds: Vec<u64> = match std::env::var("LIF_RENDER_SEEDS") {
        Ok(n) => match n.split_once("..") {
            Some((a, b)) => (a.parse().unwrap()..=b.parse().unwrap()).collect(),
            None => (1..=n.parse().unwrap()).collect(),
        },
        Err(_) => vec![1, 2, 3, 0xdead_beef, 42, 7, 1234567, 99, 5, 11, 13, 17],
    };
    for seed in seeds {
        let (b, f) = run_seed(seed, 2000);
        total.0 += b;
        total.1 += f;
    }
    eprintln!(
        "differential: {} frames, {} bytes, ~{} with scrolls",
        total.1,
        total.0,
        SCROLLS.load(std::sync::atomic::Ordering::Relaxed)
    );
}

#[test]
fn border_labels_and_pane_base_colors() {
    // Two panes side by side under a label row (pane-border-status top).
    let size = Size::new(21, 5);
    let mut f = PaneFrame::new(Size::new(10, 4));
    let g = PaneFrame::new(Size::new(10, 4));
    put(&mut f, 0, 0, "hi", Style::default());
    put(&mut f, 0, 1, "red", Style { fg: Color::Indexed(1), ..Style::default() });
    let blue = Style { fg: Color::Indexed(4), bg: Color::Indexed(8), ..Style::default() };
    let panes = [
        ScenePane { base: blue, ..pane(1, Rect::new(0, 1, 10, 4), &f, true) },
        pane(2, Rect::new(11, 1, 10, 4), &g, false),
    ];
    let bold = Style { attrs: Attrs::BOLD, fg: Color::Indexed(3), ..Style::default() };
    let one: StyledLine = vec![Span::new(" lead ".to_string(), bold)];
    let two: StyledLine = vec![Span::new(" 中x helper-with-a-long-name".to_string(), Style::default())];
    let labels = [
        BorderLabel { x: 2, y: 0, width: 6, line: &one },
        BorderLabel { x: 13, y: 0, width: 6, line: &two },
    ];
    let mut scene = single(size, &panes);
    scene.labels = &labels;
    let mut r = Renderer::new(full_caps());
    let mut vt = Vt::new(21, 5);
    check(&mut r, &mut vt, &scene, full_caps(), "labels");
    let shot = vt.shot();
    assert_eq!(shot.row_text(0), "── lead ──┬── 中x h──");
    assert_eq!(shot.cells[3].style.attrs, Attrs::BOLD);
    // window-style: default colors take the pane's base, explicit ones stay.
    assert_eq!(shot.cells[21].style.fg, Color::Indexed(4), "default fg");
    assert_eq!(shot.cells[2 * 21].style.fg, Color::Indexed(1), "explicit fg");
    assert_eq!(shot.cells[21 + 5].style.bg, Color::Indexed(8), "blank cell");
    // A changed label repaints its row; an unchanged scene sends nothing.
    let one: StyledLine = vec![Span::new(" boss ".to_string(), bold)];
    let labels = [BorderLabel { x: 2, y: 0, width: 6, line: &one }, labels[1]];
    scene.labels = &labels;
    let out = check(&mut r, &mut vt, &scene, full_caps(), "label changed");
    assert!(!out.is_empty());
    assert_eq!(vt.shot().row_text(0), "── boss ──┬── 中x h──");
    let out = check(&mut r, &mut vt, &scene, full_caps(), "same");
    assert!(out.is_empty(), "{}", escape(&out));
    // A wide character cut by the label's width becomes a blank.
    let three: StyledLine = vec![Span::new("abcde中".to_string(), Style::default())];
    let labels = [BorderLabel { x: 2, y: 0, width: 6, line: &three }];
    scene.labels = &labels;
    check(&mut r, &mut vt, &scene, full_caps(), "wide cut");
}
