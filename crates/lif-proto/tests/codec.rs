//! Codec tests: round trips, stream splitting, the compatibility rules and
//! hostile input.

use lif_proto::*;
use lif_types::{PixelSize, Size};

fn hello_full() -> Hello {
    Hello {
        protocol: PROTOCOL_VERSION,
        caps: Caps(0xdead_beef_0000_0001),
        pid: 4242,
        size: Size::new(211, 57),
        pixels: PixelSize { width: 1688, height: 912 },
        term: "xterm-256color".into(),
        cwd: "/home/ü/projects".into(),
        env: vec![("COLORTERM".into(), "truecolor".into()), ("LANG".into(), "en_US.UTF-8".into())],
        is_tty: true,
    }
}

fn client_msgs() -> Vec<ClientMsg> {
    vec![
        ClientMsg::Hello(hello_full()),
        ClientMsg::Hello(Hello::default()),
        ClientMsg::Command { argv: vec!["new-session".into(), "-s".into(), "work".into(), String::new()] },
        ClientMsg::Command { argv: vec![] },
        ClientMsg::Input(b"\x1b[A\x00\xff hello".to_vec()),
        ClientMsg::Input(vec![]),
        ClientMsg::Resize { size: Size::new(80, 24), pixels: PixelSize { width: 640, height: 480 } },
        ClientMsg::Detach,
    ]
}

fn server_msgs() -> Vec<ServerMsg> {
    vec![
        ServerMsg::Welcome { protocol: 1, caps: Caps(7), server_version: "lif 0.1.0".into(), server_pid: 99 },
        ServerMsg::CommandDone { status: -3, stdout: b"out\n".to_vec(), stderr: vec![0, 1, 2, 255] },
        ServerMsg::CommandDone { status: 0, stdout: vec![], stderr: vec![] },
        ServerMsg::Attached,
        ServerMsg::Output((0..=255u8).collect()),
        ServerMsg::Output(vec![]),
        ServerMsg::Exit { reason: ExitReason::Detached, message: String::new() },
        ServerMsg::Exit { reason: ExitReason::SessionEnded, message: "[exited]".into() },
        ServerMsg::Exit { reason: ExitReason::ServerExit, message: "bye".into() },
        ServerMsg::Exit { reason: ExitReason::Kicked, message: "kicked".into() },
        ServerMsg::Exit { reason: ExitReason::Error, message: "oops".into() },
    ]
}

fn encode_all<M: Message>(msgs: &[M]) -> Vec<u8> {
    let mut out = Vec::new();
    for m in msgs {
        m.encode(&mut out).unwrap();
    }
    out
}

fn drain<M: Message>(d: &mut Decoder<M>, into: &mut Vec<M>) {
    while let Some(m) = d.next().unwrap() {
        into.push(m);
    }
}

/// Builds a frame by hand, as some other implementation might.
fn raw_frame(type_id: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = (payload.len() as u32).to_le_bytes().to_vec();
    f.extend_from_slice(&type_id.to_le_bytes());
    f.extend_from_slice(payload);
    f
}

fn payload_of<M: Message>(m: &M) -> (u16, Vec<u8>) {
    let mut out = Vec::new();
    m.encode(&mut out).unwrap();
    let tid = u16::from_le_bytes([out[4], out[5]]);
    (tid, out[HEADER_LEN..].to_vec())
}

#[test]
fn round_trip_every_message() {
    for m in client_msgs() {
        let mut out = Vec::new();
        m.encode(&mut out).unwrap();
        assert_eq!(u32::from_le_bytes(out[..4].try_into().unwrap()) as usize, out.len() - HEADER_LEN);
        let mut d = Decoder::<ClientMsg>::new();
        d.push(&out);
        assert_eq!(d.next().unwrap(), Some(m));
        assert_eq!(d.next().unwrap(), None);
        assert_eq!(d.buffered(), 0);
    }
    for m in server_msgs() {
        let mut out = Vec::new();
        m.encode(&mut out).unwrap();
        let mut d = Decoder::<ServerMsg>::new();
        d.push(&out);
        assert_eq!(d.next().unwrap(), Some(m));
        assert_eq!(d.next().unwrap(), None);
    }
}

