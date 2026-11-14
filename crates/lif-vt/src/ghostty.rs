//! libghostty-vt backend for [`VtEngine`].
//!
//! # Ownership and lifetimes
//!
//! `libghostty_vt::Terminal<'alloc, 'cb>` borrows its allocator for `'alloc`
//! and its effect callbacks for `'cb`. We use the default allocator (so
//! `'alloc = 'static`) and only register callbacks that own everything they
//! touch: each closure captures a clone of an `Rc<RefCell<Shared>>` and
//! nothing borrowed, so `'cb = 'static` too. That makes [`GhosttyVt`] a
//! plain owned `'static` value that can be moved into a `HashMap`.
//!
//! The `Rc` gives the callback state a heap address that does not move with
//! `GhosttyVt`, and the binding itself keeps its callback table behind a raw
//! pointer for the same reason, so moving the engine never invalidates what
//! the C side holds. Callbacks only run synchronously inside `&mut self`
//! terminal calls (`vt_write*`, `resize`, `paste`), and no `RefCell` borrow
//! is held across those calls, so the `borrow_mut` in a callback can never
//! collide with one of ours. (`Rc` also makes the engine `!Send`, which
//! matches the binding: a terminal must stay on the thread that made it.)

mod frame;
mod input;
mod osc;

use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::effect::{
    ClipboardLocation, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
    PrimaryDeviceAttributes, SecondaryDeviceAttributes, SemanticPromptKind,
    TertiaryDeviceAttributes,
};
use libghostty_vt::fmt::{Format, Formatter};
use libghostty_vt::mode::Mode;
use libghostty_vt::point::{Coordinate, Point};
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::report::SizeReportSize;
use libghostty_vt::screen::Screen;
use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::{CompressionActivity, CompressionMode, CompressionResult, ScrollViewport};
use libghostty_vt::{RenderState, Terminal, key, mouse};
use lif_types::{KeyEvent, MouseEvent, PaneFrame, PixelSize, Size};

use crate::{PaneModes, Scroll, TextRange, VtEngine, VtError, VtEvent, VtMemory, VtOptions};

use frame::{FrameScratch, Links};
use osc::{OscEvent, OscWatch};

/// Replies the program has not collected yet. A program that floods queries
/// without reading the answers loses the newest ones instead of growing the
/// server without bound.
const MAX_PENDING_REPLIES: usize = 1 << 20;
/// Events not yet drained. Same reasoning as `MAX_PENDING_REPLIES`.
const MAX_PENDING_EVENTS: usize = 1024;
/// Bounded work per [`VtEngine::compact`] call.
const COMPACT_STEPS: usize = 16;
/// Cell size assumed for pixel reports when the outer terminal never told us.
const FALLBACK_CELL_PX: (u32, u32) = (8, 16);

const DA: DeviceAttributes = DeviceAttributes {
    // VT220-class with ANSI color, like xterm and Ghostty answer: `CSI ? 62 ; 22 c`.
    primary: PrimaryDeviceAttributes::new(
        ConformanceLevel::VT220,
        &[DeviceAttributeFeature::ANSI_COLOR],
    ),
    secondary: SecondaryDeviceAttributes {
        device_type: DeviceType::VT220,
        firmware_version: 1,
        rom_cartridge: 0,
    },
    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
};

const XTVERSION: &str = concat!("lif(", env!("CARGO_PKG_VERSION"), ")");

/// State written by effect callbacks during `vt_write` and read back by the
/// engine afterwards.
#[derive(Default)]
struct Shared {
    replies: Vec<u8>,
    /// Lifts `MAX_PENDING_REPLIES` while `encode_paste` borrows the reply
    /// path: a paste is bounded by its input, not by the program.
    unbounded_replies: bool,
    events: Vec<VtEvent>,
    title_changed: bool,
    pwd_changed: bool,
}

impl Shared {
    fn push_event(&mut self, ev: VtEvent) {
        if self.events.len() < MAX_PENDING_EVENTS {
            self.events.push(ev);
        }
    }
}

