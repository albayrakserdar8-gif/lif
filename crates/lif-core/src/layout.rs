//! Pane layouts: a split tree like tmux's.
//!
//! A layout is a tree of cells. A leaf holds one pane; a container splits
//! its area left-right or top-bottom between two or more children, with a
//! one-cell border between neighbours. Every cell stores its own size, as in
//! tmux, so operations (split, close, resize, presets) reproduce tmux's
//! arithmetic exactly. The tree's size is the window size, or larger when
//! the window is too small to hold every pane at the minimum size (the
//! renderer then clips, as tmux does).
//!
//! Invariants (checked by [`Layout::check`] in tests):
//! - a left-right container's children all have the container's height and
//!   their widths plus one border between each pair sum to its width
//!   (top-bottom likewise);
//! - containers have at least two children;
//! - no cell is smaller than [`PANE_MINIMUM`] in either direction.

use crate::ids::PaneId;
use lif_types::{Rect, Size};
use std::fmt;

/// Smallest pane width or height, in cells (tmux 3.8: 1).
pub const PANE_MINIMUM: u32 = 1;

/// Direction a container splits its area in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Children side by side; created by `split-window -h`.
    LeftRight,
    /// Children stacked; created by `split-window -v` (the default).
    TopBottom,
}

/// A direction on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Towards column 0.
    Left,
    /// Towards the last column.
    Right,
    /// Towards row 0.
    Up,
    /// Towards the last row.
    Down,
}

/// Size of the new pane in a split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitSize {
    /// Half of the pane being split (the new pane gets the smaller half).
    Half,
    /// This many cells.
    Cells(u32),
    /// This percentage of the pane being split.
    Percent(u32),
}

/// A size given in cells or as a percentage of what is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeSpec {
    /// A number of cells.
    Cells(u32),
    /// A percentage (0..=100).
    Percent(u32),
}

impl SizeSpec {
    /// Parse `"80"` or `"50%"`.
    pub fn parse(s: &str) -> Option<SizeSpec> {
        match s.strip_suffix('%') {
            Some(p) => p.parse().ok().filter(|p| *p <= 100).map(SizeSpec::Percent),
            None => s.parse().ok().map(SizeSpec::Cells),
        }
    }

    fn resolve(self, of: u32) -> u32 {
        match self {
            SizeSpec::Cells(n) => n,
            SizeSpec::Percent(p) => (of as u64 * p as u64 / 100) as u32,
        }
    }
}

/// The preset layouts, in `next-layout` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// All panes side by side, equal widths.
    EvenHorizontal,
    /// All panes stacked, equal heights.
    EvenVertical,
    /// The first pane on top (`main-pane-height`), the others below.
    MainHorizontal,
    /// The first pane at the bottom, the others above.
    MainHorizontalMirrored,
    /// The first pane on the left (`main-pane-width`), the others right.
    MainVertical,
    /// The first pane on the right, the others left.
    MainVerticalMirrored,
    /// A grid.
    Tiled,
}

impl Preset {
    /// Every preset in the order `next-layout` cycles through them.
    pub const ALL: [Preset; 7] = [
        Preset::EvenHorizontal,
        Preset::EvenVertical,
        Preset::MainHorizontal,
        Preset::MainHorizontalMirrored,
        Preset::MainVertical,
        Preset::MainVerticalMirrored,
        Preset::Tiled,
    ];

    /// The tmux name, e.g. `"main-vertical"`.
    pub fn name(self) -> &'static str {
        match self {
            Preset::EvenHorizontal => "even-horizontal",
            Preset::EvenVertical => "even-vertical",
            Preset::MainHorizontal => "main-horizontal",
            Preset::MainHorizontalMirrored => "main-horizontal-mirrored",
            Preset::MainVertical => "main-vertical",
            Preset::MainVerticalMirrored => "main-vertical-mirrored",
            Preset::Tiled => "tiled",
        }
    }

    /// Look a preset up by exact name or unique prefix, as tmux does.
    pub fn lookup(name: &str) -> Result<Preset, LayoutError> {
        if let Some(p) = Preset::ALL.iter().find(|p| p.name() == name) {
            return Ok(*p);
        }
        let mut found = None;
        for p in Preset::ALL {
            if !name.is_empty() && p.name().starts_with(name) {
                if found.is_some() {
                    return Err(LayoutError::Invalid(format!("ambiguous layout: {name}")));
                }
                found = Some(p);
            }
        }
        found.ok_or_else(|| LayoutError::Invalid(format!("unknown layout: {name}")))
    }

    fn index(self) -> usize {
        Preset::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The preset after this one (wrapping).
    pub fn next(self) -> Preset {
        Preset::ALL[(self.index() + 1) % Preset::ALL.len()]
    }

    /// The preset before this one (wrapping).
    pub fn previous(self) -> Preset {
        Preset::ALL[(self.index() + Preset::ALL.len() - 1) % Preset::ALL.len()]
    }
}

/// Window options the presets read (`main-pane-width` etc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresetParams {
    /// `main-pane-width` (percentages are of the window width less one).
    pub main_pane_width: SizeSpec,
    /// `main-pane-height`.
    pub main_pane_height: SizeSpec,
    /// `other-pane-width`; `Cells(0)` means "whatever the main pane leaves".
    pub other_pane_width: SizeSpec,
    /// `other-pane-height`; `Cells(0)` as above.
    pub other_pane_height: SizeSpec,
    /// 0 means unlimited.
    pub tiled_max_columns: u32,
}

impl Default for PresetParams {
    /// tmux's defaults: main pane 80 wide / 24 high.
    fn default() -> Self {
        Self {
            main_pane_width: SizeSpec::Cells(80),
            main_pane_height: SizeSpec::Cells(24),
            other_pane_width: SizeSpec::Cells(0),
            other_pane_height: SizeSpec::Cells(0),
            tiled_max_columns: 0,
        }
    }
}

