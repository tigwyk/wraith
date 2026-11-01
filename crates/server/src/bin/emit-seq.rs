//! Test helper: write one fixed escape sequence to stdout and exit.
use std::io::Write;

fn main() {
    let name = std::env::args().nth(1).unwrap_or_default();
    let Some(seq) = wraith_server::testseq::get(&name) else {
        eprintln!("unknown sequence {name}");
        std::process::exit(2);
    };
    let mut out = std::io::stdout().lock();
    out.write_all(seq).unwrap();
    out.flush().unwrap();
}
