//! Framing and payload encoding, as specified in the crate docs.
//!
//! Two layers:
//! - [`Decoder::next_frame`] / [`Decoder::peek_frame`] hand out raw frames
//!   (type id + payload) borrowed from the decoder's buffer. The hot
//!   `Output` / `Input` path uses this with [`RawFrame::bytes`] and never
//!   allocates.
//! - [`Decoder::next`] decodes frames into [`ClientMsg`] / [`ServerMsg`].

use std::fmt;
use std::io::{self, Read};
use std::marker::PhantomData;

use lif_types::{PixelSize, Size};

use crate::msg::{type_id, Caps, ClientMsg, ExitReason, Hello, ServerMsg};
use crate::{HEADER_LEN, MAX_PAYLOAD};

/// Bytes before the data of an `Input` / `Output` frame: the frame header
/// plus the u32 length of the byte string.
pub const BYTES_HEADER_LEN: usize = HEADER_LEN + 4;

/// Largest data chunk that fits in one `Input` / `Output` frame.
pub const MAX_BYTES_CHUNK: usize = MAX_PAYLOAD - 4;

/// How much [`Decoder::read_from`] asks the reader for, at least.
pub const READ_CHUNK: usize = 64 * 1024;

/// A buffer that grew past this for one big frame is released once empty, so
/// a single large message does not pin memory for the connection's lifetime.
const SHRINK_ABOVE: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// A frame announced (or a message would need) a payload above
    /// [`MAX_PAYLOAD`]. Fatal for the stream.
    TooLarge { len: usize },
    /// A field was cut off in the middle (compatibility rule 3).
    Truncated { type_id: u16 },
    /// A string field is not UTF-8.
    InvalidUtf8 { type_id: u16 },
    /// A field holds a value that is not allowed (e.g. a bool that is not 0 or 1).
    InvalidValue { type_id: u16 },
    /// `Unknown` messages describe frames we skipped; they cannot be sent.
    NotEncodable,
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::TooLarge { len } => write!(f, "frame payload of {len} bytes exceeds the {MAX_PAYLOAD} byte limit"),
            ProtoError::Truncated { type_id } => write!(f, "message 0x{type_id:x} has a field cut off in the middle"),
            ProtoError::InvalidUtf8 { type_id } => write!(f, "message 0x{type_id:x} has a string that is not UTF-8"),
            ProtoError::InvalidValue { type_id } => write!(f, "message 0x{type_id:x} has an invalid field value"),
            ProtoError::NotEncodable => f.write_str("an Unknown message cannot be encoded"),
        }
    }
}

impl std::error::Error for ProtoError {}

impl From<ProtoError> for io::Error {
    fn from(e: ProtoError) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, e)
    }
}

// ---------------------------------------------------------------------------
// Encoding

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_bool(out: &mut Vec<u8>, v: bool) {
    out.push(v as u8);
}
fn put_bytes(out: &mut Vec<u8>, v: &[u8]) {
    // A length above u32::MAX cannot fit MAX_PAYLOAD either; `frame` rejects
    // the whole message, so the wrapped value here is never sent.
    put_u32(out, v.len() as u32);
    out.extend_from_slice(v);
}
fn put_size(out: &mut Vec<u8>, v: Size) {
    put_u16(out, v.cols);
    put_u16(out, v.rows);
}
fn put_pixels(out: &mut Vec<u8>, v: PixelSize) {
    put_u16(out, v.width);
    put_u16(out, v.height);
}

fn exit_reason_to_u8(r: ExitReason) -> u8 {
    match r {
        ExitReason::Detached => 0,
        ExitReason::SessionEnded => 1,
        ExitReason::ServerExit => 2,
        ExitReason::Kicked => 3,
        ExitReason::Error => 4,
    }
}

fn exit_reason_from_u8(v: u8) -> ExitReason {
    match v {
        0 => ExitReason::Detached,
        1 => ExitReason::SessionEnded,
        2 => ExitReason::ServerExit,
        3 => ExitReason::Kicked,
        // A reason added by a newer server. The client only needs to know
        // that the attachment ended, so map it rather than fail the exit.
        _ => ExitReason::Error,
    }
}