/// The libghostty-vt engine.
pub struct GhosttyVt {
    term: Terminal<'static, 'static>,
    render: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    key_enc: key::Encoder<'static>,
    key_ev: key::Event<'static>,
    mouse_enc: mouse::Encoder<'static>,
    mouse_ev: mouse::Event<'static>,
    shared: Rc<RefCell<Shared>>,
    size: Size,
    pixels: PixelSize,
    /// Something may have changed since the last `refresh`.
    dirty: bool,
    title: String,
    pwd: String,
    links: Links,
    scratch: FrameScratch,
    osc: OscWatch,
    /// DECSCNM state the last refresh applied.
    reverse: bool,
    /// Compression activity token at the end of the last finished pass.
    compacted_at: Option<CompressionActivity>,
}

fn engine_err(e: libghostty_vt::Error) -> VtError {
    match e {
        libghostty_vt::Error::OutOfMemory => VtError::OutOfMemory,
        e => VtError::Engine(e.to_string()),
    }
}

fn cell_px(size: Size, pixels: PixelSize) -> (u32, u32) {
    if size.is_empty() || pixels.width == 0 || pixels.height == 0 {
        return FALLBACK_CELL_PX;
    }
    let w = u32::from(pixels.width) / u32::from(size.cols);
    let h = u32::from(pixels.height) / u32::from(size.rows);
    if w == 0 || h == 0 { FALLBACK_CELL_PX } else { (w, h) }
}

fn selection_byte(loc: ClipboardLocation) -> u8 {
    match loc {
        ClipboardLocation::Standard => b'c',
        ClipboardLocation::Selection => b's',
        ClipboardLocation::Primary => b'p',
    }
}

impl GhosttyVt {
    fn install_effects(&mut self) -> Result<(), libghostty_vt::Error> {
        let s = &self.shared;
        let (s1, s2, s3, s4, s5, s6, s7, s8) =
            (s.clone(), s.clone(), s.clone(), s.clone(), s.clone(), s.clone(), s.clone(), s.clone());
        self.term
            .on_pty_write(move |_, data| {
                let mut sh = s1.borrow_mut();
                if sh.unbounded_replies || sh.replies.len() + data.len() <= MAX_PENDING_REPLIES {
                    sh.replies.extend_from_slice(data);
                }
            })?
            .on_bell(move |_| s2.borrow_mut().push_event(VtEvent::Bell))?
            .on_title_changed(move |_| {
                let mut sh = s3.borrow_mut();
                sh.title_changed = true;
                sh.push_event(VtEvent::TitleChanged);
            })?
            .on_pwd_changed(move |_| {
                let mut sh = s4.borrow_mut();
                sh.pwd_changed = true;
                sh.push_event(VtEvent::PwdChanged);
            })?
            .on_reset(move |_| {
                // RIS clears title and pwd without calling their callbacks.
                let mut sh = s5.borrow_mut();
                sh.title_changed = true;
                sh.pwd_changed = true;
                sh.push_event(VtEvent::TitleChanged);
                sh.push_event(VtEvent::PwdChanged);
            })?
            .on_clipboard_write(move |_, req| {
                let selection = selection_byte(req.location());
                let mut contents = req.contents();
                let text = contents
                    .clone()
                    .find(|c| c.mime.starts_with(b"text/plain"))
                    .or_else(|| contents.next());
                let data = text.map(|c| c.data.to_vec()).unwrap_or_default();
                s6.borrow_mut().push_event(VtEvent::ClipboardWrite { selection, data });
                req.reply(Ok(()), false);
            })?
            .on_desktop_notification(move |_, n| {
                s7.borrow_mut().push_event(VtEvent::Notification {
                    title: String::from_utf8_lossy(n.title()).into_owned(),
                    body: String::from_utf8_lossy(n.body()).into_owned(),
                });
            })?
            .on_semantic_prompt(move |_, p| {
                let ev = match p.kind() {
                    Ok(SemanticPromptKind::OutputStart) => VtEvent::CommandStarted,
                    Ok(SemanticPromptKind::CommandEnd) => {
                        VtEvent::CommandFinished { exit_code: p.exit_code() }
                    }
                    _ => return,
                };
                s8.borrow_mut().push_event(ev);
            })?
            .on_device_attributes(|_| Some(DA))?
            .on_xtversion(|_| Some(XTVERSION))?
            .on_size(|t| {
                let rows = t.rows().ok()?;
                let columns = t.cols().ok()?;
                let (w, h) = (t.width_px().ok()?, t.height_px().ok()?);
                let (cw, ch) = if columns > 0 && rows > 0 && w > 0 && h > 0 {
                    (w / u32::from(columns), h / u32::from(rows))
                } else {
                    FALLBACK_CELL_PX
                };
                Some(SizeReportSize { rows, columns, cell_width: cw, cell_height: ch })
            })?;
        Ok(())
    }

