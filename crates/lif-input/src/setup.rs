//! Sequences that set the outer terminal up for lif and restore it.
//!
//! Typical attach: [`write_queries`] and [`write_enter`] (with the
//! [`CapsProbe::from_env`] guess) in one write; feed replies to the probe
//! until it reports complete (the DA1 answer); then, if the probe found the
//! kitty keyboard protocol, [`write_kitty_upgrade`] and
//! [`crate::InputParser::set_kitty_keyboard`]. On detach [`write_leave`] with
//! the caps that were in effect, so every push / set is undone exactly.

use lif_types::OuterCaps;

/// Kitty keyboard flags lif pushes: all five (disambiguate 1, event types 2,
/// alternate keys 4, all keys as escape codes 8, associated text 16).
///
/// lif re-encodes every key for whatever the program in the pane asked for,
/// so it needs the most information the outer terminal can give: a program
/// that asked for release events, alternate keys or associated text cannot
/// be served from a less detailed encoding, while anything less detailed
/// (legacy bytes included) can be produced from this one. Flag 8 also makes
/// Enter / Tab / Backspace and plain text unambiguous, and flag 16 delivers
/// the text the key produced under the user's layout.
pub const KITTY_FLAGS: u8 = 31;

/// The private modes lif asks about with DECRQM.
pub const QUERIED_MODES: [u16; 5] = [2026, 2027, 1016, 2031, 2048];

/// Queries sent on attach. DA1 goes last: every terminal answers it, and in
/// order, so its answer means all others that will ever come have come.
pub fn write_queries(out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[?u"); // kitty keyboard flags
    out.extend_from_slice(b"\x1b[?2026$p\x1b[?2027$p\x1b[?1016$p\x1b[?2031$p\x1b[?2048$p");
    out.extend_from_slice(b"\x1b[>0q"); // XTVERSION
    out.extend_from_slice(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\"); // default fg / bg
    out.extend_from_slice(b"\x1b[16t"); // cell size in pixels
    out.extend_from_slice(b"\x1b[c"); // DA1, the sentinel
}

/// Enter lif's screen. In order:
///
/// 1. `CSI ? 1049 h` alternate screen (saves the cursor)
/// 2. `ESC =` application keypad, so keypad keys are distinguishable
/// 3. `CSI ? 2004 h` bracketed paste (if supported)
/// 4. `CSI ? 1004 h` focus reporting (if supported)
/// 5. `CSI > 31 u` kitty keyboard push (if supported; after 1049 because
///    kitty keeps one flag stack per screen), otherwise `CSI > 4 ; 2 m`
///    xterm modifyOtherKeys level 2 so modified keys stay distinguishable.
///
/// Mouse reporting stays off until [`write_mouse`].
pub fn write_enter(caps: &OuterCaps, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[?1049h");
    out.extend_from_slice(b"\x1b=");
    if caps.bracketed_paste {
        out.extend_from_slice(b"\x1b[?2004h");
    }
    if caps.focus_events {
        out.extend_from_slice(b"\x1b[?1004h");
    }
    if caps.kitty_keyboard {
        write_kitty_push(out);
    } else {
        out.extend_from_slice(b"\x1b[>4;2m");
    }
}

/// Undo [`write_enter`] in reverse, after turning off everything lif may
/// have turned on since (mouse, synchronized output, in-band resize,
/// color-scheme reports) and resetting what the renderer changes (cursor
/// shape, SGR, cursor visibility):
///
/// 1. `CSI ? 1003 l` `CSI ? 1002 l` `CSI ? 1000 l` `CSI ? 1016 l` `CSI ? 1006 l` mouse off
/// 2. `CSI ? 2026 l` end any synchronized update (if supported)
/// 3. `CSI ? 2048 l` `CSI ? 2031 l` in-band resize / color-scheme reports off
/// 4. `CSI < u` kitty pop (if pushed), otherwise `CSI > 4 m` modifyOtherKeys default
/// 5. `CSI ? 1004 l` (if enabled), 6. `CSI ? 2004 l` (if enabled), 7. `ESC >` normal keypad
/// 8. `CSI 0 SP q` default cursor shape, `CSI 0 m` SGR reset, `CSI ? 25 h` show cursor
/// 9. `CSI ? 1049 l` main screen, cursor restored
pub fn write_leave(caps: &OuterCaps, out: &mut Vec<u8>) {
    write_mouse(false, false, caps, out);
    if caps.sync_output {
        out.extend_from_slice(b"\x1b[?2026l");
    }
    out.extend_from_slice(b"\x1b[?2048l\x1b[?2031l");
    if caps.kitty_keyboard {
        write_kitty_pop(out);
    } else {
        out.extend_from_slice(b"\x1b[>4m");
    }
    if caps.focus_events {
        out.extend_from_slice(b"\x1b[?1004l");
    }
    if caps.bracketed_paste {
        out.extend_from_slice(b"\x1b[?2004l");
    }
    out.extend_from_slice(b"\x1b>");
    out.extend_from_slice(b"\x1b[0 q\x1b[0m\x1b[?25h");
    out.extend_from_slice(b"\x1b[?1049l");
}

/// Push [`KITTY_FLAGS`].
pub fn write_kitty_push(out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[>31u");
}

/// For a probe that discovers the kitty protocol after [`write_enter`] ran
/// without it: undo modifyOtherKeys (`CSI > 4 m`) and push
/// [`KITTY_FLAGS`]. Afterwards the caps passed to [`write_leave`] must
/// have `kitty_keyboard` set.
pub fn write_kitty_upgrade(out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[>4m");
    write_kitty_push(out);
}

/// Pop what [`write_kitty_push`] pushed.
pub fn write_kitty_pop(out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[<u");
}

/// Mouse reporting. `enable` turns on press/release (1000) and drag motion
/// (1002), which lif needs for its own borders and selection; `motion` adds
/// all-motion reports (1003) for programs that asked for them. The encoding
/// is SGR (1006) when supported, X10 otherwise; it is set before reporting
/// starts so no report arrives in the wrong encoding. Disabling turns every
/// mouse mode off, including SGR-pixel.
pub fn write_mouse(enable: bool, motion: bool, caps: &OuterCaps, out: &mut Vec<u8>) {
    if enable {
        if caps.sgr_mouse {
            out.extend_from_slice(b"\x1b[?1006h");
        }
        out.extend_from_slice(b"\x1b[?1000h\x1b[?1002h");
        out.extend_from_slice(if motion { b"\x1b[?1003h" } else { b"\x1b[?1003l" });
    } else {
        out.extend_from_slice(b"\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1016l\x1b[?1006l");
    }
}

/// SGR-pixel mouse coordinates (1016), only if supported. Tell the parser
/// with [`crate::InputParser::set_sgr_pixels`] at the same time.
pub fn write_mouse_pixels(enable: bool, caps: &OuterCaps, out: &mut Vec<u8>) {
    if caps.sgr_pixel_mouse {
        out.extend_from_slice(if enable { b"\x1b[?1016h" } else { b"\x1b[?1016l" });
    }
}

/// In-band resize reports (2048); see [`crate::CapsProbe::in_band_resize`].
pub fn write_in_band_resize(enable: bool, out: &mut Vec<u8>) {
    out.extend_from_slice(if enable { b"\x1b[?2048h" } else { b"\x1b[?2048l" });
}

/// Color-scheme change notifications (2031); see
/// [`crate::CapsProbe::color_scheme_reports`].
pub fn write_color_scheme_reports(enable: bool, out: &mut Vec<u8>) {
    out.extend_from_slice(if enable { b"\x1b[?2031h" } else { b"\x1b[?2031l" });
}