/// Appends one frame whose payload is written by `body`. On error `out` is
/// left as it was.
fn frame(out: &mut Vec<u8>, type_id: u16, body: impl FnOnce(&mut Vec<u8>)) -> Result<(), ProtoError> {
    let start = out.len();
    out.extend_from_slice(&[0; HEADER_LEN]);
    body(out);
    let len = out.len() - start - HEADER_LEN;
    if len > MAX_PAYLOAD {
        out.truncate(start);
        return Err(ProtoError::TooLarge { len });
    }
    out[start..start + 4].copy_from_slice(&(len as u32).to_le_bytes());
    out[start + 4..start + 6].copy_from_slice(&type_id.to_le_bytes());
    Ok(())
}

/// The header of an `Input` / `Output` frame carrying `len` data bytes.
/// Write it, then the data, to send the frame without building a message.
///
/// # Panics
/// If `len` exceeds [`MAX_BYTES_CHUNK`].
pub fn bytes_header(type_id: u16, len: usize) -> [u8; BYTES_HEADER_LEN] {
    assert!(len <= MAX_BYTES_CHUNK, "chunk of {len} bytes is too large for one frame");
    let mut h = [0u8; BYTES_HEADER_LEN];
    h[0..4].copy_from_slice(&((len + 4) as u32).to_le_bytes());
    h[4..6].copy_from_slice(&type_id.to_le_bytes());
    h[6..10].copy_from_slice(&(len as u32).to_le_bytes());
    h
}

/// [`bytes_header`] for `ServerMsg::Output`.
pub fn output_header(len: usize) -> [u8; BYTES_HEADER_LEN] {
    bytes_header(type_id::OUTPUT, len)
}

/// [`bytes_header`] for `ClientMsg::Input`.
pub fn input_header(len: usize) -> [u8; BYTES_HEADER_LEN] {
    bytes_header(type_id::INPUT, len)
}

fn encode_bytes_frames(type_id: u16, bytes: &[u8], out: &mut Vec<u8>) {
    out.reserve(bytes.len() + BYTES_HEADER_LEN);
    for chunk in bytes.chunks(MAX_BYTES_CHUNK) {
        out.extend_from_slice(&bytes_header(type_id, chunk.len()));
        out.extend_from_slice(chunk);
    }
}

/// Appends `bytes` as `Output` frames: one frame, or several if `bytes` is
/// longer than [`MAX_BYTES_CHUNK`]. Nothing for empty `bytes`.
pub fn encode_output(bytes: &[u8], out: &mut Vec<u8>) {
    encode_bytes_frames(type_id::OUTPUT, bytes, out);
}

/// Appends `bytes` as `Input` frames, like [`encode_output`].
pub fn encode_input(bytes: &[u8], out: &mut Vec<u8>) {
    encode_bytes_frames(type_id::INPUT, bytes, out);
}