#[test]
fn exact_wire_layout() {
    let mut out = Vec::new();
    ClientMsg::Resize { size: Size::new(0x102, 0x304), pixels: PixelSize { width: 5, height: 6 } }.encode(&mut out).unwrap();
    assert_eq!(out, [8, 0, 0, 0, 0x04, 0, 0x02, 0x01, 0x04, 0x03, 5, 0, 6, 0]);
    out.clear();
    ServerMsg::Exit { reason: ExitReason::Kicked, message: "x".into() }.encode(&mut out).unwrap();
    assert_eq!(out, [6, 0, 0, 0, 0x05, 0x01, 3, 1, 0, 0, 0, b'x']);
    out.clear();
    ServerMsg::Attached.encode(&mut out).unwrap();
    assert_eq!(out, [0, 0, 0, 0, 0x03, 0x01]);
}

#[test]
fn many_frames_per_push_and_split_at_every_byte() {
    let msgs = server_msgs();
    let stream = encode_all(&msgs);
    // All at once.
    let mut d = Decoder::<ServerMsg>::new();
    let mut got = Vec::new();
    d.push(&stream);
    drain(&mut d, &mut got);
    assert_eq!(got, msgs);
    // Split in two at every byte.
    for i in 0..=stream.len() {
        let mut d = Decoder::<ServerMsg>::new();
        let mut got = Vec::new();
        d.push(&stream[..i]);
        drain(&mut d, &mut got);
        d.push(&stream[i..]);
        drain(&mut d, &mut got);
        assert_eq!(got, msgs, "split at {i}");
    }
    // One byte at a time.
    let mut d = Decoder::<ServerMsg>::new();
    let mut got = Vec::new();
    for b in &stream {
        d.push(std::slice::from_ref(b));
        drain(&mut d, &mut got);
    }
    assert_eq!(got, msgs);

    let msgs = client_msgs();
    let stream = encode_all(&msgs);
    for i in 0..=stream.len() {
        let mut d = Decoder::<ClientMsg>::new();
        let mut got = Vec::new();
        d.push(&stream[..i]);
        drain(&mut d, &mut got);
        d.push(&stream[i..]);
        drain(&mut d, &mut got);
        assert_eq!(got, msgs, "split at {i}");
    }
}

/// A reader that returns at most `step` bytes per read.
struct Trickle<'a> {
    data: &'a [u8],
    step: usize,
}

impl std::io::Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.step.min(buf.len()).min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

#[test]
fn read_from_reassembles_and_stays_bounded() {
    let mut msgs = server_msgs();
    msgs.push(ServerMsg::Output(vec![b'z'; 3 * 1024 * 1024]));
    msgs.extend(server_msgs());
    let stream = encode_all(&msgs);
    for step in [1, 7, 1000, 65536, 1 << 20, usize::MAX] {
        let mut r = Trickle { data: &stream, step };
        let mut d = Decoder::<ServerMsg>::new();
        let mut got = Vec::new();
        let mut peak = 0;
        while d.read_from(&mut r).unwrap() > 0 {
            peak = peak.max(d.capacity());
            drain(&mut d, &mut got);
        }
        assert_eq!(got, msgs, "step {step}");
        assert!(peak <= 3 * 1024 * 1024 + HEADER_LEN + 4 + 2 * READ_CHUNK, "peak {peak} at step {step}");
        // The big frame's memory is given back once it is consumed.
        assert!(d.capacity() <= 2 * READ_CHUNK, "capacity {} after drain", d.capacity());
    }
}

#[test]
fn borrowed_bytes_fast_path() {
    let data: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8).collect();
    let mut via_msg = Vec::new();
    ServerMsg::Output(data.clone()).encode(&mut via_msg).unwrap();
    let mut via_header = output_header(data.len()).to_vec();
    via_header.extend_from_slice(&data);
    assert_eq!(via_msg, via_header);
    let mut via_fn = Vec::new();
    encode_output(&data, &mut via_fn);
    assert_eq!(via_msg, via_fn);

    let mut via_msg = Vec::new();
    ClientMsg::Input(data.clone()).encode(&mut via_msg).unwrap();
    let mut via_fn = input_header(data.len()).to_vec();
    via_fn.extend_from_slice(&data);
    assert_eq!(via_msg, via_fn);

    let mut d = Decoder::<ServerMsg>::new();
    d.push(&via_header);
    ServerMsg::Attached.encode(&mut via_header).unwrap();
    let peeked = d.peek_frame().unwrap().unwrap();
    assert_eq!(peeked.type_id, type_id::OUTPUT);
    assert_eq!(peeked.bytes().unwrap(), &data[..]);
    // Peeking twice sees the same frame; consuming moves on.
    assert_eq!(d.peek_frame().unwrap().unwrap().payload.len(), data.len() + 4);
    d.consume_frame();
    assert_eq!(d.peek_frame().unwrap(), None);
    d.push(&[0, 0, 0, 0, 0x03, 0x01]);
    let f = d.next_frame().unwrap().unwrap();
    assert_eq!(f.type_id, type_id::ATTACHED);
    assert_eq!(f.bytes().unwrap(), b"");
}