    /// Pull title / pwd out of the terminal after the callbacks flagged them.
    fn sync_strings(&mut self) {
        let (t, p) = {
            let mut sh = self.shared.borrow_mut();
            (std::mem::take(&mut sh.title_changed), std::mem::take(&mut sh.pwd_changed))
        };
        if t {
            self.title.clear();
            if let Ok(b) = self.term.title() {
                self.title.push_str(&String::from_utf8_lossy(b));
            }
        }
        if p {
            self.pwd.clear();
            if let Ok(b) = self.term.pwd() {
                self.pwd.push_str(&String::from_utf8_lossy(b));
            }
        }
    }

    fn mode(&self, m: Mode) -> bool {
        self.term.mode(m).unwrap_or(false)
    }

    /// Format `sel` (or the whole screen) as plain text.
    fn format_text(&self, sel: Option<(Point, Point)>, join_wrapped: bool) -> Option<String> {
        let mut f = Formatter::new(&self.term).ok()?;
        f.set_format(Format::Plain).ok()?.set_unwrap(join_wrapped).ok()?.set_trim(true).ok()?;
        if let Some((a, b)) = sel {
            let start = self.term.grid_ref(a).ok()?;
            let end = self.term.grid_ref(b).ok()?;
            let sel = Selection::new(start, end, false);
            // SAFETY: both grid refs were just resolved against this
            // formatter's terminal and its active screen, and the shared
            // borrow of `self.term` rules out any mutation before the
            // formatter (which copies the selection) is dropped.
            unsafe { f.set_selection(Some(&sel)) }.ok()?;
        }
        let bytes = f.format_alloc(None).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl VtEngine for GhosttyVt {
    fn new(opts: &VtOptions) -> Result<Self, VtError> {
        if opts.size.is_empty() {
            return Err(VtError::InvalidSize);
        }
        let mut term = Terminal::new(opts.size.cols, opts.size.rows).map_err(engine_err)?;
        let (cw, ch) = cell_px(opts.size, opts.pixels);
        term.resize(opts.size.cols, opts.size.rows, cw, ch).map_err(engine_err)?;
        term.set_scrollback_max_bytes(Some(opts.scrollback_bytes)).map_err(engine_err)?;
        // Mode 2027 (grapheme clustering) on by default, like Ghostty, kitty
        // and WezTerm: ZWJ emoji and flags occupy one cluster, so widths
        // agree with modern outer terminals. Programs can still turn it off.
        term.set_default_mode(Mode::GRAPHEME_CLUSTER, true).map_err(engine_err)?;
        let mut vt = GhosttyVt {
            term,
            render: RenderState::new().map_err(engine_err)?,
            rows: RowIterator::new().map_err(engine_err)?,
            cells: CellIterator::new().map_err(engine_err)?,
            key_enc: key::Encoder::new().map_err(engine_err)?,
            key_ev: key::Event::new().map_err(engine_err)?,
            mouse_enc: mouse::Encoder::new().map_err(engine_err)?,
            mouse_ev: mouse::Event::new().map_err(engine_err)?,
            shared: Rc::new(RefCell::new(Shared::default())),
            size: opts.size,
            pixels: opts.pixels,
            dirty: true,
            title: String::new(),
            pwd: String::new(),
            links: Links::default(),
            scratch: FrameScratch::default(),
            osc: OscWatch::default(),
            reverse: false,
            compacted_at: None,
        };
        vt.install_effects().map_err(engine_err)?;
        Ok(vt)
    }

    fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.dirty = true;
        let shared = &self.shared;
        self.osc.write(&mut self.term, bytes, |ev| {
            shared.borrow_mut().push_event(match ev {
                OscEvent::ClipboardRead(selection) => VtEvent::ClipboardRead { selection },
                OscEvent::Notification { title, body } => VtEvent::Notification { title, body },
            });
        });
        let flagged = {
            let sh = self.shared.borrow();
            sh.title_changed || sh.pwd_changed
        };
        if flagged {
            self.sync_strings();
        }
    }

    fn drain_replies(&mut self, out: &mut Vec<u8>) {
        let mut sh = self.shared.borrow_mut();
        out.extend_from_slice(&sh.replies);
        sh.replies.clear();
    }

    fn drain_events(&mut self, out: &mut Vec<VtEvent>) {
        out.append(&mut self.shared.borrow_mut().events);
    }

    fn resize(&mut self, size: Size, pixels: PixelSize) {
        if size.is_empty() {
            return;
        }
        let (cw, ch) = cell_px(size, pixels);
        if self.term.resize(size.cols, size.rows, cw, ch).is_ok() {
            self.size = size;
            self.pixels = pixels;
            self.dirty = true;
        }
    }

    fn size(&self) -> Size {
        self.size
    }

    fn refresh(&mut self, frame: &mut PaneFrame) -> bool {
        let changed = frame::refresh(
            &self.term,
            &mut self.render,
            &mut self.rows,
            &mut self.cells,
            &mut self.links,
            &mut self.scratch,
            self.size,
            &mut self.reverse,
            frame,
        );
        self.dirty = false;
        changed
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn modes(&self) -> PaneModes {
        let button = self.mode(Mode::BUTTON_MOUSE);
        let any = self.mode(Mode::ANY_MOUSE);
        PaneModes {
            alt_screen: matches!(self.term.active_screen(), Ok(Screen::Alternate)),
            app_cursor_keys: self.mode(Mode::DECCKM),
            app_keypad: self.mode(Mode::KEYPAD_KEYS),
            bracketed_paste: self.mode(Mode::BRACKETED_PASTE),
            focus_events: self.mode(Mode::FOCUS_EVENT),
            mouse_tracking: self.term.is_mouse_tracking().unwrap_or(false),
            mouse_motion: button || any,
            kitty_keyboard: self.term.kitty_keyboard_flags().map_or(0, |f| f.bits()),
            synchronized_output: self.mode(Mode::SYNC_OUTPUT),
        }
    }

    fn encode_key(&mut self, key: &KeyEvent, out: &mut Vec<u8>) {
        input::encode_key(&self.term, &mut self.key_enc, &mut self.key_ev, key, out);
    }

    fn encode_mouse(&mut self, event: &MouseEvent, out: &mut Vec<u8>) {
        let (cw, ch) = cell_px(self.size, self.pixels);
        input::encode_mouse(
            &self.term,
            &mut self.mouse_enc,
            &mut self.mouse_ev,
            self.size,
            (cw, ch),
            event,
            out,
        );
    }

    fn encode_paste(&mut self, data: &[u8], out: &mut Vec<u8>) {
        use libghostty_vt::effect::ClipboardMime;
        use libghostty_vt::paste::{Options, Source};
        // The terminal streams the encoded paste through the pty write
        // callback; borrow that path and move the new tail into `out`.
        let start = {
            let mut sh = self.shared.borrow_mut();
            sh.unbounded_replies = true;
            sh.replies.len()
        };
        let mimes = [ClipboardMime::new(b"text/plain")];
        // Source::Text: always write text, never a kitty paste event (that
        // needs a synchronous clipboard read we cannot serve). The user asked
        // for this paste, so unsafe text is allowed; libghostty still strips
        // control bytes and frames or CR-converts it per the program's modes.
        let opts = Options::new().with_source(Source::Text).with_allow_unsafe(true);
        let _ = self.term.paste(opts, &mimes, |_, w| w.write_all(data));
        let mut sh = self.shared.borrow_mut();
        sh.unbounded_replies = false;
        if sh.replies.len() > start {
            out.extend_from_slice(&sh.replies[start..]);
            sh.replies.truncate(start);
        }
    }

    fn encode_focus(&mut self, focused: bool, out: &mut Vec<u8>) {
        if !self.mode(Mode::FOCUS_EVENT) {
            return;
        }
        let ev = if focused {
            libghostty_vt::focus::Event::Gained
        } else {
            libghostty_vt::focus::Event::Lost
        };
        let mut buf = [0u8; 8];
        if let Ok(n) = ev.encode(&mut buf) {
            out.extend_from_slice(&buf[..n]);
        }
    }

    fn clipboard_reply(&mut self, selection: u8, data: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b]52;");
        out.push(selection);
        out.push(b';');
        osc::base64_encode(data, out);
        out.extend_from_slice(b"\x1b\\");
    }

    fn scroll(&mut self, to: Scroll) {
        let s = match to {
            Scroll::Lines(n) => ScrollViewport::Delta(n as isize),
            Scroll::Top => ScrollViewport::Top,
            Scroll::Bottom => ScrollViewport::Bottom,
        };
        self.term.scroll_viewport(s);
        self.dirty = true;
    }

    fn scrollback_rows(&self) -> usize {
        self.term.scrollback_rows().unwrap_or(0)
    }

    fn viewport_offset(&self) -> usize {
        match self.term.scrollbar() {
            Ok(sb) => sb.total.saturating_sub(sb.offset).saturating_sub(sb.len) as usize,
            Err(_) => 0,
        }
    }

    fn text(&self, range: TextRange, join_wrapped: bool) -> String {
        let last_col = self.size.cols.saturating_sub(1);
        let sel = match range {
            TextRange::All => None,
            TextRange::Screen => Some((
                Point::Viewport(Coordinate { x: 0, y: 0 }),
                Point::Viewport(Coordinate { x: last_col, y: u32::from(self.size.rows) - 1 }),
            )),
            TextRange::Rows { start, end } => {
                let total = self.term.total_rows().unwrap_or(0);
                let end = end.min(total);
                if start >= end {
                    return String::new();
                }
                let (Ok(a), Ok(b)) = (u32::try_from(start), u32::try_from(end - 1)) else {
                    return String::new();
                };
                Some((
                    Point::Screen(Coordinate { x: 0, y: a }),
                    Point::Screen(Coordinate { x: last_col, y: b }),
                ))
            }
        };
        self.format_text(sel, join_wrapped).unwrap_or_default()
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn pwd(&self) -> &str {
        &self.pwd
    }

    fn hyperlink(&self, id: u32) -> Option<&str> {
        self.links.get(id)
    }

    fn memory(&self) -> VtMemory {
        match self.term.memory_usage() {
            Ok(m) => VtMemory {
                resident_bytes: (m.primary_resident_bytes + m.alternate_resident_bytes) as usize,
                compressed_bytes: (m.primary_compressed_bytes + m.alternate_compressed_bytes)
                    as usize,
            },
            Err(_) => VtMemory::default(),
        }
    }

    fn compact(&mut self) {
        let Ok(now) = self.term.compression_activity() else { return };
        if self.compacted_at == Some(now) {
            return;
        }
        for _ in 0..COMPACT_STEPS {
            match self.term.compress(CompressionMode::Incremental) {
                Ok(CompressionResult::Pending) => continue,
                Ok(CompressionResult::Complete | CompressionResult::Unsupported) => {
                    self.compacted_at = self.term.compression_activity().ok();
                    return;
                }
                Err(_) => return,
            }
        }
    }
}