impl ClientMsg {
    /// Appends this message as one complete frame.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), ProtoError> {
        match self {
            ClientMsg::Hello(h) => frame(out, type_id::HELLO, |o| {
                put_u16(o, h.protocol);
                put_u64(o, h.caps.0);
                put_u32(o, h.pid);
                put_size(o, h.size);
                put_pixels(o, h.pixels);
                put_bytes(o, h.term.as_bytes());
                put_bytes(o, h.cwd.as_bytes());
                put_u32(o, h.env.len() as u32);
                for (k, v) in &h.env {
                    put_bytes(o, k.as_bytes());
                    put_bytes(o, v.as_bytes());
                }
                put_bool(o, h.is_tty);
            }),
            ClientMsg::Command { argv } => frame(out, type_id::COMMAND, |o| {
                put_u32(o, argv.len() as u32);
                for a in argv {
                    put_bytes(o, a.as_bytes());
                }
            }),
            ClientMsg::Input(b) => frame(out, type_id::INPUT, |o| put_bytes(o, b)),
            ClientMsg::Resize { size, pixels } => frame(out, type_id::RESIZE, |o| {
                put_size(o, *size);
                put_pixels(o, *pixels);
            }),
            ClientMsg::Detach => frame(out, type_id::DETACH, |_| {}),
            ClientMsg::UseTty { path, dev } => frame(out, type_id::USE_TTY, |o| {
                put_bytes(o, path.as_bytes());
                put_u64(o, *dev);
            }),
            ClientMsg::Unknown { .. } => Err(ProtoError::NotEncodable),
        }
    }

    /// Decodes one payload of type `type_id`. Unknown ids give `Unknown`.
    pub fn decode(type_id: u16, payload: &[u8]) -> Result<ClientMsg, ProtoError> {
        let mut r = Reader::new(type_id, payload);
        Ok(match type_id {
            type_id::HELLO => ClientMsg::Hello(Hello {
                protocol: r.opt(Reader::u16)?,
                caps: Caps(r.opt(Reader::u64)?),
                pid: r.opt(Reader::u32)?,
                size: r.opt(Reader::size)?,
                pixels: r.opt(Reader::pixels)?,
                term: r.opt(Reader::string)?,
                cwd: r.opt(Reader::string)?,
                env: r.opt(|r| r.vec(|r| Ok((r.string()?, r.string()?))))?,
                is_tty: r.opt(Reader::bool)?,
            }),
            type_id::COMMAND => ClientMsg::Command { argv: r.opt(|r| r.vec(Reader::string))? },
            type_id::INPUT => ClientMsg::Input(r.opt(|r| r.bytes().map(<[u8]>::to_vec))?),
            type_id::RESIZE => ClientMsg::Resize { size: r.opt(Reader::size)?, pixels: r.opt(Reader::pixels)? },
            type_id::DETACH => ClientMsg::Detach,
            type_id::USE_TTY => ClientMsg::UseTty { path: r.opt(Reader::string)?, dev: r.opt(Reader::u64)? },
            _ => ClientMsg::Unknown { type_id },
        })
    }
}

impl ServerMsg {
    /// Appends this message as one complete frame.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), ProtoError> {
        match self {
            ServerMsg::Welcome { protocol, caps, server_version, server_pid } => frame(out, type_id::WELCOME, |o| {
                put_u16(o, *protocol);
                put_u64(o, caps.0);
                put_bytes(o, server_version.as_bytes());
                put_u32(o, *server_pid);
            }),
            ServerMsg::CommandDone { status, stdout, stderr } => frame(out, type_id::COMMAND_DONE, |o| {
                put_i32(o, *status);
                put_bytes(o, stdout);
                put_bytes(o, stderr);
            }),
            ServerMsg::Attached => frame(out, type_id::ATTACHED, |_| {}),
            ServerMsg::Output(b) => frame(out, type_id::OUTPUT, |o| put_bytes(o, b)),
            ServerMsg::Exit { reason, message } => frame(out, type_id::EXIT, |o| {
                o.push(exit_reason_to_u8(*reason));
                put_bytes(o, message.as_bytes());
            }),
            ServerMsg::Tty { direct } => frame(out, type_id::TTY, |o| put_bool(o, *direct)),
            ServerMsg::Unknown { .. } => Err(ProtoError::NotEncodable),
        }
    }

    /// Decodes one payload of type `type_id`. Unknown ids give `Unknown`.
    pub fn decode(type_id: u16, payload: &[u8]) -> Result<ServerMsg, ProtoError> {
        let mut r = Reader::new(type_id, payload);
        Ok(match type_id {
            type_id::WELCOME => ServerMsg::Welcome {
                protocol: r.opt(Reader::u16)?,
                caps: Caps(r.opt(Reader::u64)?),
                server_version: r.opt(Reader::string)?,
                server_pid: r.opt(Reader::u32)?,
            },
            type_id::COMMAND_DONE => ServerMsg::CommandDone {
                status: r.opt(Reader::i32)?,
                stdout: r.opt(|r| r.bytes().map(<[u8]>::to_vec))?,
                stderr: r.opt(|r| r.bytes().map(<[u8]>::to_vec))?,
            },
            type_id::ATTACHED => ServerMsg::Attached,
            type_id::OUTPUT => ServerMsg::Output(r.opt(|r| r.bytes().map(<[u8]>::to_vec))?),
            type_id::EXIT => ServerMsg::Exit {
                reason: exit_reason_from_u8(r.opt(Reader::u8)?),
                message: r.opt(Reader::string)?,
            },
            type_id::TTY => ServerMsg::Tty { direct: r.opt(Reader::bool)? },
            _ => ServerMsg::Unknown { type_id },
        })
    }
}

