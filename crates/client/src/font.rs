//! DirectWrite font stack: cell metrics, shaping into cells, fallback faces.
use std::cell::RefCell;

use unicode_width::UnicodeWidthChar;
use windows::core::BOOL;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG, DWRITE_FONT_METRICS,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_GLYPH_METRICS,
    DWRITE_SCRIPT_ANALYSIS, DWRITE_SHAPING_GLYPH_PROPERTIES, DWRITE_SHAPING_TEXT_PROPERTIES, DWRITE_TYPOGRAPHIC_FEATURES,
    DWriteCreateFactory, IDWriteFactory2, IDWriteFontCollection, IDWriteFontFace, IDWriteFontFace2, IDWriteTextAnalyzer,
};
use windows::core::{Interface, PCWSTR, Result, w};
use wraith_proto::StyleFlags;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellMetrics {
    pub w: u32,
    pub h: u32,
    pub baseline: u32,
    pub underline_pos: u32,
    pub underline_thickness: u32,
}

pub type FaceId = u16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub face: FaceId,
    pub glyph: u16,
    /// First cell this glyph's cluster covers.
    pub cell: u16,
    pub cells_wide: u8,
    pub x_offset: f32,
    pub color: bool,
}

struct Face {
    face: IDWriteFontFace,
    color: bool,
}

// shortcut: fixed fallback list instead of IDWriteFontFallback::MapCharacters (needs a COM text-analysis
// source); upgrade when users hit scripts these don't cover.
const FALLBACKS: &[&str] = &[
    "Segoe UI Emoji",
    "Segoe UI Symbol",
    "Cascadia Mono",
    "Consolas",
    "Yu Gothic UI",
    "Microsoft YaHei UI",
    "Malgun Gothic",
    "Nirmala UI",
    "Segoe UI",
];

pub struct FontStack {
    factory: IDWriteFactory2,
    analyzer: IDWriteTextAnalyzer,
    collection: IDWriteFontCollection,
    /// 0 regular, 1 bold, 2 italic, 3 bold-italic; fallbacks appended on demand.
    faces: RefCell<Vec<Face>>,
    fallback_ids: RefCell<Vec<(&'static str, Option<FaceId>)>>,
    primary: String,
    em_px: f32,
    metrics: CellMetrics,
    features: Vec<DWRITE_FONT_FEATURE>,
}

fn tag(s: &str) -> DWRITE_FONT_FEATURE_TAG {
    let b = s.as_bytes();
    let get = |i: usize| u32::from(*b.get(i).unwrap_or(&b' '));
    DWRITE_FONT_FEATURE_TAG(get(0) | get(1) << 8 | get(2) << 16 | get(3) << 24)
}

impl FontStack {
    /// `features`: Ghostty `font-feature` values like `-calt` or `ss01`.
    pub fn new(families: &[String], size_pt: f32, dpi: u32, features: &[String]) -> Result<FontStack> {
        unsafe {
            let factory: IDWriteFactory2 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let analyzer = factory.CreateTextAnalyzer()?;
            let mut collection = None;
            factory.GetSystemFontCollection(&mut collection, false)?;
            let collection = collection.unwrap();
            let names = families.iter().map(String::as_str).chain(["Cascadia Mono", "Consolas"]);
            let mut found = None;
            for name in names {
                if let Some(f) = find_face(&collection, name, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL) {
                    found = Some((name.to_string(), f));
                    break;
                }
            }
            let (primary, regular) = found.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?;
            let styled = |weight, style| find_face(&collection, &primary, weight, style).unwrap_or_else(|| regular.clone());
            let faces = vec![
                regular.clone(),
                styled(DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL),
                styled(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_ITALIC),
                styled(DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_ITALIC),
            ]
            .into_iter()
            .map(|face| Face { color: is_color(&face), face })
            .collect();

            let em_px = size_pt * dpi as f32 / 72.0;
            let metrics = cell_metrics(&regular, em_px)?;
            let mut feats = vec![("calt", 1), ("liga", 1)];
            for f in features {
                let (name, on) = match f.strip_prefix('-') {
                    Some(n) => (n, 0),
                    None => (f.strip_prefix('+').unwrap_or(f), 1),
                };
                feats.retain(|(n, _)| *n != name);
                feats.push((Box::leak(name.to_string().into_boxed_str()), on));
            }
            let features = feats.iter().map(|(n, v)| DWRITE_FONT_FEATURE { nameTag: tag(n), parameter: *v }).collect();
            Ok(FontStack {
                factory,
                analyzer,
                collection,
                faces: RefCell::new(faces),
                fallback_ids: RefCell::new(FALLBACKS.iter().map(|n| (*n, None)).collect()),
                primary,
                em_px,
                metrics,
                features,
            })
        }
    }

    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    pub fn em_px(&self) -> f32 {
        self.em_px
    }

