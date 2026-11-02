//! Fixed escape sequences for the M0 ConPTY passthrough check.

/// (name, bytes written, bytes that must reach the reader unchanged)
pub const SEQS: &[(&str, &[u8], &[u8])] = &[
    ("kitty", b"\x1b_Ga=T,f=24,s=1,v=1;AAAA\x1b\\", b"\x1b_Ga=T,f=24,s=1,v=1;AAAA"),
    ("osc9", b"\x1b]9;wraith-osc9\x07", b"\x1b]9;wraith-osc9"),
    ("osc777", b"\x1b]777;notify;T;wraith-777\x1b\\", b"\x1b]777;notify;T;wraith-777"),
    ("osc133", b"\x1b]133;A\x1b\\", b"\x1b]133;A"),
    ("osc7", b"\x1b]7;file://host/C:/tmp\x1b\\", b"\x1b]7;file://host/C:/tmp"),
];

/// Agent signals the terminal must recognise (spec §5.2). "QWxsb3cgQmFzaD8=" is "Allow Bash?".
pub const AGENT_SEQS: &[(&str, &[u8])] = &[
    ("osc777-notify", b"\x1b]777;notify;Claude;needs you\x1b\\"),
    ("osc7501-blocked", b"\x1b]7501;state=blocked:kind=permission:msg=QWxsb3cgQmFzaD8=\x1b\\"),
    ("osc7501-working", b"\x1b]7501;state=working\x1b\\"),
    ("osc7501-done", b"\x1b]7501;state=done\x1b\\"),
    ("progress", b"\x1b]9;4;1;40\x1b\\"),
    ("bel", b"\x07"),
];

pub fn get(name: &str) -> Option<&'static [u8]> {
    SEQS.iter().find(|s| s.0 == name).map(|s| s.1).or_else(|| AGENT_SEQS.iter().find(|s| s.0 == name).map(|s| s.1))
}