#[test]
fn streaming_data_through_partial_frames() {
    let data: Vec<u8> = XorShift(5).bytes(300_000);
    // Data, then a trailing field from a newer peer, then a normal message.
    let mut payload = (data.len() as u32).to_le_bytes().to_vec();
    payload.extend_from_slice(&data);
    payload.extend_from_slice(b"future");
    let mut stream = raw_frame(type_id::OUTPUT, &payload);
    ServerMsg::Exit { reason: ExitReason::Kicked, message: "k".into() }.encode(&mut stream).unwrap();
    encode_output(b"tail", &mut stream);

    for step in [1usize, 9, 4096, 70_000, stream.len()] {
        let mut d = Decoder::<ServerMsg>::new();
        let mut out = Vec::new();
        let mut msgs = Vec::new();
        let mut peak = 0;
        let mut r = Trickle { data: &stream, step };
        loop {
            // Hand out data as it arrives, a little at a time.
            loop {
                if let Some(chunk) = d.peek_data(type_id::OUTPUT) {
                    let n = chunk.len().min(1 + out.len() % 5000);
                    out.extend_from_slice(&chunk[..n]);
                    d.consume_data(n);
                } else if let Some(m) = d.next().unwrap() {
                    msgs.push(m);
                } else {
                    break;
                }
            }
            peak = peak.max(d.capacity());
            if d.read_from(&mut r).unwrap() == 0 {
                break;
            }
        }
        assert!(out[..data.len()] == data[..], "step {step}");
        assert_eq!(&out[data.len()..], b"tail");
        // The emptied frames still decode (as empty Output), framing intact.
        assert_eq!(
            msgs,
            vec![
                ServerMsg::Output(vec![]),
                ServerMsg::Exit { reason: ExitReason::Kicked, message: "k".into() },
                ServerMsg::Output(vec![]),
            ]
        );
        assert!(peak <= 2 * READ_CHUNK + BYTES_HEADER_LEN + step.min(READ_CHUNK), "peak {peak} at step {step}");
    }
}

#[test]
fn oversize_messages_are_split_or_rejected() {
    let big = vec![0xabu8; MAX_PAYLOAD + 10];
    let mut out = Vec::new();
    assert_eq!(ServerMsg::Output(big.clone()).encode(&mut out), Err(ProtoError::TooLarge { len: MAX_PAYLOAD + 14 }));
    assert!(out.is_empty(), "a failed encode leaves the buffer untouched");
    // Exactly the largest payload is fine.
    ServerMsg::Output(big[..MAX_BYTES_CHUNK].to_vec()).encode(&mut out).unwrap();
    assert_eq!(out.len(), HEADER_LEN + MAX_PAYLOAD);
    out.clear();
    encode_output(&big, &mut out);
    let mut d = Decoder::<ServerMsg>::new();
    d.push(&out);
    let mut joined = Vec::new();
    while let Some(m) = d.next().unwrap() {
        match m {
            ServerMsg::Output(b) => joined.extend_from_slice(&b),
            m => panic!("{m:?}"),
        }
    }
    assert_eq!(joined.len(), big.len());
    assert!(joined == big);
}

#[test]
fn oversize_header_is_rejected_before_buffering() {
    let mut d = Decoder::<ClientMsg>::new();
    d.push(&((MAX_PAYLOAD + 1) as u32).to_le_bytes());
    d.push(&[0x03, 0x00]);
    assert_eq!(d.next(), Err(ProtoError::TooLarge { len: MAX_PAYLOAD + 1 }));
    // The stream is dead: further data is not buffered and the error sticks.
    d.push(&vec![0u8; 1 << 20]);
    assert!(d.buffered() <= HEADER_LEN);
    assert_eq!(d.next(), Err(ProtoError::TooLarge { len: MAX_PAYLOAD + 1 }));

    let mut d = Decoder::<ClientMsg>::new();
    let hostile = [0xff, 0xff, 0xff, 0xff, 0x03, 0x00];
    let mut r = &hostile[..];
    assert!(d.read_from(&mut r).is_err());
    assert!(d.read_from(&mut r).is_err());
}

