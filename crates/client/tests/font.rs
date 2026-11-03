#![cfg(windows)]
use wraith_client::atlas::{GlyphAtlas, GlyphKey};
use wraith_client::font::{FontStack, monospace_families};
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
    let key = GlyphKey { font: 0, face: g.face, glyph: g.glyph, subpx: 0 };
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
        let e = atlas.get(&gpu, &fonts, GlyphKey { font: 0, face: g.face, glyph: g.glyph, subpx: 0 });
        assert!(e.uv[2] <= 1.0 && e.uv[3] <= 1.0);
    }
}

#[test]
fn nerd_font_icons_fall_back_from_plain_cascadia() {
    let nerd_installed = FontStack::new(&["CaskaydiaCove NFM".into()], 12.0, 96, &[]).unwrap().has_family("CaskaydiaCove NFM");
    if !nerd_installed {
        eprintln!("no Nerd Font installed; skipping");
        return;
    }
    // U+F09B (GitHub icon) is not in Cascadia Mono; an earlier fallback family that isn't installed
    // ("Symbols Nerd Font Mono" here) must not stop the search.
    let g = stack("Cascadia Mono", 96).shape("\u{f09b}", StyleFlags(0));
    assert_eq!(g.len(), 1);
    assert_ne!(g[0].glyph, 0, "icon rendered as .notdef box");
    assert!(g[0].face >= 4, "expected a fallback face, got {}", g[0].face);
}

#[test]
fn monospace_families_lists_installed_monospace_only() {
    let f = monospace_families();
    assert!(f.iter().any(|n| n == "Consolas"), "{f:?}");
    assert!(!f.iter().any(|n| n == "Arial"), "{f:?}");
    assert!(f.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
}

#[test]
fn two_fonts_same_glyph_do_not_collide() {
    let gpu = Gpu::new().unwrap();
    let small = FontStack::new(&["Cascadia Mono".to_string()], 12.0, 96, &[]).unwrap();
    let big = FontStack::new(&["Cascadia Mono".to_string()], 24.0, 96, &[]).unwrap();
    let mut atlas = GlyphAtlas::new(&gpu, 512).unwrap();
    let g = small.shape("A", StyleFlags(0))[0];
    let k = GlyphKey { font: 0, face: g.face, glyph: g.glyph, subpx: 0 };
    let a = atlas.get(&gpu, &small, k);
    let b = atlas.get(&gpu, &big, GlyphKey { font: 1, ..k });
    assert_ne!(a.h, b.h);
}

#[test]
fn installed_checks_the_system_not_the_primary() {
    let s = stack("Cascadia Mono", 96);
    assert!(!s.installed("NoSuchFont Zz"));
    if !stack("Courier New", 96).has_family("Courier New") {
        eprintln!("Courier New not installed; skipping");
        return;
    }
    assert!(!s.has_family("Courier New"), "the window stack's primary is not Courier New");
    assert!(s.installed("Courier New"));
}
