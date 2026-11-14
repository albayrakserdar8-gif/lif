use super::*;

fn p(n: u32) -> PaneId {
    PaneId(n)
}

/// Pane geometry as `index:x,y,WxH` in the given order, like the
/// `list-panes -F` strings recorded from tmux 3.8.
fn show(l: &Layout, order: &[PaneId]) -> String {
    let g = l.geometry();
    order
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let r = g.rect(*id).unwrap();
            format!("{i}:{},{},{}x{}", r.x, r.y, r.cols, r.rows)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sizes(l: &Layout, order: &[PaneId]) -> String {
    let g = l.geometry();
    order
        .iter()
        .map(|id| {
            let r = g.rect(*id).unwrap();
            format!("{}x{}", r.cols, r.rows)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every cell of the layout is covered exactly once by a pane or a border.
fn assert_tiles(l: &Layout) {
    l.check().unwrap();
    let s = l.size();
    let (w, h) = (s.cols as usize, s.rows as usize);
    let mut grid = vec![0u8; w * h];
    let g = l.unzoomed_geometry();
    for r in g.panes.iter().map(|(_, r)| r).chain(g.borders.iter()) {
        assert!(r.cols as u32 >= PANE_MINIMUM && r.rows as u32 >= PANE_MINIMUM, "{r:?}");
        assert!(r.right() as usize <= w && r.bottom() as usize <= h, "{r:?} outside {w}x{h}");
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                grid[y as usize * w + x as usize] += 1;
            }
        }
    }
    assert!(grid.iter().all(|c| *c == 1), "coverage {:?}", grid.iter().filter(|c| **c != 1).count());
}

#[test]
fn split_matches_tmux() {
    // tmux 3.8, 80x24: "splitw -h" -> 40x24 + 39x24 at 41,0; then
    // "splitw -v -t 0" -> 40x12 / 40x11 at 0,13.
    let mut l = Layout::new(p(0), Size::new(80, 24));
    l.split(p(0), p(1), Axis::LeftRight, SplitSize::Half, false).unwrap();
    assert_eq!(show(&l, &[p(0), p(1)]), "0:0,0,40x24 1:41,0,39x24");
    l.split(p(0), p(2), Axis::TopBottom, SplitSize::Half, false).unwrap();
    assert_eq!(show(&l, &[p(0), p(2), p(1)]), "0:0,0,40x12 1:0,13,40x11 2:41,0,39x24");
    assert_tiles(&l);
}

#[test]
fn split_sizes_and_before_match_tmux() {
    // tmux 3.8, 81x24: splitw -h -l 30%; splitw -b -v -l 5 (on the new
    // pane); splitw -b -h -t 0 -> "0 0,0 28x24 / 1 29,0 27x24 / 2 57,0 24x5 /
    // 3 57,6 24x18" (list order D A C B).
    let (a, b, c, d) = (p(0), p(1), p(2), p(3));
    let mut l = Layout::new(a, Size::new(81, 24));
    l.split(a, b, Axis::LeftRight, SplitSize::Percent(30), false).unwrap();
    l.split(b, c, Axis::TopBottom, SplitSize::Cells(5), true).unwrap();
    l.split(a, d, Axis::LeftRight, SplitSize::Half, true).unwrap();
    assert_eq!(show(&l, &[d, a, c, b]), "0:0,0,28x24 1:29,0,27x24 2:57,0,24x5 3:57,6,24x18");
    assert_eq!(l.panes(), vec![d, a, c, b]);
    assert_tiles(&l);
    // Oversized requests are clamped, not refused (tmux: splitw -l 200).
    let mut l = Layout::new(a, Size::new(80, 24));
    l.split(a, b, Axis::LeftRight, SplitSize::Cells(200), false).unwrap();
    assert_eq!(sizes(&l, &[a, b]), "1x24 78x24");
}

#[test]
fn no_space_matches_tmux() {
    // tmux 3.8 in a 3x3 window: splitw -h works (1x3 1x3), a second -h fails
    // with "no space for a new pane"; -v on a 1x3 pane gives 1x1 + 1x1.
    let mut l = Layout::new(p(0), Size::new(3, 3));
    l.split(p(0), p(1), Axis::LeftRight, SplitSize::Half, false).unwrap();
    assert_eq!(sizes(&l, &[p(0), p(1)]), "1x3 1x3");
    let e = l.split(p(1), p(2), Axis::LeftRight, SplitSize::Half, false).unwrap_err();
    assert_eq!(e.to_string(), "no space for a new pane");
    l.split(p(1), p(2), Axis::TopBottom, SplitSize::Half, false).unwrap();
    assert_eq!(show(&l, &[p(0), p(1), p(2)]), "0:0,0,1x3 1:2,0,1x1 2:2,2,1x1");
    assert!(l.split(p(2), p(3), Axis::TopBottom, SplitSize::Half, false).is_err());
    assert_tiles(&l);
}

#[test]
fn presets_match_tmux_80x24() {
    let order = [p(0), p(2), p(1)];
    let mut l = Layout::new(p(0), Size::new(80, 24));
    l.split(p(0), p(1), Axis::LeftRight, SplitSize::Half, false).unwrap();
    l.split(p(0), p(2), Axis::TopBottom, SplitSize::Half, false).unwrap();
    let size = Size::new(80, 24);
    let pp = PresetParams::default();
    let cases = [
        (Preset::EvenHorizontal, "0:0,0,26x24 1:27,0,26x24 2:54,0,26x24"),
        (Preset::EvenVertical, "0:0,0,80x8 1:0,9,80x7 2:0,17,80x7"),
        (Preset::MainHorizontal, "0:0,0,80x22 1:0,23,40x1 2:41,23,39x1"),
        (Preset::MainVertical, "0:0,0,78x24 1:79,0,1x12 2:79,13,1x11"),
        (Preset::Tiled, "0:0,0,39x11 1:40,0,40x11 2:0,12,80x12"),
        (Preset::MainVerticalMirrored, "0:2,0,78x24 1:0,0,1x12 2:0,13,1x11"),
    ];
    for (preset, want) in cases {
        l.apply_preset(preset, &order, size, &pp);
        assert_eq!(show(&l, &order), want, "{}", preset.name());
        assert_tiles(&l);
    }
}

#[test]
fn presets_match_tmux_five_panes() {
    let order: Vec<PaneId> = (0..5).map(p).collect();
    let mut l = Layout::new(p(0), Size::new(80, 24));
    for i in 1..5 {
        l.split(p(i - 1), p(i), Axis::LeftRight, SplitSize::Cells(1), false).unwrap_or(());
    }
    let mut l2 = Layout::new(p(0), Size::new(80, 24));
    l2.apply_preset(Preset::Tiled, &order, Size::new(80, 24), &PresetParams::default());
    let _ = &mut l;
    let pp = PresetParams::default();
    let cases = [
        (Preset::Tiled, "0:0,0,39x7 1:40,0,40x7 2:0,8,39x7 3:40,8,40x7 4:0,16,80x8"),
        (Preset::MainVertical, "0:0,0,78x24 1:79,0,1x6 2:79,7,1x5 3:79,13,1x5 4:79,19,1x5"),
        (Preset::MainHorizontal, "0:0,0,80x22 1:0,23,20x1 2:21,23,19x1 3:41,23,19x1 4:61,23,19x1"),
    ];
    for (preset, want) in cases {
        l2.apply_preset(preset, &order, Size::new(80, 24), &pp);
        assert_eq!(show(&l2, &order), want, "{}", preset.name());
        assert_tiles(&l2);
    }
}

#[test]
fn presets_match_tmux_200x50() {
    let order = [p(0), p(1), p(2)];
    let size = Size::new(200, 50);
    let mut l = Layout::new(p(0), size);
    let mut pp = PresetParams::default();
    l.apply_preset(Preset::MainVertical, &order, size, &pp);
    assert_eq!(show(&l, &order), "0:0,0,80x50 1:81,0,119x25 2:81,26,119x24");
    l.apply_preset(Preset::MainHorizontal, &order, size, &pp);
    assert_eq!(show(&l, &order), "0:0,0,200x24 1:0,25,100x25 2:101,25,99x25");
    pp.other_pane_width = SizeSpec::Cells(30);
    pp.other_pane_height = SizeSpec::Cells(10);
    l.apply_preset(Preset::MainVertical, &order, size, &pp);
    assert_eq!(show(&l, &order), "0:0,0,169x50 1:170,0,30x25 2:170,26,30x24");
    l.apply_preset(Preset::MainHorizontal, &order, size, &pp);
    assert_eq!(show(&l, &order), "0:0,0,200x39 1:0,40,100x10 2:101,40,99x10");
    let pp = PresetParams {
        main_pane_width: SizeSpec::parse("50%").unwrap(),
        main_pane_height: SizeSpec::parse("30%").unwrap(),
        ..PresetParams::default()
    };
    l.apply_preset(Preset::MainVertical, &order, size, &pp);
    assert_eq!(show(&l, &order), "0:0,0,99x50 1:100,0,100x25 2:100,26,100x24");
    l.apply_preset(Preset::MainHorizontal, &order, size, &pp);
    assert_eq!(show(&l, &order), "0:0,0,200x14 1:0,15,100x35 2:101,15,99x35");
    let two = [p(0), p(1)];
    l.apply_preset(Preset::MainVertical, &two, size, &PresetParams::default());
    assert_eq!(show(&l, &two), "0:0,0,80x50 1:81,0,119x50");
    let seven: Vec<PaneId> = (0..7).map(p).collect();
    l.apply_preset(Preset::Tiled, &seven, size, &PresetParams::default());
    assert_eq!(
        show(&l, &seven),
        "0:0,0,66x16 1:67,0,66x16 2:134,0,66x16 3:0,17,66x16 4:67,17,66x16 5:134,17,66x16 6:0,34,200x16"
    );
}

#[test]
fn even_layouts_match_tmux_odd_widths() {
    // tmux 3.8: even-horizontal for 2/3/4 panes at widths 80..83, then
    // even-vertical for 4 panes at 24 rows.
    let want = [
        (80, ["40x24 39x24", "26x24 26x24 26x24", "20x24 19x24 19x24 19x24"]),
        (81, ["40x24 40x24", "27x24 26x24 26x24", "20x24 20x24 19x24 19x24"]),
        (82, ["41x24 40x24", "27x24 27x24 26x24", "20x24 20x24 20x24 19x24"]),
        (83, ["41x24 41x24", "27x24 27x24 27x24", "20x24 20x24 20x24 20x24"]),
    ];
    for (w, rows) in want {
        let size = Size::new(w, 24);
        for (k, want) in rows.iter().enumerate() {
            let order: Vec<PaneId> = (0..k as u32 + 2).map(p).collect();
            let mut l = Layout::new(p(0), size);
            l.apply_preset(Preset::EvenHorizontal, &order, size, &PresetParams::default());
            assert_eq!(&sizes(&l, &order), want, "w={w}");
        }
        let order: Vec<PaneId> = (0..4).map(p).collect();
        let mut l = Layout::new(p(0), size);
        l.apply_preset(Preset::EvenVertical, &order, size, &PresetParams::default());
        assert_eq!(sizes(&l, &order), format!("{w}x6 {w}x5 {w}x5 {w}x5"));
        // resize-pane -t 0 -y 2, then select-layout -E -> back to 6/5/5/5.
        l.resize_pane_to(p(0), Axis::TopBottom, 2);
        assert_eq!(sizes(&l, &order), format!("{w}x2 {w}x9 {w}x5 {w}x5"));
        l.spread_out(p(0));
        assert_eq!(sizes(&l, &order), format!("{w}x6 {w}x5 {w}x5 {w}x5"));
    }
}

#[test]
fn resize_sequence_matches_tmux() {
    // tmux 3.8, 80x24: splitw -h; splitw -v; splitw -v, then a series of
    // resize-pane commands; expected strings are tmux's list-panes output.
    let (a, b, c, d) = (p(0), p(1), p(2), p(3));
    let order = [a, b, c, d];
    let mut l = Layout::new(a, Size::new(80, 24));
    l.split(a, b, Axis::LeftRight, SplitSize::Half, false).unwrap();
    l.split(b, c, Axis::TopBottom, SplitSize::Half, false).unwrap();
    l.split(c, d, Axis::TopBottom, SplitSize::Half, false).unwrap();
    assert_eq!(show(&l, &order), "0:0,0,40x24 1:41,0,39x12 2:41,13,39x5 3:41,19,39x5");
    l.resize_pane(a, Direction::Right, 10);
    assert_eq!(show(&l, &order), "0:0,0,50x24 1:51,0,29x12 2:51,13,29x5 3:51,19,29x5");
    l.resize_pane(b, Direction::Down, 3);
    assert_eq!(show(&l, &order), "0:0,0,50x24 1:51,0,29x15 2:51,16,29x2 3:51,19,29x5");
    l.resize_pane(d, Direction::Up, 30);
    assert_eq!(show(&l, &order), "0:0,0,50x24 1:51,0,29x1 2:51,2,29x1 3:51,4,29x20");
    l.resize_pane(c, Direction::Left, 100);
    assert_eq!(show(&l, &order), "0:0,0,1x24 1:2,0,78x1 2:2,2,78x1 3:2,4,78x20");
    l.resize_pane_to(a, Axis::LeftRight, 20);
    assert_eq!(show(&l, &order), "0:0,0,20x24 1:21,0,59x1 2:21,2,59x1 3:21,4,59x20");
    l.resize_pane_to(d, Axis::TopBottom, 4);
    assert_eq!(show(&l, &order), "0:0,0,20x24 1:21,0,59x1 2:21,2,59x17 3:21,20,59x4");
    l.resize_pane_to(b, Axis::LeftRight, 40); // -x 50%
    assert_eq!(show(&l, &order), "0:0,0,39x24 1:40,0,40x1 2:40,2,40x17 3:40,20,40x4");
    l.spread_out(d);
    assert_eq!(show(&l, &order), "0:0,0,39x24 1:40,0,40x8 2:40,9,40x7 3:40,17,40x7");
    l.spread_out(a); // already even: unchanged
    assert_eq!(show(&l, &order), "0:0,0,39x24 1:40,0,40x8 2:40,9,40x7 3:40,17,40x7");
    assert!(l.remove(c));
    assert_eq!(show(&l, &[a, b, d]), "0:0,0,39x24 1:40,0,40x8 2:40,9,40x15");
    assert!(l.remove(a));
    assert_eq!(show(&l, &[b, d]), "0:0,0,80x8 1:0,9,80x15");
    assert_tiles(&l);
    assert!(l.remove(b));
    assert!(!l.remove(d), "last pane cannot be removed");
}

#[test]
fn close_gives_space_to_next_then_previous() {
    // tmux 3.8: even-horizontal 4 panes at 80 cols = 20 19 19 19; resize
    // pane 1 to 5 -> 20 5 33 19; kill index 2 -> 20 5 53; kill index 2 ->
    // 20 59.
    let order: Vec<PaneId> = (0..4).map(p).collect();
    let size = Size::new(80, 24);
    let mut l = Layout::new(p(0), size);
    l.apply_preset(Preset::EvenHorizontal, &order, size, &PresetParams::default());
    l.resize_pane_to(p(1), Axis::LeftRight, 5);
    assert_eq!(sizes(&l, &order), "20x24 5x24 33x24 19x24");
    l.remove(p(2));
    assert_eq!(sizes(&l, &[p(0), p(1), p(3)]), "20x24 5x24 53x24");
    l.remove(p(3));
    assert_eq!(sizes(&l, &[p(0), p(1)]), "20x24 59x24");
}

#[test]
fn neighbors_wrap_like_tmux() {
    // Layout from tmux: %0 left 40x24; %2 at 41,0 39x12; %1 at 41,13 39x11.
    let mut l = Layout::new(p(0), Size::new(80, 24));
    l.split(p(0), p(2), Axis::LeftRight, SplitSize::Half, false).unwrap();
    l.split(p(2), p(1), Axis::TopBottom, SplitSize::Half, false).unwrap();
    let order = [p(0), p(2), p(1)];
    let rec = |id: PaneId| match id.0 {
        0 => 1,
        1 => 2,
        _ => 3,
    };
    // tmux: from %2, -R wraps to %0; from %0, -L wraps to the most recently
    // active right pane (%2); from %2, -U wraps to %1.
    assert_eq!(l.neighbor(p(2), Direction::Right, &order, rec), Some(p(0)));
    assert_eq!(l.neighbor(p(0), Direction::Left, &order, rec), Some(p(2)));
    assert_eq!(l.neighbor(p(2), Direction::Up, &order, rec), Some(p(1)));
    assert_eq!(l.neighbor(p(2), Direction::Down, &order, rec), Some(p(1)));
    assert_eq!(l.neighbor(p(0), Direction::Right, &order, |id| if id == p(1) { 9 } else { 0 }), Some(p(1)));
    assert_eq!(l.neighbor(p(0), Direction::Up, &order, rec), None);
    // tmux: select-pane -t '{right}' picks %2 (the middle row is a border
    // that belongs to the pane above it).
    assert_eq!(l.pane_at_position("right", &order), Some(p(2)));
    assert_eq!(l.pane_at_position("bottom-right", &order), Some(p(1)));
    assert_eq!(l.pane_at_position("left", &order), Some(p(0)));
}

#[test]
fn zoom_geometry() {
    let mut l = Layout::new(p(0), Size::new(80, 24));
    assert!(!l.zoom(p(0)), "single pane cannot zoom");
    l.split(p(0), p(1), Axis::LeftRight, SplitSize::Half, false).unwrap();
    assert!(l.zoom(p(1)));
    let g = l.geometry();
    assert_eq!(g.panes, vec![(p(1), Rect::new(0, 0, 80, 24))]);
    assert!(g.borders.is_empty());
    assert_eq!(l.unzoomed_geometry().panes.len(), 2);
    assert!(l.unzoom());
    assert!(!l.unzoom());
    l.zoom(p(0));
    l.split(p(0), p(2), Axis::TopBottom, SplitSize::Half, false).unwrap();
    assert_eq!(l.zoomed(), None, "split unzooms");
}

#[test]
fn swap_rotate_replace() {
    let mut l = Layout::new(p(0), Size::new(80, 24));
    l.split(p(0), p(1), Axis::LeftRight, SplitSize::Half, false).unwrap();
    l.swap(p(0), p(1));
    assert_eq!(l.panes(), vec![p(1), p(0)]);
    l.reassign(&[p(1), p(0)], &[p(0), p(1)]);
    assert_eq!(l.panes(), vec![p(0), p(1)]);
    l.replace(p(1), p(7));
    assert_eq!(l.panes(), vec![p(0), p(7)]);
}

#[test]
fn window_resize_is_proportional() {
    // A 3:1 split stays 3:1 when the window doubles and comes back exactly.
    let mut l = Layout::new(p(0), Size::new(81, 24));
    l.split(p(0), p(1), Axis::LeftRight, SplitSize::Cells(20), false).unwrap();
    assert_eq!(sizes(&l, &[p(0), p(1)]), "60x24 20x24");
    l.resize(Size::new(161, 48));
    assert_eq!(sizes(&l, &[p(0), p(1)]), "120x48 40x48");
    l.resize(Size::new(81, 24));
    assert_eq!(sizes(&l, &[p(0), p(1)]), "60x24 20x24");
    assert_eq!(l.compute(Size::new(41, 10)).rect(p(0)).unwrap().cols, 30);
    assert_eq!(l.size(), Size::new(81, 24), "compute does not change the layout");
    // Too small: the layout keeps its minimum and is clipped by the caller.
    l.resize(Size::new(2, 1));
    assert_eq!(l.size(), Size::new(3, 1));
    assert_tiles(&l);
}

#[test]
fn dump_and_parse() {
    // From the tmux manual: "bb62,159x48,0,0{79x48,0,0,79x48,80,0}".
    let l = Layout::parse("bb62,159x48,0,0{79x48,0,0,79x48,80,0}", &[p(4), p(5)], Size::new(159, 48)).unwrap();
    assert_eq!(sizes(&l, &[p(4), p(5)]), "79x48 79x48");
    let s = l.dump();
    assert!(s.ends_with(",159x48,0,0{79x48,0,0,4,79x48,80,0,5}"), "{s}");
    let back = Layout::parse(&s, &[p(4), p(5)], Size::new(159, 48)).unwrap();
    assert_eq!(back, l);
    assert!(Layout::parse("0000,159x48,0,0{79x48,0,0,79x48,80,0}", &[p(4), p(5)], Size::new(159, 48)).is_err());
    let e = Layout::parse("bb62,159x48,0,0{79x48,0,0,79x48,80,0}", &[p(4)], Size::new(159, 48)).unwrap_err();
    assert_eq!(e.to_string(), "have 1 panes but need 2");
    let mut nested = Layout::new(p(0), Size::new(80, 24));
    nested.split(p(0), p(1), Axis::LeftRight, SplitSize::Half, false).unwrap();
    nested.split(p(1), p(2), Axis::TopBottom, SplitSize::Half, false).unwrap();
    let s = nested.dump();
    assert!(s.ends_with(",80x24,0,0{40x24,0,0,0,39x24,41,0[39x12,41,0,1,39x11,41,13,2]}"), "{s}");
    assert_eq!(Layout::parse(&s, &[p(0), p(1), p(2)], Size::new(80, 24)).unwrap(), nested);
    assert_eq!(Preset::lookup("main-v"), Err(LayoutError::Invalid("ambiguous layout: main-v".into())));
    assert_eq!(Preset::lookup("ti"), Ok(Preset::Tiled));
    assert_eq!(Preset::Tiled.next(), Preset::EvenHorizontal);
    assert_eq!(Preset::EvenHorizontal.previous(), Preset::Tiled);
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[test]
fn random_operations_keep_invariants() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..300 {
        let mut size = Size::new(rng.below(200) as u16 + 1, rng.below(60) as u16 + 1);
        let mut l = Layout::new(p(0), size);
        let mut panes = vec![p(0)];
        let mut next = 1;
        for step in 0..120 {
            let target = panes[rng.below(panes.len() as u64) as usize];
            match rng.below(12) {
                0..=3 => {
                    let axis = if rng.below(2) == 0 { Axis::LeftRight } else { Axis::TopBottom };
                    let sz = match rng.below(3) {
                        0 => SplitSize::Half,
                        1 => SplitSize::Cells(rng.below(100) as u32),
                        _ => SplitSize::Percent(rng.below(101) as u32),
                    };
                    let before = rng.below(2) == 0;
                    let r = l.split(target, p(next), axis, sz, before);
                    if r.is_ok() {
                        let at = panes.iter().position(|x| *x == target).unwrap();
                        panes.insert(if before { at } else { at + 1 }, p(next));
                        next += 1;
                    } else {
                        assert_eq!(r, Err(LayoutError::NoSpace));
                    }
                }
                4 => {
                    if l.remove(target) {
                        panes.retain(|x| *x != target);
                    } else {
                        assert_eq!(panes.len(), 1);
                    }
                }
                5 => {
                    let dir = [Direction::Left, Direction::Right, Direction::Up, Direction::Down][rng.below(4) as usize];
                    l.resize_pane(target, dir, rng.below(50) as u32);
                }
                6 => {
                    let axis = if rng.below(2) == 0 { Axis::LeftRight } else { Axis::TopBottom };
                    l.resize_pane_to(target, axis, rng.below(100) as u32);
                }
                7 => {
                    size = Size::new(rng.below(250) as u16 + 1, rng.below(80) as u16 + 1);
                    l.resize(size);
                    let min = l.min_size();
                    assert_eq!(l.size(), Size::new(size.cols.max(min.cols), size.rows.max(min.rows)));
                }
                8 => {
                    let preset = Preset::ALL[rng.below(7) as usize];
                    l.apply_preset(preset, &panes, size, &PresetParams::default());
                    if panes.len() > 1 {
                        let min = l.min_size();
                        assert_eq!(l.size(), Size::new(size.cols.max(min.cols), size.rows.max(min.rows)), "{preset:?}");
                    }
                }
                9 => {
                    let other = panes[rng.below(panes.len() as u64) as usize];
                    l.swap(target, other);
                    let (i, j) = (
                        panes.iter().position(|x| *x == target).unwrap(),
                        panes.iter().position(|x| *x == other).unwrap(),
                    );
                    panes.swap(i, j);
                }
                10 => l.spread_out(target),
                _ => {
                    if l.zoomed().is_some() {
                        l.unzoom();
                    } else {
                        l.zoom(target);
                    }
                }
            }
            assert_tiles(&l);
            let mut have = l.panes();
            have.sort();
            let mut want = panes.clone();
            want.sort();
            assert_eq!(have, want, "round {round} step {step}");
            let g = l.geometry();
            let shown = if l.zoomed().is_some() { 1 } else { panes.len() };
            assert_eq!(g.panes.len(), shown);
            let s = l.dump();
            let back = Layout::parse(&s, &l.panes(), l.size()).unwrap();
            assert_eq!(back.unzoomed_geometry(), l.unzoomed_geometry());
        }
    }
}
