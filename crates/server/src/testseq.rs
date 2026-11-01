//! Fixed escape sequences for the M0 ConPTY passthrough check.

/// (name, bytes written, bytes that must reach the reader unchanged)
pub const SEQS: &[(&str, &[u8], &[u8])] = &[
    ("kitty", b"\x1b_Ga=T,f=24,s=1,v=1;AAAA\x1b\\", b"\x1b_Ga=T,f=24,s=1,v=1;AAAA"),
    ("osc9", b"\x1b]9;wraith-osc9\x07", b"\x1b]9;wraith-osc9"),
    ("osc777", b"\x1b]777;notify;T;wraith-777\x1b\\", b"\x1b]777;notify;T;wraith-777"),
    ("osc133", b"\x1b]133;A\x1b\\", b"\x1b]133;A"),
    ("osc7", b"\x1b]7;file://host/C:/tmp\x1b\\", b"\x1b]7;file://host/C:/tmp"),
];

pub fn get(name: &str) -> Option<&'static [u8]> {
    SEQS.iter().find(|s| s.0 == name).map(|s| s.1)
}