    pub fn factory(&self) -> &IDWriteFactory2 {
        &self.factory
    }

    pub fn has_family(&self, name: &str) -> bool {
        self.primary.eq_ignore_ascii_case(name)
    }

    pub fn face(&self, id: FaceId) -> (IDWriteFontFace, bool) {
        let faces = self.faces.borrow();
        let f = &faces[id as usize];
        (f.face.clone(), f.color)
    }

    /// Shape one style run. Each char occupies its Unicode width in cells (combining marks: 0).
    pub fn shape(&self, text: &str, flags: StyleFlags) -> Vec<ShapedGlyph> {
        let style: FaceId = match (flags.has(StyleFlags::BOLD), flags.has(StyleFlags::ITALIC)) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        };
        // Per char: utf16 offset, cell, width, face.
        let mut units: Vec<u16> = Vec::with_capacity(text.len());
        let mut chars: Vec<(usize, u16, u8, FaceId)> = Vec::new();
        let mut cell: u16 = 0;
        for c in text.chars() {
            let width = c.width().unwrap_or(0) as u8;
            let face = if self.has_glyph(style, c) { style } else { self.fallback_for(c).unwrap_or(style) };
            chars.push((units.len(), cell.saturating_sub(u16::from(width == 0)), width, face));
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
            cell += u16::from(width);
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let face = chars[i].3;
            let mut j = i + 1;
            // Combining marks stay with their base character's run.
            while j < chars.len() && (chars[j].3 == face || chars[j].2 == 0) {
                j += 1;
            }
            let start = chars[i].0;
            let end = chars.get(j).map_or(units.len(), |c| c.0);
            // Cell span of each utf16 unit within the run.
            let mut unit_cells = Vec::with_capacity(end - start);
            for (k, ch) in chars[i..j].iter().enumerate() {
                let next = chars.get(i + k + 1).map_or(units.len(), |c| c.0);
                for _ in ch.0..next {
                    unit_cells.push((ch.1, ch.2));
                }
            }
            self.shape_run(&units[start..end], &unit_cells, face, face == style, &mut out);
            i = j;
        }
        out
    }

    fn has_glyph(&self, face: FaceId, c: char) -> bool {
        let f = self.faces.borrow()[face as usize].face.clone();
        glyph_index(&f, c) != 0
    }

    fn fallback_for(&self, c: char) -> Option<FaceId> {
        let mut ids = self.fallback_ids.borrow_mut();
        for (name, id) in ids.iter_mut() {
            let face = match id {
                Some(id) => self.faces.borrow()[*id as usize].face.clone(),
                None => {
                    let face = find_face(&self.collection, name, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL)?;
                    let mut faces = self.faces.borrow_mut();
                    faces.push(Face { color: is_color(&face), face: face.clone() });
                    *id = Some((faces.len() - 1) as FaceId);
                    face
                }
            };
            if glyph_index(&face, c) != 0 {
                return *id;
            }
        }
        None
    }

