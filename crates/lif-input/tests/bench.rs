//! Throughput of the key decoder. Run with
//! \`cargo test -p lif-input --release --test bench -- --ignored --nocapture\`.

use std::time::Instant;

use lif_input::{InputEvent, InputParser};

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog; THE QUICK BROWN FOX 0123456789\r";

fn measure(name: &str, keys_per_rep: usize, reads: &[Vec<u8>], reps: usize) {
    let mut p = InputParser::new();
    let mut out: Vec<InputEvent> = Vec::with_capacity(1024);
    let mut events = 0usize;
    let t = Instant::now();
    for _ in 0..reps {
        for r in reads {
            p.feed(r, &mut out);
            events += out.len();
            out.clear();
        }
    }
    let dt = t.elapsed().as_secs_f64();
    let keys = (keys_per_rep * reps) as f64;
    assert_eq!(events, keys_per_rep * reps);
    println!("{name:<44} {:>8.1} M keys/s  {:>6.1} ns/key", keys / dt / 1e6, dt * 1e9 / keys);
}

fn kitty_press_release(c: u8) -> Vec<u8> {
    let code = c.to_ascii_lowercase() as u32;
    let mods = if c.is_ascii_uppercase() { "2" } else { "" };
    let (code, text) = match c {
        b'\r' => (13, String::new()),
        _ => (code, format!(";{}", c as u32)),
    };
    format!("\x1b[{code};{mods}{text}u\x1b[{code};{}:3u", if mods.is_empty() { "1" } else { mods }).into_bytes()
}

#[test]
#[ignore]
fn keys_per_second() {
    let reps = 20_000;
    // One read per keystroke, as typing arrives.
    let per_key: Vec<Vec<u8>> = TEXT.iter().map(|&c| vec![c]).collect();
    measure("legacy, one key per read", TEXT.len(), &per_key, reps);
    // The same text in one read (fast typist / key repeat batching).
    measure("legacy, whole line per read", TEXT.len(), &[TEXT.to_vec()], reps);
    // Kitty flags 31: press (with text) + release per key.
    let kitty: Vec<Vec<u8>> = TEXT.iter().map(|&c| kitty_press_release(c)).collect();
    measure("kitty 31, press+release per read (events)", TEXT.len() * 2, &kitty, reps);
    let joined: Vec<u8> = kitty.concat();
    measure("kitty 31, whole line per read (events)", TEXT.len() * 2, &[joined], reps);
    // Mouse motion, SGR.
    let mouse: Vec<Vec<u8>> = (0..64).map(|i| format!("\x1b[<35;{};{}M", 10 + i, 5).into_bytes()).collect();
    measure("SGR mouse motion, one per read", 64, &mouse, reps);
}
