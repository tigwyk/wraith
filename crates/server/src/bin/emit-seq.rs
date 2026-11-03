//! Test helper. `emit-seq <name>`: write one fixed escape sequence.
//! `emit-seq flood [n]`: n numbered lines (forever without n). `emit-seq print <text>`: write text.
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    match args.first().map(String::as_str) {
        Some("flood") => {
            let n: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
            for i in 1..=n {
                if writeln!(out, "line-{i} the quick brown fox jumps over the lazy dog").is_err() {
                    return;
                }
            }
        }
        Some("stream") => {
            // `stream <words> <ms>`: one flushed word every <ms>, like an LLM streaming tokens.
            let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300);
            let ms: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
            for i in 0..n {
                // Wall-clock µs lets a reader measure ConPTY delay; this test helper never ships.
                let us = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros();
                let _ = write!(out, "t{i}@{us} ");
                let _ = out.flush();
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
        }
        Some("tui") => {
            // `tui <frames> <ms>`: repaint 40 colored rows per frame, like a TUI redrawing on every token.
            let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300);
            let ms: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(15);
            for i in 0..n {
                let _ = write!(out, "\x1b[?2026h\x1b[H");
                for r in 0..40 {
                    let _ = write!(out, "\x1b[3{}m{:>3} │ frame {i:>4} ── the quick brown fox jumps over the lazy dog ── {}\x1b[0m\x1b[K\r\n", r % 7 + 1, r, "▌".repeat((i as usize + r) % 20));
                }
                let _ = write!(out, "\x1b[?2026l");
                let _ = out.flush();
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
        }
        Some("sync-split") => {
            // `sync-split <frames>`: synchronized redraws flushed in two halves, so any frame sent
            // mid-redraw shows two frame numbers.
            let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
            for i in 0..n {
                let _ = write!(out, "\x1b[?2026h\x1b[H");
                for r in 0..20 {
                    let _ = write!(out, "row {r:>2} frame-{i}\x1b[K\r\n");
                    if r == 9 {
                        let _ = out.flush();
                        std::thread::sleep(std::time::Duration::from_millis(40));
                    }
                }
                let _ = write!(out, "\x1b[?2026l");
                let _ = out.flush();
            }
        }
        Some("sync-hang") => {
            // Starts a hold and never ends it: the terminal must give up and show the text.
            let _ = write!(out, "\x1b[?2026hheld-text");
            let _ = out.flush();
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        Some("print") => out.write_all(args[1..].join(" ").as_bytes()).unwrap(),
        Some("title") => out.write_all(b"\x1b]0;wraith-title\x07").unwrap(),
        Some(name) => match wraith_server::testseq::get(name) {
            Some(seq) => out.write_all(seq).unwrap(),
            None => {
                eprintln!("unknown sequence {name}");
                std::process::exit(2);
            }
        },
        None => std::process::exit(2),
    }
    out.flush().unwrap();
    // Keep the console alive briefly so the last output is flushed through ConPTY.
    std::thread::sleep(std::time::Duration::from_millis(300));
}
