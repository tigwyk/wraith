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