/// A message type a [`Decoder`] produces: [`ClientMsg`] on the server,
/// [`ServerMsg`] on the client.
pub trait Message: Sized {
    fn decode(type_id: u16, payload: &[u8]) -> Result<Self, ProtoError>;
    fn encode(&self, out: &mut Vec<u8>) -> Result<(), ProtoError>;
}

impl Message for ClientMsg {
    fn decode(type_id: u16, payload: &[u8]) -> Result<Self, ProtoError> {
        ClientMsg::decode(type_id, payload)
    }
    fn encode(&self, out: &mut Vec<u8>) -> Result<(), ProtoError> {
        ClientMsg::encode(self, out)
    }
}

impl Message for ServerMsg {
    fn decode(type_id: u16, payload: &[u8]) -> Result<Self, ProtoError> {
        ServerMsg::decode(type_id, payload)
    }
    fn encode(&self, out: &mut Vec<u8>) -> Result<(), ProtoError> {
        ServerMsg::encode(self, out)
    }
}

// ---------------------------------------------------------------------------
// Payload decoding

struct Reader<'a> {
    type_id: u16,
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(type_id: u16, buf: &'a [u8]) -> Self {
        Reader { type_id, buf, pos: 0 }
    }

    /// A top-level field: absent at the end of the payload means default
    /// (rule 3); present but short is an error.
    fn opt<T: Default>(&mut self, read: impl FnOnce(&mut Self) -> Result<T, ProtoError>) -> Result<T, ProtoError> {
        if self.pos >= self.buf.len() { Ok(T::default()) } else { read(self) }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtoError> {
        if self.buf.len() - self.pos < n {
            return Err(ProtoError::Truncated { type_id: self.type_id });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtoError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtoError> {
        self.array().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Result<u32, ProtoError> {
        self.array().map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Result<u64, ProtoError> {
        self.array().map(u64::from_le_bytes)
    }
    fn i32(&mut self) -> Result<i32, ProtoError> {
        self.array().map(i32::from_le_bytes)
    }
    fn bool(&mut self) -> Result<bool, ProtoError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtoError::InvalidValue { type_id: self.type_id }),
        }
    }
    fn size(&mut self) -> Result<Size, ProtoError> {
        Ok(Size { cols: self.u16()?, rows: self.u16()? })
    }
    fn pixels(&mut self) -> Result<PixelSize, ProtoError> {
        Ok(PixelSize { width: self.u16()?, height: self.u16()? })
    }
    fn bytes(&mut self) -> Result<&'a [u8], ProtoError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn string(&mut self) -> Result<String, ProtoError> {
        let b = self.bytes()?;
        match std::str::from_utf8(b) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => Err(ProtoError::InvalidUtf8 { type_id: self.type_id }),
        }
    }
    fn vec<T>(&mut self, mut elem: impl FnMut(&mut Self) -> Result<T, ProtoError>) -> Result<Vec<T>, ProtoError> {
        let n = self.u32()? as usize;
        // The count comes from the peer: every element takes at least one
        // byte, so never reserve more than the bytes that are left.
        let mut v = Vec::with_capacity(n.min(self.buf.len() - self.pos));
        for _ in 0..n {
            v.push(elem(self)?);
        }
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// Streaming decoder

/// One frame as it sits in the decoder's buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawFrame<'a> {
    pub type_id: u16,
    pub payload: &'a [u8],
}

impl<'a> RawFrame<'a> {
    /// The data of an `Input` / `Output` frame, borrowed (no copy). Follows
    /// the same compatibility rules as the typed decoder.
    pub fn bytes(&self) -> Result<&'a [u8], ProtoError> {
        let mut r = Reader::new(self.type_id, self.payload);
        r.opt(Reader::bytes)
    }

    pub fn decode<M: Message>(&self) -> Result<M, ProtoError> {
        M::decode(self.type_id, self.payload)
    }
}

