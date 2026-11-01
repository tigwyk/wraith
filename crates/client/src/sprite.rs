//! Block elements and box drawing drawn as exact cell rectangles instead of font glyphs.
//! Font glyphs are fractional-pixel sized, so adjacent blocks and lines show seams.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub alpha: f32,
}

/// Rectangles (cell-relative pixels) for `c` in a `w`×`h` cell with light line thickness `t`,
/// or `None` if `c` is left to the font.
pub fn sprite_rects(c: char, w: u32, h: u32, t: u32) -> Option<Vec<SpriteRect>> {
    let cp = c as u32;
    match cp {
        0x2580..=0x259F => Some(block(cp, w, h)),
        0x2500..=0x2570 | 0x2574..=0x257F => Some(lines(BOX[(cp - 0x2500) as usize], w, h, t.max(1))),
        _ => None,
    }
}

fn rect(x: u32, y: u32, w: u32, h: u32) -> SpriteRect {
    SpriteRect { x, y, w, h, alpha: 1.0 }
}

fn block(cp: u32, w: u32, h: u32) -> Vec<SpriteRect> {
    let (hw, hh) = (w / 2, h / 2);
    // Eighths rounded to whole pixels so neighbours meet exactly.
    let ey = |k: u32| (h * k + 4) / 8;
    let ex = |k: u32| (w * k + 4) / 8;
    let quads = |ul: bool, ur: bool, ll: bool, lr: bool| {
        let mut v = Vec::new();
        if ul {
            v.push(rect(0, 0, hw, hh));
        }
        if ur {
            v.push(rect(hw, 0, w - hw, hh));
        }
        if ll {
            v.push(rect(0, hh, hw, h - hh));
        }
        if lr {
            v.push(rect(hw, hh, w - hw, h - hh));
        }
        v
    };
    let shade = |a: f32| vec![SpriteRect { x: 0, y: 0, w, h, alpha: a }];
    match cp {
        0x2580 => vec![rect(0, 0, w, hh)],
        0x2581..=0x2587 => {
            let top = h - ey(cp - 0x2580);
            vec![rect(0, top, w, h - top)]
        }
        0x2588 => vec![rect(0, 0, w, h)],
        0x2589..=0x258F => vec![rect(0, 0, ex(8 - (cp - 0x2588)), h)],
        0x2590 => vec![rect(hw, 0, w - hw, h)],
        0x2591 => shade(0.25),
        0x2592 => shade(0.5),
        0x2593 => shade(0.75),
        0x2594 => vec![rect(0, 0, w, ey(1))],
        0x2595 => {
            let left = w - ex(1);
            vec![rect(left, 0, w - left, h)]
        }
        0x2596 => quads(false, false, true, false),
        0x2597 => quads(false, false, false, true),
        0x2598 => quads(true, false, false, false),
        0x2599 => quads(true, false, true, true),
        0x259A => quads(true, false, false, true),
        0x259B => quads(true, true, true, false),
        0x259C => quads(true, true, false, true),
        0x259D => quads(false, true, false, false),
        0x259E => quads(false, true, true, false),
        _ => quads(false, true, true, true), // 0x259F
    }
}

// Arm weights, packed as up/right/down/left digits: 0 none, 1 light, 2 heavy, 3 double.
// shortcut: dashed lines draw solid, rounded corners draw square, double-line joins overlap
// instead of mitring; refine if users notice.
const BOX: [u16; 0x80] = {
    let src = b"\
0101020210102020010102021010202001010202101020200110021001200220\
0011001200210022110012002100220010011002200120021110121021101120\
2120221012202220101110122011102120212012102220220111011202110212\
0121012202210222110111021201120221012102220122021111111212111212\
2111112121212112221111221221221212222122222122220101020210102020\
0303303003100130033000130031003313003100330010033001300313103130\
3330101330313033031301310333130331013303131331313333011000111001\
1100000000000000000110000100001000022000020000200201102001022010";
    let mut t = [0u16; 0x80];
    let mut i = 0;
    while i < 0x80 {
        let k = i * 4;
        t[i] = ((src[k] - b'0') as u16) << 12
            | ((src[k + 1] - b'0') as u16) << 8
            | ((src[k + 2] - b'0') as u16) << 4
            | (src[k + 3] - b'0') as u16;
        i += 1;
    }
    t
};

fn lines(arms: u16, w: u32, h: u32, t: u32) -> Vec<SpriteRect> {
    let (up, right, down, left) = (arms >> 12 & 0xF, arms >> 8 & 0xF, arms >> 4 & 0xF, arms & 0xF);
    let (cx, cy) = (w / 2, h / 2);
    let thick = |weight: u16| if weight == 2 { 2 * t } else { t };
    // Strokes (offset from the centre line, thickness) for a weight.
    let strokes = |weight: u16| -> Vec<(i32, u32)> {
        match weight {
            0 => vec![],
            3 => vec![(-(t as i32) - (t as i32 + 1) / 2, t), ((t as i32) - (t as i32) / 2, t)],
            k => {
                let th = thick(k);
                vec![(-(th as i32) / 2, th)]
            }
        }
    };
    // How far a horizontal arm must reach past the centre to cover the widest vertical stroke.
    let vreach = thick(up.max(down)) as i32 + if up == 3 || down == 3 { t as i32 * 2 } else { 0 };
    let hreach = thick(left.max(right)) as i32 + if left == 3 || right == 3 { t as i32 * 2 } else { 0 };
    let clamp = |v: i32, hi: u32| v.clamp(0, hi as i32) as u32;
    let mut out = Vec::new();
    for (off, th) in strokes(left) {
        let y = clamp(cy as i32 + off, h);
        out.push(rect(0, y, clamp(cx as i32 + vreach / 2 + 1, w), th.min(h - y)));
    }
    for (off, th) in strokes(right) {
        let y = clamp(cy as i32 + off, h);
        let x = clamp(cx as i32 - vreach / 2, w);
        out.push(rect(x, y, w - x, th.min(h - y)));
    }
    for (off, th) in strokes(up) {
        let x = clamp(cx as i32 + off, w);
        out.push(rect(x, 0, th.min(w - x), clamp(cy as i32 + hreach / 2 + 1, h)));
    }
    for (off, th) in strokes(down) {
        let x = clamp(cx as i32 + off, w);
        let y = clamp(cy as i32 - hreach / 2, h);
        out.push(rect(x, y, th.min(w - x), h - y));
    }
    out
}