// --- Compatibility rules ---------------------------------------------------

#[test]
fn rule1_trailing_bytes_are_ignored() {
    for m in client_msgs() {
        let (tid, mut p) = payload_of(&m);
        p.extend_from_slice(b"\x01\x02future fields");
        assert_eq!(ClientMsg::decode(tid, &p).unwrap(), m);
    }
    for m in server_msgs() {
        let (tid, mut p) = payload_of(&m);
        p.extend_from_slice(&[9; 33]);
        assert_eq!(ServerMsg::decode(tid, &p).unwrap(), m);
    }
}

#[test]
fn rule2_unknown_types_are_skipped() {
    let mut stream = raw_frame(0x7777, b"some payload we do not understand");
    stream.extend(raw_frame(0x0042, b""));
    ClientMsg::Detach.encode(&mut stream).unwrap();
    // A server message arriving at the server is just as unknown.
    ServerMsg::Attached.encode(&mut stream).unwrap();
    let mut d = Decoder::<ClientMsg>::new();
    let mut got = Vec::new();
    d.push(&stream);
    drain(&mut d, &mut got);
    assert_eq!(
        got,
        vec![
            ClientMsg::Unknown { type_id: 0x7777 },
            ClientMsg::Unknown { type_id: 0x42 },
            ClientMsg::Detach,
            ClientMsg::Unknown { type_id: type_id::ATTACHED },
        ]
    );
    assert_eq!(ClientMsg::Unknown { type_id: 1 }.encode(&mut Vec::new()), Err(ProtoError::NotEncodable));
}

#[test]
fn rule3_missing_trailing_fields_default_but_cut_fields_fail() {
    let h = hello_full();
    let (tid, p) = payload_of(&ClientMsg::Hello(h.clone()));
    // Payload offsets where each field ends.
    let env_len: usize = 4 + h.env.iter().map(|(k, v)| 8 + k.len() + v.len()).sum::<usize>();
    let mut ends = vec![0usize];
    for w in [2, 8, 4, 4, 4, 4 + h.term.len(), 4 + h.cwd.len(), env_len, 1] {
        ends.push(ends.last().unwrap() + w);
    }
    assert_eq!(*ends.last().unwrap(), p.len());
    for cut in 0..=p.len() {
        let r = ClientMsg::decode(tid, &p[..cut]);
        if let Some(fields) = ends.iter().position(|&e| e == cut) {
            let mut want = Hello::default();
            let parts: [&dyn Fn(&mut Hello); 9] = [
                &|w| w.protocol = h.protocol,
                &|w| w.caps = h.caps,
                &|w| w.pid = h.pid,
                &|w| w.size = h.size,
                &|w| w.pixels = h.pixels,
                &|w| w.term = h.term.clone(),
                &|w| w.cwd = h.cwd.clone(),
                &|w| w.env = h.env.clone(),
                &|w| w.is_tty = h.is_tty,
            ];
            for set in &parts[..fields] {
                set(&mut want);
            }
            assert_eq!(r, Ok(ClientMsg::Hello(want)), "cut at field boundary {cut}");
        } else {
            assert_eq!(r, Err(ProtoError::Truncated { type_id: tid }), "cut inside a field at {cut}");
        }
    }

    // Same for a server message.
    let m = ServerMsg::CommandDone { status: 5, stdout: b"abc".to_vec(), stderr: b"de".to_vec() };
    let (tid, p) = payload_of(&m);
    let ok = [
        (0, ServerMsg::CommandDone { status: 0, stdout: vec![], stderr: vec![] }),
        (4, ServerMsg::CommandDone { status: 5, stdout: vec![], stderr: vec![] }),
        (11, ServerMsg::CommandDone { status: 5, stdout: b"abc".to_vec(), stderr: vec![] }),
        (17, m.clone()),
    ];
    for cut in 0..=p.len() {
        match ok.iter().find(|(c, _)| *c == cut) {
            Some((_, want)) => assert_eq!(ServerMsg::decode(tid, &p[..cut]).as_ref(), Ok(want)),
            None => assert!(ServerMsg::decode(tid, &p[..cut]).is_err(), "cut {cut}"),
        }
    }
    // Empty Output / Input payloads (an older peer could not have sent less).
    assert_eq!(ServerMsg::decode(type_id::OUTPUT, b""), Ok(ServerMsg::Output(vec![])));
    assert_eq!(ClientMsg::decode(type_id::INPUT, b""), Ok(ClientMsg::Input(vec![])));
    assert_eq!(ServerMsg::decode(type_id::OUTPUT, &[2, 0, 0, 0, b'a']), Err(ProtoError::Truncated { type_id: type_id::OUTPUT }));
}