/// Splits a byte stream into frames and decodes them into `M`.
///
/// Memory bound: the buffer holds at most the frame being assembled
/// (`HEADER_LEN + MAX_PAYLOAD`) plus one read (`READ_CHUNK`, or what one
/// [`push`](Self::push) appends), provided the caller drains complete frames
/// before feeding more. A frame announcing more than `MAX_PAYLOAD` is
/// rejected as soon as its header arrives; after that the decoder is dead
/// (the stream has lost its framing) and every call returns the error.
pub struct Decoder<M> {
    buf: Vec<u8>,
    start: usize,
    end: usize,
    dead: Option<ProtoError>,
    _msg: PhantomData<fn() -> M>,
}

impl<M> Default for Decoder<M> {
    fn default() -> Self {
        Decoder { buf: Vec::new(), start: 0, end: 0, dead: None, _msg: PhantomData }
    }
}

impl<M> fmt::Debug for Decoder<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Decoder").field("buffered", &(self.end - self.start)).field("dead", &self.dead).finish()
    }
}

impl<M: Message> Decoder<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes received but not yet handed out as frames.
    pub fn buffered(&self) -> usize {
        self.end - self.start
    }

    /// Memory held by the buffer.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Appends received bytes. Ignored once the stream is dead.
    pub fn push(&mut self, data: &[u8]) {
        if self.dead.is_some() {
            return;
        }
        self.compact();
        let need = self.end + data.len();
        self.grow(need);
        self.buf[self.end..need].copy_from_slice(data);
        self.end = need;
        if let Err(e) = self.head() {
            self.dead = Some(e);
        }
    }

    /// One `read` from `r` into the buffer. Returns what `read` returned
    /// (0 is end of stream). Reads at least [`READ_CHUNK`] bytes' worth, or
    /// the rest of a larger frame in one go.
    pub fn read_from(&mut self, r: &mut impl Read) -> io::Result<usize> {
        if let Some(e) = &self.dead {
            return Err(e.clone().into());
        }
        if let Err(e) = self.head() {
            self.dead = Some(e.clone());
            return Err(e.into());
        }
        self.compact();
        let need = self.end + READ_CHUNK;
        self.grow(need);
        let n = r.read(&mut self.buf[self.end..need])?;
        self.end += n;
        if let Err(e) = self.head() {
            self.dead = Some(e.clone());
            return Err(e.into());
        }
        Ok(n)
    }

    /// Makes `buf[..need]` addressable. The buffer only grows (the zero fill
    /// happens once per byte of capacity, not once per read), doubling for
    /// amortized O(1) appends but never past what the frame being assembled
    /// needs, so a 16 MiB frame does not reserve 32 MiB.
    fn grow(&mut self, need: usize) {
        if self.buf.len() >= need {
            return;
        }
        if self.buf.capacity() < need {
            let frame_end = match self.head() {
                Ok(Some((_, len))) => self.start + HEADER_LEN + len,
                _ => need,
            };
            let target = need.max((2 * self.buf.capacity()).min(frame_end));
            self.buf.reserve_exact(target - self.buf.len());
        }
        self.buf.resize(need, 0);
    }

    /// Streaming access to an `Input` / `Output` style frame (payload = one
    /// byte string, then possibly fields from a newer peer): if the first
    /// buffered frame has type `type_id` and its header and length prefix
    /// have arrived, the data bytes received so far, even if the frame is
    /// not complete yet. `None` when there are none yet, the frame is
    /// another type, or it is malformed (the typed path then reports it).
    ///
    /// With [`consume_data`](Self::consume_data) this lets a relay pass a
    /// large frame through while holding only one read of it.
    pub fn peek_data(&self, type_id: u16) -> Option<&[u8]> {
        let (tid, plen, dlen) = self.data_head()?;
        if tid != type_id || dlen == 0 {
            return None;
        }
        let p = self.start + BYTES_HEADER_LEN;
        let avail = (self.end - p).min(dlen);
        if avail == 0 {
            return None;
        }
        debug_assert!(dlen + 4 <= plen);
        Some(&self.buf[p..p + avail])
    }

    /// Drops the first `n` data bytes returned by [`peek_data`](Self::peek_data).
    /// The frame stays a valid frame (its header is rewritten in place to
    /// describe the rest), so every other method keeps working on it.
    ///
    /// # Panics
    /// If `n` is more than `peek_data` returned.
    pub fn consume_data(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let avail = self.data_head().map_or(0, |(_, _, dlen)| (self.end - self.start - BYTES_HEADER_LEN).min(dlen));
        assert!(n <= avail, "consume_data({n}) but only {avail} bytes were peeked");
        let (tid, plen, dlen) = self.data_head().expect("checked above");
        let s = self.start + n;
        self.buf[s..s + 4].copy_from_slice(&((plen - n) as u32).to_le_bytes());
        self.buf[s + 4..s + 6].copy_from_slice(&tid.to_le_bytes());
        self.buf[s + 6..s + 10].copy_from_slice(&((dlen - n) as u32).to_le_bytes());
        self.start = s;
    }

    /// (type id, payload length, data length) of a well-formed byte-string
    /// frame at the head of the buffer.
    fn data_head(&self) -> Option<(u16, usize, usize)> {
        let (tid, plen) = self.head().ok()??;
        if self.buffered() < BYTES_HEADER_LEN || plen < 4 {
            return None;
        }
        let p = self.start + HEADER_LEN;
        let dlen = u32::from_le_bytes(self.buf[p..p + 4].try_into().expect("4 bytes")) as usize;
        if dlen > plen - 4 {
            return None;
        }
        Some((tid, plen, dlen))
    }

    /// The header of the first buffered frame, if complete.
    fn head(&self) -> Result<Option<(u16, usize)>, ProtoError> {
        let b = &self.buf[self.start..self.end];
        if b.len() < HEADER_LEN {
            return Ok(None);
        }
        let len = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(ProtoError::TooLarge { len });
        }
        Ok(Some((u16::from_le_bytes([b[4], b[5]]), len)))
    }

    fn compact(&mut self) {
        if self.start == self.end {
            self.start = 0;
            self.end = 0;
            if self.buf.len() > SHRINK_ABOVE {
                self.buf.truncate(READ_CHUNK);
                self.buf.shrink_to(READ_CHUNK);
            }
        } else if self.start > 0 {
            // Only the tail of a partial frame moves, and it moves once:
            // `start` stays 0 until that frame is complete and consumed.
            self.buf.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
    }

    /// The first complete frame, without consuming it. Pair with
    /// [`consume_frame`](Self::consume_frame); this lets a writer that
    /// could only write part of an `Output` keep it in place instead of
    /// copying the rest out.
    pub fn peek_frame(&self) -> Result<Option<RawFrame<'_>>, ProtoError> {
        if let Some(e) = &self.dead {
            return Err(e.clone());
        }
        match self.head()? {
            Some((type_id, len)) if self.buffered() >= HEADER_LEN + len => {
                let p = self.start + HEADER_LEN;
                Ok(Some(RawFrame { type_id, payload: &self.buf[p..p + len] }))
            }
            _ => Ok(None),
        }
    }

    /// Drops the frame [`peek_frame`](Self::peek_frame) returned. Does
    /// nothing if no complete frame is buffered.
    pub fn consume_frame(&mut self) {
        if let Ok(Some((_, len))) = self.head()
            && self.buffered() >= HEADER_LEN + len
        {
            self.start += HEADER_LEN + len;
        }
    }

    /// The next complete frame, consumed. `Ok(None)` means more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<RawFrame<'_>>, ProtoError> {
        if let Some(e) = &self.dead {
            return Err(e.clone());
        }
        match self.head() {
            Err(e) => {
                self.dead = Some(e.clone());
                Err(e)
            }
            Ok(Some((type_id, len))) if self.buffered() >= HEADER_LEN + len => {
                let p = self.start + HEADER_LEN;
                self.start = p + len;
                Ok(Some(RawFrame { type_id, payload: &self.buf[p..p + len] }))
            }
            Ok(_) => Ok(None),
        }
    }

    /// The next complete message. Unknown frame types come back as
    /// `Unknown` (rule 2). A payload that fails to decode is consumed and
    /// reported; the stream stays usable because framing is intact.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<M>, ProtoError> {
        match self.next_frame()? {
            Some(f) => f.decode().map(Some),
            None => Ok(None),
        }
    }
}
