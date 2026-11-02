//! Kitty graphics (spec §3.4, M3): placements read from libghostty each frame, and image pixels
//! uploaded to the client once per view. libghostty parses the protocol and owns storage and eviction.
use std::collections::HashMap;

use libghostty_vt::Terminal;
use libghostty_vt::alloc::{Allocator, Bytes};
use libghostty_vt::kitty::graphics::{DecodePng, DecodedImage, ImageFormat, PlacementIterator, set_png_decoder};
use wraith_proto::ImagePlacement;
use wraith_proto::pane::PaneEvent;

/// PNG → RGBA8 with the `png` crate (libghostty only stores RGBA from a decoder).
struct PngDecoder;

impl DecodePng for PngDecoder {
    fn decode_png<'alloc>(&mut self, alloc: &'alloc Allocator<'_>, data: &[u8]) -> Option<DecodedImage<'alloc>> {
        let mut d = png::Decoder::new(std::io::Cursor::new(data));
        d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = d.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        let format = match info.color_type {
            png::ColorType::Rgb => ImageFormat::Rgb,
            png::ColorType::Rgba => ImageFormat::Rgba,
            png::ColorType::Grayscale => ImageFormat::Gray,
            png::ColorType::GrayscaleAlpha => ImageFormat::GrayAlpha,
            png::ColorType::Indexed => return None, // EXPAND turns palettes into RGB(A)
        };
        let rgba = to_rgba(format, info.width, info.height, &buf[..info.buffer_size()])?;
        let mut bytes = Bytes::new_with_alloc(alloc, rgba.len()).ok()?;
        bytes.copy_from_slice(&rgba);
        Some(DecodedImage { width: info.width, height: info.height, data: bytes })
    }
}

/// Install the PNG decoder, once per process. libghostty's hook is process-global and unsynchronized,
/// so every thread calls this before its first libghostty call: the one real install then runs while
/// no thread can be inside libghostty (the others wait on the `Once`).
pub fn install_png_decoder() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: see above; no other thread has reached libghostty yet.
        if let Err(e) = unsafe { set_png_decoder(Some(Box::new(PngDecoder))) } {
            tracing::warn!("kitty graphics: no PNG decoder: {e}");
        }
    });
}

/// Stored pixels as RGBA8, or `None` when `data` is shorter than `w`×`h` says.
pub fn to_rgba(format: ImageFormat, w: u32, h: u32, data: &[u8]) -> Option<Vec<u8>> {
    let n = w as usize * h as usize;
    let bpp = match format {
        ImageFormat::Rgba => 4,
        ImageFormat::Rgb => 3,
        ImageFormat::GrayAlpha => 2,
        ImageFormat::Gray => 1,
        _ => return None,
    };
    let data = data.get(..n * bpp)?;
    if bpp == 4 {
        return Some(data.to_vec());
    }
    let mut out = Vec::with_capacity(n * 4);
    for p in data.chunks_exact(bpp) {
        out.extend_from_slice(&match p {
            [r, g, b] => [*r, *g, *b, 255],
            [v, a] => [*v, *v, *v, *a],
            [v] => [*v, *v, *v, 255],
            _ => unreachable!(),
        });
    }
    Some(out)
}

/// Every placement on screen now. Virtual (Unicode placeholder) placements are not drawn yet.
pub fn collect(term: &Terminal) -> Vec<ImagePlacement> {
    let mut out = Vec::new();
    let (Ok(g), Ok(mut iter)) = (term.kitty_graphics(), PlacementIterator::new()) else { return out };
    let Ok(mut it) = iter.update(&g) else { return out };
    while let Some(p) = it.next() {
        if p.is_virtual().unwrap_or(true) {
            continue;
        }
        let Some(image) = p.image_id().ok().and_then(|id| g.image(id)) else { continue };
        let Ok(info) = p.placement_render_info(&image, term) else { continue };
        if !info.viewport_visible {
            continue;
        }
        out.push(ImagePlacement {
            image_id: p.image_id().unwrap_or(0),
            col: info.viewport_col,
            row: info.viewport_row,
            cols: info.grid_cols.min(u32::from(u16::MAX)) as u16,
            rows: info.grid_rows.min(u32::from(u16::MAX)) as u16,
            x_off: p.x_offset().unwrap_or(0),
            y_off: p.y_offset().unwrap_or(0),
            src: (info.source_x, info.source_y, info.source_width, info.source_height),
            px: (info.pixel_width, info.pixel_height),
            z: p.z().unwrap_or(0),
        });
    }
    out
}

/// Which images the current view holds, by id and libghostty generation (a retransmission of the
/// same id gets a new generation and is uploaded again).
#[derive(Default)]
pub struct ImageSync {
    sent: HashMap<u32, u64>,
}

impl ImageSync {
    /// A new view (reattach) holds nothing.
    pub fn reset(&mut self) {
        self.sent.clear();
    }

    /// Uploads for referenced images the view lacks, deletes for ones storage dropped, and the
    /// placements whose image the view has. Images over `max_bytes` (they would not fit a frame) are
    /// logged and left out.
    pub fn sync(&mut self, term: &Terminal, max_bytes: usize) -> (Vec<PaneEvent>, Vec<ImagePlacement>) {
        let mut events = Vec::new();
        let Ok(g) = term.kitty_graphics() else { return (events, Vec::new()) };
        let mut placements = collect(term);
        placements.retain(|p| {
            let Some(img) = g.image(p.image_id) else { return false };
            let generation = img.generation().unwrap_or(0);
            if self.sent.get(&p.image_id) == Some(&generation) {
                return true;
            }
            let (w, h) = (img.width().unwrap_or(0), img.height().unwrap_or(0));
            if w as usize * h as usize * 4 > max_bytes {
                tracing::warn!(image = p.image_id, w, h, "kitty image too large to send; skipped");
                return false;
            }
            // Stored pixels are always decompressed (zlib inflated, PNG decoded) by libghostty.
            let Some(rgba) = img.format().ok().zip(img.data().ok().flatten()).and_then(|(f, d)| to_rgba(f, w, h, d)) else { return false };
            events.push(PaneEvent::ImageUpload { image_id: p.image_id, width: w, height: h, rgba });
            self.sent.insert(p.image_id, generation);
            true
        });
        self.sent.retain(|&id, _| {
            let alive = g.image(id).is_some();
            if !alive {
                events.push(PaneEvent::ImageDelete { image_id: id });
            }
            alive
        });
        (events, placements)
    }
}