/// A newer peer: it appends a field to `Hello` and `Resize` and sends a
/// message type this build has never heard of, in the middle of a session.
#[test]
fn future_peer_talks_to_this_decoder() {
    let h = hello_full();
    let (_, mut hello_v2) = payload_of(&ClientMsg::Hello(h.clone()));
    hello_v2.extend_from_slice(&0x1234_5678u32.to_le_bytes()); // new: u32 flags
    hello_v2.extend_from_slice(&5u32.to_le_bytes()); // new: String locale
    hello_v2.extend_from_slice(b"tr_TR");
    let mut resize_v2 = Vec::new();
    for v in [100u16, 30, 800, 600, 2] {
        resize_v2.extend_from_slice(&v.to_le_bytes()); // last one: new u16 scale
    }
    let mut stream = raw_frame(type_id::HELLO, &hello_v2);
    stream.extend(raw_frame(0x00f0, b"\x01\x02\x03 clipboard sync, say"));
    stream.extend(raw_frame(type_id::RESIZE, &resize_v2));
    stream.extend(raw_frame(type_id::INPUT, b"\x02\x00\x00\x00hi\xee\xee"));
    stream.extend(raw_frame(type_id::DETACH, b"\x01"));

    let want = vec![
        ClientMsg::Hello(h),
        ClientMsg::Unknown { type_id: 0xf0 },
        ClientMsg::Resize { size: Size::new(100, 30), pixels: PixelSize { width: 800, height: 600 } },
        ClientMsg::Input(b"hi".to_vec()),
        ClientMsg::Detach,
    ];
    for split in [1, 3, stream.len()] {
        let mut d = Decoder::<ClientMsg>::new();
        let mut got = Vec::new();
        for chunk in stream.chunks(split) {
            d.push(chunk);
            drain(&mut d, &mut got);
        }
        assert_eq!(got, want);
    }
    // A future exit reason still ends the attachment.
    assert_eq!(
        ServerMsg::decode(type_id::EXIT, &[9, 0, 0, 0, 0]),
        Ok(ServerMsg::Exit { reason: ExitReason::Error, message: String::new() })
    );
}

#[test]
fn invalid_payloads_are_errors_and_the_stream_continues() {
    let mut argv = 1u32.to_le_bytes().to_vec();
    argv.extend_from_slice(&2u32.to_le_bytes());
    argv.extend_from_slice(&[0xc3, 0x28]);
    let mut stream = raw_frame(type_id::COMMAND, &argv);
    let (_, mut bad_bool) = payload_of(&ClientMsg::Hello(Hello::default()));
    *bad_bool.last_mut().unwrap() = 2;
    stream.extend(raw_frame(type_id::HELLO, &bad_bool));
    // A string length far past the payload.
    stream.extend(raw_frame(type_id::COMMAND, &[1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]));
    // A vector count far past the payload must not allocate for it.
    stream.extend(raw_frame(type_id::COMMAND, &[0xff, 0xff, 0xff, 0xff]));
    ClientMsg::Detach.encode(&mut stream).unwrap();

    let mut d = Decoder::<ClientMsg>::new();
    d.push(&stream);
    assert_eq!(d.next(), Err(ProtoError::InvalidUtf8 { type_id: type_id::COMMAND }));
    assert_eq!(d.next(), Err(ProtoError::InvalidValue { type_id: type_id::HELLO }));
    assert_eq!(d.next(), Err(ProtoError::Truncated { type_id: type_id::COMMAND }));
    assert_eq!(d.next(), Err(ProtoError::Truncated { type_id: type_id::COMMAND }));
    assert_eq!(d.next(), Ok(Some(ClientMsg::Detach)));
    assert_eq!(d.next(), Ok(None));

    let e: std::io::Error = ProtoError::TooLarge { len: 1 }.into();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
}