/// Why a layout operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// The pane is too small to split (tmux: "no space for a new pane").
    NoSpace,
    /// The pane is not in this layout.
    NoSuchPane,
    /// A layout string or name could not be used; the text is the message.
    Invalid(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::NoSpace => f.write_str("no space for a new pane"),
            LayoutError::NoSuchPane => f.write_str("pane not in layout"),
            LayoutError::Invalid(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Where everything goes on screen, relative to the window's top-left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    /// Every visible pane and its rectangle. When the window is zoomed this
    /// is only the zoomed pane, covering the whole window.
    pub panes: Vec<(PaneId, Rect)>,
    /// The border lines between panes, one cell thick.
    pub borders: Vec<Rect>,
}

impl Geometry {
    /// The rectangle of `pane`, if it is visible.
    pub fn rect(&self, pane: PaneId) -> Option<Rect> {
        self.panes.iter().find(|(p, _)| *p == pane).map(|(_, r)| *r)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    sx: u32,
    sy: u32,
    node: Node,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Pane(PaneId),
    Split(Axis, Vec<Cell>),
}

impl Cell {
    fn leaf(pane: PaneId, sx: u32, sy: u32) -> Cell {
        Cell { sx, sy, node: Node::Pane(pane) }
    }

    fn along(&self, axis: Axis) -> u32 {
        match axis {
            Axis::LeftRight => self.sx,
            Axis::TopBottom => self.sy,
        }
    }

    fn set_along(&mut self, axis: Axis, v: u32) {
        match axis {
            Axis::LeftRight => self.sx = v,
            Axis::TopBottom => self.sy = v,
        }
    }

    /// Smallest size this cell can have along `axis`.
    fn min_along(&self, axis: Axis) -> u32 {
        match &self.node {
            Node::Pane(_) => PANE_MINIMUM,
            Node::Split(a, ch) if *a == axis => {
                ch.iter().map(|c| c.min_along(axis)).sum::<u32>() + ch.len() as u32 - 1
            }
            Node::Split(_, ch) => ch.iter().map(|c| c.min_along(axis)).max().unwrap_or(PANE_MINIMUM),
        }
    }

    /// How much this cell can shrink along `axis` (tmux `layout_resize_check`).
    fn available(&self, axis: Axis) -> u32 {
        self.along(axis).saturating_sub(self.min_along(axis))
    }

    /// tmux's `layout_resize_adjust`: change the size by `change`, spreading
    /// it one cell at a time over same-direction children.
    fn adjust(&mut self, axis: Axis, change: i64) {
        let v = (self.along(axis) as i64 + change).max(0) as u32;
        self.set_along(axis, v);
        match &mut self.node {
            Node::Pane(_) => {}
            Node::Split(a, ch) if *a != axis => {
                for c in ch {
                    c.adjust(axis, change);
                }
            }
            Node::Split(_, ch) => {
                let mut change = change;
                while change != 0 {
                    let mut progressed = false;
                    for c in ch.iter_mut() {
                        if change == 0 {
                            break;
                        }
                        if change > 0 {
                            c.adjust(axis, 1);
                            change -= 1;
                            progressed = true;
                        } else if c.available(axis) > 0 {
                            c.adjust(axis, -1);
                            change += 1;
                            progressed = true;
                        }
                    }
                    if !progressed {
                        debug_assert!(false, "adjust past minimum");
                        break;
                    }
                }
            }
        }
    }

    /// Proportional re-fit to `new` cells along `axis` (`new` must be at least
    /// `min_along`). Each child keeps its share of the space; children that
    /// would drop below their minimum take the shortfall from the children
    /// with the most room.
    fn fit(&mut self, axis: Axis, new: u32) {
        self.set_along(axis, new);
        match &mut self.node {
            Node::Pane(_) => {}
            Node::Split(a, ch) if *a != axis => {
                for c in ch {
                    c.fit(axis, new);
                }
            }
            Node::Split(_, ch) => {
                let n = ch.len() as u32;
                let content_new = new.saturating_sub(n - 1) as u64;
                let content_old: u64 = ch.iter().map(|c| c.along(axis) as u64).sum::<u64>().max(1);
                let mut t: Vec<u32> = Vec::with_capacity(ch.len());
                let mut frac: Vec<(u64, usize)> = Vec::with_capacity(ch.len());
                for (i, c) in ch.iter().enumerate() {
                    let num = c.along(axis) as u64 * content_new;
                    t.push((num / content_old) as u32);
                    frac.push((num % content_old, i));
                }
                let mut rem = content_new as u32 - t.iter().sum::<u32>();
                frac.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
                for (_, i) in frac {
                    if rem == 0 {
                        break;
                    }
                    t[i] += 1;
                    rem -= 1;
                }
                let mins: Vec<u32> = ch.iter().map(|c| c.min_along(axis)).collect();
                let mut deficit = 0u32;
                for i in 0..t.len() {
                    if t[i] < mins[i] {
                        deficit += mins[i] - t[i];
                        t[i] = mins[i];
                    }
                }
                while deficit > 0 {
                    let mut best = None;
                    let mut best_slack = 0;
                    for i in 0..t.len() {
                        let slack = t[i] - mins[i];
                        if slack > best_slack {
                            best_slack = slack;
                            best = Some(i);
                        }
                    }
                    let Some(i) = best else { break };
                    let take = deficit.min(best_slack.div_ceil(2).max(1));
                    t[i] -= take;
                    deficit -= take;
                }
                for (c, v) in ch.iter_mut().zip(t) {
                    c.fit(axis, v);
                }
            }
        }
    }

    fn path_to(&self, pane: PaneId, path: &mut Vec<usize>) -> bool {
        match &self.node {
            Node::Pane(p) => *p == pane,
            Node::Split(_, ch) => {
                for (i, c) in ch.iter().enumerate() {
                    path.push(i);
                    if c.path_to(pane, path) {
                        return true;
                    }
                    path.pop();
                }
                false
            }
        }
    }

    fn at(&self, path: &[usize]) -> &Cell {
        let mut c = self;
        for &i in path {
            match &c.node {
                Node::Split(_, ch) => c = &ch[i],
                Node::Pane(_) => unreachable!("path through a leaf"),
            }
        }
        c
    }

    fn at_mut(&mut self, path: &[usize]) -> &mut Cell {
        let mut c = self;
        for &i in path {
            match &mut c.node {
                Node::Split(_, ch) => c = &mut ch[i],
                Node::Pane(_) => unreachable!("path through a leaf"),
            }
        }
        c
    }

    fn leaves(&self, out: &mut Vec<PaneId>) {
        match &self.node {
            Node::Pane(p) => out.push(*p),
            Node::Split(_, ch) => ch.iter().for_each(|c| c.leaves(out)),
        }
    }

    fn map_panes(&mut self, f: &mut impl FnMut(PaneId) -> PaneId) {
        match &mut self.node {
            Node::Pane(p) => *p = f(*p),
            Node::Split(_, ch) => ch.iter_mut().for_each(|c| c.map_panes(f)),
        }
    }

    fn walk(&self, x: u32, y: u32, g: &mut Geometry) {
        match &self.node {
            Node::Pane(p) => g.panes.push((*p, rect(x, y, self.sx, self.sy))),
            Node::Split(axis, ch) => {
                let (mut cx, mut cy) = (x, y);
                for (i, c) in ch.iter().enumerate() {
                    c.walk(cx, cy, g);
                    let last = i + 1 == ch.len();
                    match axis {
                        Axis::LeftRight => {
                            cx += c.sx;
                            if !last {
                                g.borders.push(rect(cx, y, 1, self.sy));
                                cx += 1;
                            }
                        }
                        Axis::TopBottom => {
                            cy += c.sy;
                            if !last {
                                g.borders.push(rect(x, cy, self.sx, 1));
                                cy += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    fn dump(&self, x: u32, y: u32, out: &mut String) {
        use std::fmt::Write;
        let _ = write!(out, "{}x{},{},{}", self.sx, self.sy, x, y);
        match &self.node {
            Node::Pane(p) => {
                let _ = write!(out, ",{}", p.0);
            }
            Node::Split(axis, ch) => {
                let (open, close) = match axis {
                    Axis::LeftRight => ('{', '}'),
                    Axis::TopBottom => ('[', ']'),
                };
                out.push(open);
                let (mut cx, mut cy) = (x, y);
                for (i, c) in ch.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    c.dump(cx, cy, out);
                    match axis {
                        Axis::LeftRight => cx += c.sx + 1,
                        Axis::TopBottom => cy += c.sy + 1,
                    }
                }
                out.push(close);
            }
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.sx < PANE_MINIMUM || self.sy < PANE_MINIMUM {
            return Err(format!("cell {}x{} below minimum", self.sx, self.sy));
        }
        if let Node::Split(axis, ch) = &self.node {
            if ch.len() < 2 {
                return Err("container with fewer than two children".into());
            }
            // u64: a parsed layout string can list many large cells.
            let mut total = 0u64;
            for c in ch {
                c.check()?;
                match axis {
                    Axis::LeftRight => {
                        if c.sy != self.sy {
                            return Err(format!("child height {} != {}", c.sy, self.sy));
                        }
                        total += c.sx as u64;
                    }
                    Axis::TopBottom => {
                        if c.sx != self.sx {
                            return Err(format!("child width {} != {}", c.sx, self.sx));
                        }
                        total += c.sy as u64;
                    }
                }
            }
            total += ch.len() as u64 - 1;
            if total != self.along(*axis) as u64 {
                return Err(format!("children sum {total} != {}", self.along(*axis)));
            }
        }
        Ok(())
    }
}

fn clamp16(v: u32) -> u16 {
    v.min(u16::MAX as u32) as u16
}

fn rect(x: u32, y: u32, sx: u32, sy: u32) -> Rect {
    Rect::new(clamp16(x), clamp16(y), clamp16(sx), clamp16(sy))
}

/// The layout of one window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    root: Cell,
    zoomed: Option<PaneId>,
}

impl Layout {
    /// A layout holding one pane that fills `size` (at least 1x1).
    pub fn new(pane: PaneId, size: Size) -> Layout {
        let sx = (size.cols as u32).max(PANE_MINIMUM);
        let sy = (size.rows as u32).max(PANE_MINIMUM);
        Layout { root: Cell::leaf(pane, sx, sy), zoomed: None }
    }

    /// The size of the tree: the window size, or more when the window is
    /// too small for the panes.
    pub fn size(&self) -> Size {
        Size::new(clamp16(self.root.sx), clamp16(self.root.sy))
    }

    /// The smallest size the tree can be shrunk to.
    pub fn min_size(&self) -> Size {
        Size::new(clamp16(self.root.min_along(Axis::LeftRight)), clamp16(self.root.min_along(Axis::TopBottom)))
    }

    /// Panes in tree order (top-left first).
    pub fn panes(&self) -> Vec<PaneId> {
        let mut v = Vec::new();
        self.root.leaves(&mut v);
        v
    }

    /// Number of panes.
    pub fn pane_count(&self) -> usize {
        self.panes().len()
    }

    /// Whether `pane` is in this layout.
    pub fn contains(&self, pane: PaneId) -> bool {
        self.path(pane).is_some()
    }

    fn path(&self, pane: PaneId) -> Option<Vec<usize>> {
        let mut p = Vec::new();
        self.root.path_to(pane, &mut p).then_some(p)
    }

    /// The zoomed pane, if any.
    pub fn zoomed(&self) -> Option<PaneId> {
        self.zoomed
    }

    /// Zoom `pane` (it alone fills the window). Fails, like tmux, when the
    /// layout has a single pane or the pane is not in it.
    pub fn zoom(&mut self, pane: PaneId) -> bool {
        if !self.contains(pane) || self.pane_count() < 2 {
            return false;
        }
        self.zoomed = Some(pane);
        true
    }

    /// Undo [`Layout::zoom`]. Returns whether the layout was zoomed.
    pub fn unzoom(&mut self) -> bool {
        self.zoomed.take().is_some()
    }

    /// Split `target`, putting `new` after it (right / below) or, with
    /// `before`, in front of it. tmux's `layout_split_pane`: the new pane gets
    /// `size` cells (clamped so both panes keep at least one cell).
    pub fn split(
        &mut self,
        target: PaneId,
        new: PaneId,
        axis: Axis,
        size: SplitSize,
        before: bool,
    ) -> Result<(), LayoutError> {
        let path = self.path(target).ok_or(LayoutError::NoSuchPane)?;
        let cell = self.root.at(&path);
        let (sx, sy) = (cell.sx, cell.sy);
        let saved = cell.along(axis);
        if saved < PANE_MINIMUM * 2 + 1 {
            return Err(LayoutError::NoSpace);
        }
        let want = match size {
            SplitSize::Half => None,
            SplitSize::Cells(n) => Some(n as i64),
            SplitSize::Percent(p) => Some(saved as i64 * p as i64 / 100),
        };
        let mut size2: i64 = match want {
            None => (saved as i64 + 1) / 2 - 1,
            Some(s) if before => saved as i64 - s - 1,
            Some(s) => s,
        };
        if size2 < PANE_MINIMUM as i64 {
            size2 = PANE_MINIMUM as i64;
        } else if size2 > saved as i64 - 2 {
            size2 = saved as i64 - 2;
        }
        let size2 = size2 as u32;
        let size1 = saved - 1 - size2;
        let (new_size, old_size) = if before { (size1, size2) } else { (size2, size1) };
        let mut new_cell = Cell::leaf(new, sx, sy);
        new_cell.set_along(axis, new_size);
        self.zoomed = None;

        let parent_same = match path.split_last() {
            Some((_, parent)) => matches!(&self.root.at(parent).node, Node::Split(a, _) if *a == axis),
            None => false,
        };
        if parent_same {
            let (&idx, parent) = path.split_last().expect("has parent");
            let p = self.root.at_mut(parent);
            if let Node::Split(_, ch) = &mut p.node {
                ch[idx].set_along(axis, old_size);
                ch.insert(if before { idx } else { idx + 1 }, new_cell);
            }
        } else {
            let cell = self.root.at_mut(&path);
            let mut old = cell.clone();
            old.set_along(axis, old_size);
            let children = if before { vec![new_cell, old] } else { vec![old, new_cell] };
            *cell = Cell { sx, sy, node: Node::Split(axis, children) };
        }
        Ok(())
    }

    /// Put `new` beside everything else, across the whole window
    /// (`split-window -f`): below or right of the rest, or with `before`
    /// above or left of it.
    pub fn split_full(&mut self, new: PaneId, axis: Axis, size: SplitSize, before: bool) -> Result<(), LayoutError> {
        let (sx, sy) = (self.root.sx, self.root.sy);
        let saved = self.root.along(axis);
        let least = self.root.min_along(axis);
        if saved < least + 1 + PANE_MINIMUM {
            return Err(LayoutError::NoSpace);
        }
        let new_size = match size {
            SplitSize::Half => saved.div_ceil(2) - 1,
            SplitSize::Cells(n) => n,
            SplitSize::Percent(p) => (saved as u64 * p as u64 / 100) as u32,
        }
        .clamp(PANE_MINIMUM, saved - 1 - least);
        let mut new_cell = Cell::leaf(new, sx, sy);
        new_cell.set_along(axis, new_size);
        self.zoomed = None;
        let mut old = std::mem::replace(&mut self.root, Cell::leaf(new, sx, sy));
        old.fit(axis, saved - 1 - new_size);
        let children = match old.node {
            // A root already split this way takes the pane as one more
            // child, so the tree stays as tmux would have it.
            Node::Split(a, mut ch) if a == axis => {
                ch.insert(if before { 0 } else { ch.len() }, new_cell);
                ch
            }
            node => {
                let old = Cell { sx: old.sx, sy: old.sy, node };
                if before { vec![new_cell, old] } else { vec![old, new_cell] }
            }
        };
        self.root = Cell { sx, sy, node: Node::Split(axis, children) };
        Ok(())
    }

    /// Remove `pane`; its space goes to the next sibling (the previous one if
    /// it was last), as tmux 3.8 does. Returns false if the pane is the only
    /// one (the caller removes the window instead) or not in the layout.
    pub fn remove(&mut self, pane: PaneId) -> bool {
        let Some(path) = self.path(pane) else { return false };
        let Some((&idx, parent_path)) = path.split_last() else { return false };
        if self.zoomed.is_some() {
            self.zoomed = None;
        }
        let parent = self.root.at_mut(parent_path);
        let Node::Split(axis, ch) = &mut parent.node else { return false };
        let axis = *axis;
        // tmux 3.8 gives the space to the next sibling, or the previous one
        // when the pane was last (verified against tmux 3.8).
        let other = if idx + 1 < ch.len() { idx + 1 } else { idx - 1 };
        let freed = ch[idx].along(axis) as i64 + 1;
        ch[other].adjust(axis, freed);
        ch.remove(idx);
        if ch.len() == 1 {
            let only = ch.pop().expect("one child");
            *parent = only;
        }
        true
    }

    /// Move the border next to `pane` by `amount` cells in `dir` (tmux
    /// `resize-pane -L/-R/-U/-D`). Moves as far as the minimum sizes allow and
    /// returns how far that was; never fails, like tmux.
    pub fn resize_pane(&mut self, pane: PaneId, dir: Direction, amount: u32) -> u32 {
        let (axis, change) = match dir {
            Direction::Left => (Axis::LeftRight, -(amount as i64)),
            Direction::Right => (Axis::LeftRight, amount as i64),
            Direction::Up => (Axis::TopBottom, -(amount as i64)),
            Direction::Down => (Axis::TopBottom, amount as i64),
        };
        self.resize_signed(pane, axis, change)
    }

    /// Make `pane` (or the ancestor that sits in a container of the right
    /// direction) `size` cells wide or high (tmux `resize-pane -x/-y`).
    pub fn resize_pane_to(&mut self, pane: PaneId, axis: Axis, size: u32) -> u32 {
        let Some((parent_path, idx)) = self.resizable(pane, axis) else { return 0 };
        let Node::Split(_, ch) = &self.root.at(&parent_path).node else { return 0 };
        let cur = ch[idx].along(axis) as i64;
        let change = if idx + 1 == ch.len() { cur - size as i64 } else { size as i64 - cur };
        self.resize_signed(pane, axis, change)
    }

    /// The container of direction `axis` closest to `pane` and the index of
    /// the child on the path to `pane`.
    fn resizable(&self, pane: PaneId, axis: Axis) -> Option<(Vec<usize>, usize)> {
        let mut path = self.path(pane)?;
        loop {
            let (&idx, parent) = path.split_last()?;
            if matches!(&self.root.at(parent).node, Node::Split(a, _) if *a == axis) {
                return Some((parent.to_vec(), idx));
            }
            path.pop();
        }
    }

    fn resize_signed(&mut self, pane: PaneId, axis: Axis, change: i64) -> u32 {
        let Some((parent_path, mut idx)) = self.resizable(pane, axis) else { return 0 };
        let parent = self.root.at_mut(&parent_path);
        let Node::Split(_, ch) = &mut parent.node else { return 0 };
        if idx + 1 == ch.len() {
            idx -= 1;
        }
        let mut needed = change;
        let mut moved = 0u32;
        while needed != 0 {
            let size = if change > 0 { grow(ch, idx, axis, needed as u32) } else { shrink(ch, idx, axis, (-needed) as u32) };
            if size == 0 {
                break;
            }
            moved += size;
            if change > 0 {
                needed -= size as i64;
            } else {
                needed += size as i64;
            }
        }
        moved
    }

    /// The pane next to `pane` in `dir` for `select-pane -L/-R/-U/-D`, with
    /// tmux's wrap-around at the window edges. `order` is the window's pane
    /// list (ties go to the earliest); `recency` gives each pane's last-active
    /// counter (the most recently active candidate wins). Uses the unzoomed
    /// layout.
    pub fn neighbor(&self, pane: PaneId, dir: Direction, order: &[PaneId], recency: impl Fn(PaneId) -> u64) -> Option<PaneId> {
        let g = self.unzoomed_geometry();
        let r = g.rect(pane)?;
        let (wx, wy) = (self.root.sx as i64, self.root.sy as i64);
        let (x, y, sx, sy) = (r.x as i64, r.y as i64, r.cols as i64, r.rows as i64);
        let mut best: Option<(PaneId, u64)> = None;
        for &p in order {
            if p == pane {
                continue;
            }
            let Some(n) = g.rect(p) else { continue };
            let (nx, ny, nsx, nsy) = (n.x as i64, n.y as i64, n.cols as i64, n.rows as i64);
            let ok = match dir {
                Direction::Left | Direction::Right => {
                    let edge_ok = if dir == Direction::Left {
                        let edge = if x == 0 { wx + 1 } else { x };
                        nx + nsx + 1 == edge
                    } else {
                        let mut edge = x + sx + 1;
                        if edge >= wx {
                            edge = 0;
                        }
                        nx == edge
                    };
                    let (top, bottom, end) = (y, y + sy, ny + nsy - 1);
                    edge_ok
                        && ((ny < top && end > bottom) || (ny >= top && ny <= bottom) || (end >= top && end <= bottom))
                }
                Direction::Up | Direction::Down => {
                    let edge_ok = if dir == Direction::Up {
                        let edge = if y == 0 { wy + 1 } else { y };
                        ny + nsy + 1 == edge
                    } else {
                        let mut edge = y + sy + 1;
                        if edge >= wy {
                            edge = 0;
                        }
                        ny == edge
                    };
                    let (left, right, end) = (x, x + sx, nx + nsx - 1);
                    edge_ok
                        && ((nx < left && end > right) || (nx >= left && nx <= right) || (end >= left && end <= right))
                }
            };
            if ok {
                let rec = recency(p);
                if best.is_none_or(|(_, b)| rec > b) {
                    best = Some((p, rec));
                }
            }
        }
        best.map(|(p, _)| p)
    }

    /// The pane at a named position (`top`, `bottom-left`, ...) for targets
    /// like `{top-left}`.
    pub fn pane_at_position(&self, name: &str, order: &[PaneId]) -> Option<PaneId> {
        // tmux's window_find_string: the unnamed coordinate is the middle,
        // and a pane's right/bottom border counts as part of it.
        let g = self.unzoomed_geometry();
        let (w, h) = (self.root.sx, self.root.sy);
        let (mut x, mut y) = (w / 2, h / 2);
        match name {
            "top" => y = 0,
            "bottom" => y = h - 1,
            "left" => x = 0,
            "right" => x = w - 1,
            "top-left" => (x, y) = (0, 0),
            "top-right" => (x, y) = (w - 1, 0),
            "bottom-left" => (x, y) = (0, h - 1),
            "bottom-right" => (x, y) = (w - 1, h - 1),
            _ => return None,
        }
        order.iter().copied().find(|p| {
            g.rect(*p).is_some_and(|r| {
                let (rx, ry) = (r.x as u32, r.y as u32);
                x >= rx && x <= rx + r.cols as u32 && y >= ry && y <= ry + r.rows as u32
            })
        })
    }

    /// Exchange the positions of two panes.
    pub fn swap(&mut self, a: PaneId, b: PaneId) {
        self.root.map_panes(&mut |p| {
            if p == a {
                b
            } else if p == b {
                a
            } else {
                p
            }
        });
        if let Some(z) = self.zoomed {
            self.zoomed = Some(if z == a { b } else if z == b { a } else { z });
        }
    }

    /// Put `new` where `old` is (`old` leaves the layout).
    pub fn replace(&mut self, old: PaneId, new: PaneId) {
        self.root.map_panes(&mut |p| if p == old { new } else { p });
        if self.zoomed == Some(old) {
            self.zoomed = Some(new);
        }
    }

    /// Reassign panes to cells: the cell holding `from[i]` gets `to[i]`
    /// (used by `rotate-window`).
    pub fn reassign(&mut self, from: &[PaneId], to: &[PaneId]) {
        self.root.map_panes(&mut |p| match from.iter().position(|x| *x == p) {
            Some(i) => to[i],
            None => p,
        });
    }

    /// Replace the tree with a preset arrangement of `panes` (in window
    /// order) for a window of `size`, as tmux's `layout_set_*` functions do.
    /// Nothing happens with fewer than two panes.
    pub fn apply_preset(&mut self, preset: Preset, panes: &[PaneId], size: Size, params: &PresetParams) {
        if panes.len() <= 1 {
            return;
        }
        let (w, h) = (size.cols as u32, size.rows as u32);
        self.zoomed = None;
        self.root = match preset {
            Preset::EvenHorizontal => even(panes, Axis::LeftRight, w, h),
            Preset::EvenVertical => even(panes, Axis::TopBottom, w, h),
            Preset::MainVertical => main(panes, Axis::LeftRight, w, h, params, false),
            Preset::MainVerticalMirrored => main(panes, Axis::LeftRight, w, h, params, true),
            Preset::MainHorizontal => main(panes, Axis::TopBottom, w, h, params, false),
            Preset::MainHorizontalMirrored => main(panes, Axis::TopBottom, w, h, params, true),
            Preset::Tiled => tiled(panes, w, h, params.tiled_max_columns),
        };
        self.resize(size);
    }

    /// `select-layout -E`: spread the panes of the nearest container (going up
    /// from `pane`) whose children are not already even.
    pub fn spread_out(&mut self, pane: PaneId) {
        let Some(mut path) = self.path(pane) else { return };
        while path.pop().is_some() {
            if spread(self.root.at_mut(&path)) {
                return;
            }
        }
    }

    /// Fit the layout to a new window size: proportional in each container,
    /// never below the minimum (the tree stays larger than the window then).
    ///
    /// tmux 3.8 instead adds or removes one cell per child in turn (equal
    /// steps, verified: 28/27/24 columns at 81 wide become 41/40/37 at 120);
    /// lif keeps each pane's share so a shrink and grow round-trips.
    pub fn resize(&mut self, size: Size) {
        let w = (size.cols as u32).max(self.root.min_along(Axis::LeftRight));
        let h = (size.rows as u32).max(self.root.min_along(Axis::TopBottom));
        if w != self.root.sx {
            self.root.fit(Axis::LeftRight, w);
        }
        if h != self.root.sy {
            self.root.fit(Axis::TopBottom, h);
        }
    }

    /// Pane rectangles and borders for the current size, honouring zoom.
    pub fn geometry(&self) -> Geometry {
        match self.zoomed {
            Some(p) => Geometry { panes: vec![(p, rect(0, 0, self.root.sx, self.root.sy))], borders: Vec::new() },
            None => self.unzoomed_geometry(),
        }
    }

    /// Pane rectangles and borders ignoring zoom.
    pub fn unzoomed_geometry(&self) -> Geometry {
        let mut g = Geometry::default();
        self.root.walk(0, 0, &mut g);
        g
    }

    /// The geometry the layout would have in a window of `size` (after a
    /// proportional re-fit), without changing the layout.
    pub fn compute(&self, size: Size) -> Geometry {
        if size == self.size() {
            return self.geometry();
        }
        let mut l = self.clone();
        l.resize(size);
        l.geometry()
    }

    /// The tmux layout string (`#{window_layout}`, accepted by
    /// `select-layout`): `"bb62,159x48,0,0{79x48,0,0,1,79x48,80,0,2}"`.
    pub fn dump(&self) -> String {
        let mut body = String::new();
        self.root.dump(0, 0, &mut body);
        format!("{:04x},{}", checksum(&body), body)
    }

    /// Parse a tmux layout string and put `panes` (window order) into its
    /// cells in tree order. The pane ids in the string are ignored, as in
    /// tmux. The result is fitted to `size`.
    pub fn parse(s: &str, panes: &[PaneId], size: Size) -> Result<Layout, LayoutError> {
        let bad = || LayoutError::Invalid(format!("invalid layout: {s}"));
        let (csum, body) = s.split_once(',').ok_or_else(bad)?;
        if csum.len() != 4 || u16::from_str_radix(csum, 16).ok() != Some(checksum(body)) {
            return Err(bad());
        }
        let mut p = Parser { s: body.as_bytes(), i: 0, depth: 0 };
        let mut root = p.cell().ok_or_else(bad)?;
        if p.i != p.s.len() {
            return Err(bad());
        }
        let mut leaves = Vec::new();
        root.leaves(&mut leaves);
        if leaves.len() != panes.len() {
            return Err(LayoutError::Invalid(format!("have {} panes but need {}", panes.len(), leaves.len())));
        }
        root.check().map_err(|_| bad())?;
        let mut it = panes.iter();
        root.map_panes(&mut |_| *it.next().expect("count checked"));
        let mut l = Layout { root, zoomed: None };
        l.resize(size);
        Ok(l)
    }

    /// Check the structural invariants (for tests and debug assertions).
    pub fn check(&self) -> Result<(), String> {
        self.root.check()
    }
}

fn checksum(s: &str) -> u16 {
    let mut c: u16 = 0;
    for b in s.bytes() {
        c = (c >> 1) | ((c & 1) << 15);
        c = c.wrapping_add(b as u16);
    }
    c
}

/// How deeply a layout string's `{...}` / `[...]` cells may nest. The parser
/// recurses once per level, so without a limit a long string of `{` given to
/// `select-layout` overflows the stack. A window of the largest size holds
/// far fewer nested splits than this.
const MAX_LAYOUT_DEPTH: usize = 1000;

/// Largest cell size a layout string may give: what fits in a window
/// (sizes are u16), so the size arithmetic on parsed trees cannot overflow.
const MAX_LAYOUT_CELLS: u32 = u16::MAX as u32;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn num(&mut self) -> Option<u32> {
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok().filter(|n| *n <= MAX_LAYOUT_CELLS)
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn cell(&mut self) -> Option<Cell> {
        let sx = self.num()?;
        self.eat(b'x').then_some(())?;
        let sy = self.num()?;
        self.eat(b',').then_some(())?;
        self.num()?;
        self.eat(b',').then_some(())?;
        self.num()?;
        let close = match self.s.get(self.i) {
            Some(b'{') => b'}',
            Some(b'[') => b']',
            Some(b',') => {
                // ",id" of a leaf, unless the next thing is a sibling cell
                // ("WxH,..."), which only a container's child list has.
                let save = self.i;
                self.i += 1;
                let id = self.num();
                if id.is_some() && !matches!(self.s.get(self.i), Some(b'x')) {
                    return Some(Cell::leaf(PaneId(id.unwrap_or(0)), sx, sy));
                }
                self.i = save;
                return Some(Cell::leaf(PaneId(0), sx, sy));
            }
            _ => return Some(Cell::leaf(PaneId(0), sx, sy)),
        };
        let axis = if close == b'}' { Axis::LeftRight } else { Axis::TopBottom };
        if self.depth >= MAX_LAYOUT_DEPTH {
            return None;
        }
        self.i += 1;
        self.depth += 1;
        let mut ch = vec![self.cell()?];
        while self.eat(b',') {
            ch.push(self.cell()?);
        }
        self.depth -= 1;
        self.eat(close).then_some(())?;
        if ch.len() == 1 {
            return ch.pop();
        }
        Some(Cell { sx, sy, node: Node::Split(axis, ch) })
    }
}

fn grow(ch: &mut [Cell], idx: usize, axis: Axis, needed: u32) -> u32 {
    let mut remove = None;
    for (j, c) in ch.iter().enumerate().skip(idx + 1) {
        let a = c.available(axis);
        if a > 0 {
            remove = Some((j, a));
            break;
        }
    }
    if remove.is_none() {
        for j in (0..idx).rev() {
            let a = ch[j].available(axis);
            if a > 0 {
                remove = Some((j, a));
                break;
            }
        }
    }
    let Some((j, avail)) = remove else { return 0 };
    let size = avail.min(needed);
    ch[idx].adjust(axis, size as i64);
    ch[j].adjust(axis, -(size as i64));
    size
}

fn shrink(ch: &mut [Cell], idx: usize, axis: Axis, needed: u32) -> u32 {
    let mut remove = None;
    for j in (0..=idx).rev() {
        let a = ch[j].available(axis);
        if a > 0 {
            remove = Some((j, a));
            break;
        }
    }
    let Some((j, avail)) = remove else { return 0 };
    if idx + 1 >= ch.len() {
        return 0;
    }
    let size = avail.min(needed);
    ch[idx + 1].adjust(axis, size as i64);
    ch[j].adjust(axis, -(size as i64));
    size
}

/// tmux 3.8's `layout_spread_cell`: equal sizes, the remainder going to the
/// first children. Returns whether anything changed.
fn spread(cell: &mut Cell) -> bool {
    let Node::Split(axis, ch) = &mut cell.node else { return false };
    let axis = *axis;
    let n = ch.len() as u32;
    let size = match axis {
        Axis::LeftRight => cell.sx,
        Axis::TopBottom => cell.sy,
    };
    if n <= 1 || size < n - 1 {
        return false;
    }
    let content = size - (n - 1);
    let each = content / n;
    if each == 0 {
        return false;
    }
    // Already as even as it can be: tmux 3.8 leaves it alone (so `-E` moves
    // on to the parent) rather than moving the odd cell around.
    let sum: u32 = ch.iter().map(|c| c.along(axis)).sum();
    let min = ch.iter().map(|c| c.along(axis)).min().unwrap_or(0);
    let max = ch.iter().map(|c| c.along(axis)).max().unwrap_or(0);
    if sum == content && max - min <= 1 {
        return false;
    }
    if ch.iter().any(|c| c.min_along(axis) > each) {
        return false;
    }
    let mut rem = content % n;
    let mut changed = false;
    for c in ch.iter_mut() {
        let mut target = each;
        if rem > 0 {
            target += 1;
            rem -= 1;
        }
        let change = target as i64 - c.along(axis) as i64;
        if change != 0 {
            // Children may be below their own minimum along a fresh
            // container; set leaves directly, adjust containers.
            c.adjust(axis, change);
            changed = true;
        }
    }
    changed
}

fn even(panes: &[PaneId], axis: Axis, w: u32, h: u32) -> Cell {
    let n = panes.len() as u32;
    let need = n * (PANE_MINIMUM + 1) - 1;
    let (sx, sy) = match axis {
        Axis::LeftRight => (need.max(w), h.max(PANE_MINIMUM)),
        Axis::TopBottom => (w.max(PANE_MINIMUM), need.max(h)),
    };
    let leaves = panes.iter().map(|p| Cell::leaf(*p, sx, sy)).collect();
    let mut root = Cell { sx, sy, node: Node::Split(axis, leaves) };
    spread(&mut root);
    root
}

fn main(panes: &[PaneId], axis: Axis, w: u32, h: u32, params: &PresetParams, mirrored: bool) -> Cell {
    let others = panes.len() as u32 - 1;
    // "along" is the axis the main pane and the others share.
    let (total, cross, main_spec, other_spec) = match axis {
        Axis::LeftRight => (w, h, params.main_pane_width, params.other_pane_width),
        Axis::TopBottom => (h, w, params.main_pane_height, params.other_pane_height),
    };
    let avail = total.saturating_sub(1);
    let mut mainw = main_spec.resolve(avail);
    let otherw_opt = other_spec.resolve(avail);
    if otherw_opt > 0 && otherw_opt < avail {
        mainw = avail - otherw_opt;
    }
    // Not `mainw + 1 > avail`: main-pane-width can be any u32.
    if mainw >= avail {
        mainw = avail.saturating_sub(1);
    }
    let mainw = mainw.max(PANE_MINIMUM);
    let otherw = avail.saturating_sub(mainw).max(PANE_MINIMUM);
    let cross = cross.max(others * (PANE_MINIMUM + 1) - 1);
    let other_axis = match axis {
        Axis::LeftRight => Axis::TopBottom,
        Axis::TopBottom => Axis::LeftRight,
    };
    let mk = |along: u32, node: Node| {
        let mut c = Cell { sx: 0, sy: 0, node };
        c.set_along(axis, along);
        c.set_along(other_axis, cross);
        c
    };
    let main_cell = mk(mainw, Node::Pane(panes[0]));
    let other_cell = if others == 1 {
        mk(otherw, Node::Pane(panes[1]))
    } else {
        let kids = panes[1..]
            .iter()
            .map(|p| {
                let mut c = Cell::leaf(*p, 0, 0);
                c.set_along(axis, otherw);
                c.set_along(other_axis, PANE_MINIMUM);
                c
            })
            .collect();
        let mut c = mk(otherw, Node::Split(other_axis, kids));
        spread(&mut c);
        c
    };
    let children = if mirrored { vec![other_cell, main_cell] } else { vec![main_cell, other_cell] };
    let mut root = Cell { sx: 0, sy: 0, node: Node::Split(axis, children) };
    root.set_along(axis, mainw + otherw + 1);
    root.set_along(other_axis, cross);
    root
}

fn tiled(panes: &[PaneId], w: u32, h: u32, max_columns: u32) -> Cell {
    let n = panes.len() as u32;
    let (mut rows, mut columns) = (1u32, 1u32);
    while rows * columns < n {
        rows += 1;
        if rows * columns < n && (max_columns == 0 || columns < max_columns) {
            columns += 1;
        }
    }
    let width = (w.saturating_sub(columns - 1) / columns).max(PANE_MINIMUM);
    let height = (h.saturating_sub(rows - 1) / rows).max(PANE_MINIMUM);
    let sx = ((width + 1) * columns - 1).max(w);
    let sy = ((height + 1) * rows - 1).max(h);
    let mut row_cells = Vec::new();
    let mut it = panes.iter().copied().peekable();
    for j in 0..rows {
        if it.peek().is_none() {
            break;
        }
        if n - j * columns == 1 || columns == 1 {
            let p = it.next().expect("peeked");
            row_cells.push(Cell::leaf(p, sx, height));
            continue;
        }
        let mut kids = Vec::new();
        for _ in 0..columns {
            match it.next() {
                Some(p) => kids.push(Cell::leaf(p, width, height)),
                None => break,
            }
        }
        let used = kids.len() as u32 * (width + 1) - 1;
        if sx > used {
            let extra = sx - used;
            kids.last_mut().expect("non-empty").sx += extra;
        }
        let row = if kids.len() == 1 { kids.pop().expect("one") } else { Cell { sx, sy: height, node: Node::Split(Axis::LeftRight, kids) } };
        row_cells.push(row);
    }
    let used = row_cells.len() as u32 * (height + 1) - 1;
    if sy > used {
        let extra = (sy - used) as i64;
        row_cells.last_mut().expect("non-empty").adjust(Axis::TopBottom, extra);
    }
    if row_cells.len() == 1 {
        return row_cells.pop().expect("one");
    }
    Cell { sx, sy, node: Node::Split(Axis::TopBottom, row_cells) }
}

#[cfg(test)]
mod tests;
