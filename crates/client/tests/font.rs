#![cfg(windows)]
use wraith_client::atlas::{GlyphAtlas, GlyphKey};
use wraith_client::font::FontStack;
use wraith_client::gpu::Gpu;
use wraith_proto::StyleFlags;

fn stack(family: &str, dpi: u32) -> FontStack {
    FontStack::new(&[family.to_string()], 12.0, dpi, &[]).unwrap()
}

#[test]
fn metrics_are_sane() {
    let m = stack("Cascadia Mono", 96).metrics();
    assert!((7..=12).contains(&m.w), "w {}", m.w);
    assert!((14..=22).contains(&m.h), "h {}", m.h);
    assert!(m.baseline < m.h && m.baseline > 0);
    assert!(m.underline_thickness >= 1);
}

#[test]
fn metrics_scale_with_dpi() {
    let a = stack("Cascadia Mono", 96).metrics();
    let b = stack("Cascadia Mono", 144).metrics();
    assert!((b.h as f32 - a.h as f32 * 1.5).abs() <= 1.0, "{} vs {}", a.h, b.h);
    assert!((b.w as f32 - a.w as f32 * 1.5).abs() <= 1.0, "{} vs {}", a.w, b.w);
}

#[test]
fn plain_text_one_glyph_per_cell() {
    let g = stack("Cascadia Mono", 96).shape("abc", StyleFlags(0));
    assert_eq!(g.iter().map(|g| (g.cell, g.cells_wide)).collect::<Vec<_>>(), vec![(0, 1), (1, 1), (2, 1)]);
}

#[test]
fn wide_char_spans_two_cells() {
    let g = stack("Cascadia Mono", 96).shape("日x", StyleFlags(0));
    assert_eq!(g[0].cells_wide, 2);
    assert_eq!(g.last().unwrap().cell, 2);
}

#[test]
fn ligature_spans_two_cells() {
    let s = FontStack::new(&["Cascadia Code".into()], 12.0, 96, &[]).unwrap();
    if !s.has_family("Cascadia Code") {
        eprintln!("Cascadia Code not installed; skipping");
        return;
    }
    // Cascadia Code ligates with per-character contextual alternates: same cells, different glyphs.
    let lig: Vec<u16> = s.shape("=>", StyleFlags(0)).iter().map(|g| g.glyph).collect();
    let plain: Vec<u16> = ["=", ">"].iter().map(|c| s.shape(c, StyleFlags(0))[0].glyph).collect();
    assert_ne!(lig, plain, "shaping must apply calt/liga");
}

#[test]
fn emoji_falls_back_to_color_face() {
    let g = stack("Cascadia Mono", 96).shape("😀", StyleFlags(0));
    assert_eq!(g.len(), 1);
    assert!(g[0].color, "{g:?}");
    assert_eq!(g[0].cells_wide, 2);
}

#[test]
fn missing_family_falls_back_not_errors() {
    let s = FontStack::new(&["No Such Font".into()], 12.0, 96, &[]).unwrap();
    assert!(s.metrics().w > 0);
    assert!(!s.shape("a", StyleFlags(0)).is_empty());
}

#[test]
fn atlas_returns_same_entry_for_same_key() {
    let gpu = Gpu::new().unwrap();
    let fonts = stack("Cascadia Mono", 96);
    let mut atlas = GlyphAtlas::new(&gpu, 512).unwrap();
    let g = fonts.shape("A", StyleFlags(0))[0];
    let key = GlyphKey { face: g.face, glyph: g.glyph, subpx: 0 };
    let a = atlas.get(&gpu, &fonts, key);
    let b = atlas.get(&gpu, &fonts, key);
    assert_eq!(a, b);
    assert!(a.w > 0 && a.h > 0);
    let other = atlas.get(&gpu, &fonts, GlyphKey { glyph: fonts.shape("B", StyleFlags(0))[0].glyph, ..key });
    assert_ne!(a.uv, other.uv);
}

#[test]
fn atlas_survives_overflow() {
    let gpu = Gpu::new().unwrap();
    let fonts = stack("Cascadia Mono", 96);
    let mut atlas = GlyphAtlas::new(&gpu, 64).unwrap(); // tiny: forces resets
    for c in ('!'..='~').chain('À'..='ÿ') {
        let g = fonts.shape(&c.to_string(), StyleFlags(0))[0];
        let e = atlas.get(&gpu, &fonts, GlyphKey { face: g.face, glyph: g.glyph, subpx: 0 });
        assert!(e.uv[2] <= 1.0 && e.uv[3] <= 1.0);
    }
}