// --- Hostile input ---------------------------------------------------------

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn feed<M: Message + std::fmt::Debug>(rng: &mut XorShift, stream: &[u8]) {
    let mut d = Decoder::<M>::new();
    let mut rest = stream;
    let mut max_chunk = 0;
    while !rest.is_empty() {
        let n = (1 + rng.below(4096)).min(rest.len());
        max_chunk = max_chunk.max(n);
        d.push(&rest[..n]);
        rest = &rest[n..];
        loop {
            match d.next() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                // A payload error leaves framing intact; a framing error
                // kills the stream and must keep reporting.
                Err(ProtoError::TooLarge { .. }) => {
                    assert!(d.next().is_err());
                    break;
                }
                Err(_) => {}
            }
        }
        assert!(d.buffered() <= HEADER_LEN + MAX_PAYLOAD + max_chunk);
        assert!(d.capacity() <= stream.len().max(HEADER_LEN) + 2 * READ_CHUNK, "capacity {}", d.capacity());
    }
}

#[test]
fn random_garbage_and_truncations_never_panic() {
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    let valid_c = encode_all(&client_msgs());
    let valid_s = encode_all(&server_msgs());
    for round in 0..3000 {
        // Pure garbage.
        let n = rng.below(8192);
        let g = rng.bytes(n);
        feed::<ClientMsg>(&mut rng, &g);
        feed::<ServerMsg>(&mut rng, &g);

        // Garbage with plausible small lengths, so frames get through to the
        // payload decoders with random type ids.
        let mut s = Vec::new();
        for _ in 0..rng.below(20) {
            let len = rng.below(64);
            let tid = match rng.below(3) {
                0 => rng.below(6) as u16,
                1 => 0x100 + rng.below(6) as u16,
                _ => rng.next() as u16,
            };
            s.extend(raw_frame(tid, &rng.bytes(len)));
        }
        feed::<ClientMsg>(&mut rng, &s);
        feed::<ServerMsg>(&mut rng, &s);

        // Valid streams, truncated and with a few bytes flipped.
        for valid in [&valid_c, &valid_s] {
            let mut v = valid[..rng.below(valid.len() + 1)].to_vec();
            if round % 2 == 0 && !v.is_empty() {
                for _ in 0..1 + rng.below(4) {
                    let i = rng.below(v.len());
                    v[i] = rng.next() as u8;
                }
            }
            feed::<ClientMsg>(&mut rng, &v);
            feed::<ServerMsg>(&mut rng, &v);
        }

        // Every payload decoder on random and truncated payloads.
        let n = rng.below(128);
        let p = rng.bytes(n);
        for tid in [1u16, 2, 3, 4, 5, 0x101, 0x102, 0x103, 0x104, 0x105] {
            let _ = ClientMsg::decode(tid, &p);
            let _ = ServerMsg::decode(tid, &p);
        }
        for m in client_msgs() {
            let (tid, p) = payload_of(&m);
            let _ = ClientMsg::decode(tid, &p[..rng.below(p.len() + 1)]);
        }
        for m in server_msgs() {
            let (tid, p) = payload_of(&m);
            let _ = ServerMsg::decode(tid, &p[..rng.below(p.len() + 1)]);
        }
    }
}

/// Throughput of the hot path. Run with
/// `cargo test -p lif-proto --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_output_throughput() {
    let chunk = vec![b'x'; 16 * 1024];
    let mut wire = Vec::new();
    let frames = 64 * 1024; // 1 GiB of data
    let t = std::time::Instant::now();
    for _ in 0..frames {
        wire.clear();
        wire.extend_from_slice(&output_header(chunk.len()));
        wire.extend_from_slice(&chunk);
    }
    let enc = t.elapsed();
    let mut stream = Vec::new();
    for _ in 0..64 {
        encode_output(&chunk, &mut stream);
    }
    let mut d = Decoder::<ServerMsg>::new();
    let t = std::time::Instant::now();
    let mut total = 0usize;
    for _ in 0..frames / 64 {
        let mut r = &stream[..];
        while d.read_from(&mut r).unwrap() > 0 {
            while let Some(f) = d.next_frame().unwrap() {
                total += f.bytes().unwrap().len();
            }
        }
    }
    let dec = t.elapsed();
    let gib = (frames * chunk.len()) as f64 / (1u64 << 30) as f64;
    assert_eq!(total, frames * chunk.len());
    println!("encode (header + copy): {:.1} GiB/s; decode (read_from + borrow): {:.1} GiB/s", gib / enc.as_secs_f64(), gib / dec.as_secs_f64());
}