    fn shape_run(&self, units: &[u16], unit_cells: &[(u16, u8)], face_id: FaceId, features: bool, out: &mut Vec<ShapedGlyph>) {
        let (face, color) = self.face(face_id);
        let n = units.len();
        let max = n * 3 / 2 + 16;
        let mut cluster_map = vec![0u16; n];
        let mut text_props = vec![DWRITE_SHAPING_TEXT_PROPERTIES::default(); n];
        let mut glyphs = vec![0u16; max];
        let mut glyph_props = vec![DWRITE_SHAPING_GLYPH_PROPERTIES::default(); max];
        let mut count = 0u32;
        let feature_set = DWRITE_TYPOGRAPHIC_FEATURES {
            features: self.features.as_ptr() as *mut _,
            featureCount: self.features.len() as u32,
        };
        let feature_ptr: *const DWRITE_TYPOGRAPHIC_FEATURES = &feature_set;
        let range_len = n as u32;
        let ok = unsafe {
            self.analyzer.GetGlyphs(
                PCWSTR(units.as_ptr()),
                n as u32,
                &face,
                false,
                false,
                &DWRITE_SCRIPT_ANALYSIS::default(),
                w!("en-us"),
                None,
                if features { Some(&feature_ptr) } else { None },
                if features { Some(&range_len) } else { None },
                if features { 1 } else { 0 },
                max as u32,
                cluster_map.as_mut_ptr(),
                text_props.as_mut_ptr(),
                glyphs.as_mut_ptr(),
                glyph_props.as_mut_ptr(),
                &mut count,
            )
        };
        if ok.is_err() {
            // Unshapeable run: fall back to one nominal glyph per char.
            for (u, &(cell, w)) in units.iter().zip(unit_cells) {
                if w > 0 {
                    let c = char::from_u32(u32::from(*u)).unwrap_or('?');
                    out.push(ShapedGlyph { face: face_id, glyph: glyph_index(&face, c), cell, cells_wide: w, x_offset: 0.0, color });
                }
            }
            return;
        }
        let count = count as usize;
        // Walk clusters: units [a, b) share cluster_map value; glyphs [cluster_map[a], next) belong to them.
        let mut a = 0;
        while a < n {
            let mut b = a + 1;
            while b < n && cluster_map[b] == cluster_map[a] {
                b += 1;
            }
            let g0 = cluster_map[a] as usize;
            let g1 = if b < n { cluster_map[b] as usize } else { count };
            let first = unit_cells[a].0;
            let last = unit_cells[a..b].iter().map(|&(c, w)| c + u16::from(w)).max().unwrap_or(first + 1);
            let wide = (last.saturating_sub(first)).max(1) as u8;
            for &glyph in &glyphs[g0..g1.max(g0)] {
                out.push(ShapedGlyph { face: face_id, glyph, cell: first, cells_wide: wide, x_offset: 0.0, color });
            }
            a = b;
        }
    }
}

fn find_face(c: &IDWriteFontCollection, name: &str, weight: DWRITE_FONT_WEIGHT, style: DWRITE_FONT_STYLE) -> Option<IDWriteFontFace> {
    unsafe {
        let wname: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut index = 0u32;
        let mut exists = BOOL(0);
        c.FindFamilyName(PCWSTR(wname.as_ptr()), &mut index, &mut exists).ok()?;
        if !exists.as_bool() {
            return None;
        }
        let font = c.GetFontFamily(index).ok()?.GetFirstMatchingFont(weight, DWRITE_FONT_STRETCH_NORMAL, style).ok()?;
        font.CreateFontFace().ok()
    }
}

fn is_color(face: &IDWriteFontFace) -> bool {
    face.cast::<IDWriteFontFace2>().is_ok_and(|f| unsafe { f.IsColorFont() }.as_bool())
}

fn glyph_index(face: &IDWriteFontFace, c: char) -> u16 {
    let cp = c as u32;
    let mut g = 0u16;
    unsafe {
        let _ = face.GetGlyphIndices(&cp, 1, &mut g);
    }
    g
}

fn cell_metrics(face: &IDWriteFontFace, em_px: f32) -> Result<CellMetrics> {
    unsafe {
        let mut m = DWRITE_FONT_METRICS::default();
        face.GetMetrics(&mut m);
        let scale = em_px / f32::from(m.designUnitsPerEm);
        let g = glyph_index(face, 'M');
        let mut gm = DWRITE_GLYPH_METRICS::default();
        face.GetDesignGlyphMetrics(&g, 1, &mut gm, false)?;
        let ascent = f32::from(m.ascent) * scale;
        let descent = f32::from(m.descent) * scale;
        let gap = f32::from(m.lineGap) * scale;
        let h = (ascent + descent + gap).ceil() as u32;
        let baseline = (ascent + gap / 2.0).ceil() as u32;
        Ok(CellMetrics {
            w: (gm.advanceWidth as f32 * scale).ceil() as u32,
            h,
            baseline,
            underline_pos: (baseline as f32 - f32::from(m.underlinePosition) * scale).round().min(h as f32 - 1.0) as u32,
            underline_thickness: (f32::from(m.underlineThickness) * scale).round().max(1.0) as u32,
        })
    }
}
