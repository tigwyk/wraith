//! Chrome motion (spec §10 polish): values that are pure functions of time, evaluated when a frame is
//! drawn. Nothing runs between frames, so motion never delays input (spec §6.3).
use std::time::{Duration, Instant};

use wraith_proto::model::Rect;

/// Hover and press washes.
pub const FADE: Duration = Duration::from_millis(80);
/// Slides and overlay entrances.
pub const SLIDE: Duration = Duration::from_millis(120);
/// The once-only attention pulse.
pub const PULSE: Duration = Duration::from_millis(600);

/// Cubic ease-out over `t` in 0..=1 (clamped).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// `base`, or nothing when Windows' "Show animations" is off.
pub fn duration(base: Duration, animations_on: bool) -> Duration {
    if animations_on { base } else { Duration::ZERO }
}

/// Fraction of `dur` elapsed since `start`; 1 for a zero duration.
fn progress(start: Instant, dur: Duration, now: Instant) -> f32 {
    if dur.is_zero() { 1.0 } else { now.saturating_duration_since(start).as_secs_f32() / dur.as_secs_f32() }
}

fn lerp(a: i32, b: i32, t: f32) -> i32 {
    a + ((b - a) as f32 * t).round() as i32
}

/// A rectangle that glides to each new target.
#[derive(Clone, Copy, Debug)]
pub struct Slide {
    from: Rect,
    to: Rect,
    start: Instant,
    dur: Duration,
}

impl Slide {
    pub fn new(r: Rect) -> Slide {
        Slide { from: r, to: r, start: Instant::now(), dur: Duration::ZERO }
    }

    /// Head for `to`, starting from wherever the slide is now.
    pub fn retarget(&mut self, to: Rect, now: Instant, dur: Duration) {
        if to != self.to {
            *self = Slide { from: self.value(now), to, start: now, dur };
        }
    }

    /// Jump to `to` (after a resize or DPI change: the old coordinates mean nothing now).
    pub fn snap(&mut self, to: Rect) {
        *self = Slide::new(to);
    }

    pub fn target(&self) -> Rect {
        self.to
    }

    pub fn value(&self, now: Instant) -> Rect {
        let t = ease_out(progress(self.start, self.dur, now));
        let (a, b) = (self.from, self.to);
        Rect { x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t), w: lerp(a.w, b.w, t), h: lerp(a.h, b.h, t) }
    }

    pub fn active(&self, now: Instant) -> bool {
        self.from != self.to && progress(self.start, self.dur, now) < 1.0
    }
}

/// An opacity between 0 and 1 that eases toward on or off.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    from: f32,
    on: bool,
    start: Instant,
    dur: Duration,
}

impl Default for Fade {
    fn default() -> Fade {
        Fade { from: 0.0, on: false, start: Instant::now(), dur: Duration::ZERO }
    }
}

impl Fade {
    pub fn set(&mut self, on: bool, now: Instant, dur: Duration) {
        if on != self.on {
            *self = Fade { from: self.value(now), on, start: now, dur };
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    pub fn value(&self, now: Instant) -> f32 {
        let to = if self.on { 1.0 } else { 0.0 };
        self.from + (to - self.from) * ease_out(progress(self.start, self.dur, now))
    }

    pub fn active(&self, now: Instant) -> bool {
        progress(self.start, self.dur, now) < 1.0
    }
}

/// Rises 0 → 1 → 0 once over its duration, then stays 0.
#[derive(Clone, Copy, Debug)]
pub struct Pulse {
    start: Instant,
    dur: Duration,
}

impl Pulse {
    pub fn new(start: Instant, dur: Duration) -> Pulse {
        Pulse { start, dur }
    }

    pub fn value(&self, now: Instant) -> f32 {
        let t = progress(self.start, self.dur, now);
        if t >= 1.0 { 0.0 } else { (t * std::f32::consts::PI).sin() }
    }

    pub fn active(&self, now: Instant) -> bool {
        progress(self.start, self.dur, now) < 1.0
    }
}

/// Which attention dot: tab badge, workspace row or agent row, with its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dot {
    Tab(u64),
    SidebarTab(u64),
    Workspace(u64),
    Agent(u64),
}

/// What chrome positions depend on: the window size, the docked sidebar's width, the cell size and
/// the workspace shown. When any changes, slides jump instead of gliding from stale coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub client: (i32, i32),
    pub sidebar_w: i32,
    pub cell: (u32, u32),
    pub workspace: Option<u64>,
}

/// Everything in the chrome that moves, keyed by what it belongs to. Asked for values while a frame
/// is drawn; `active` says whether another frame is wanted.
#[derive(Default)]
pub struct Motion {
    /// Windows' "Show animations" setting.
    pub on: bool,
    pub underline: Option<Slide>,
    pub ws_bar: Option<Slide>,
    pub sidebar: Option<Slide>,
    pub palette: Fade,
    pub find: Fade,
    pub toast: Fade,
    hover: std::collections::HashMap<crate::chrome::Hit, Fade>,
    pulses: std::collections::HashMap<Dot, Pulse>,
    lit: std::collections::HashSet<Dot>,
    seen: std::collections::HashSet<Dot>,
    layout: Option<Layout>,
}

impl Motion {
    pub fn new(on: bool) -> Motion {
        Motion { on, ..Motion::default() }
    }

    fn dur(&self, base: Duration) -> Duration {
        duration(base, self.on)
    }

    /// Start of a frame. A new layout snaps every slide.
    pub fn begin(&mut self, layout: Layout) {
        if self.layout != Some(layout) {
            self.layout = Some(layout);
            self.underline = None;
            self.ws_bar = None;
            self.sidebar = None;
        }
        self.seen.clear();
    }

    /// End of a frame: dots not drawn this frame are no longer lit.
    pub fn end(&mut self, now: Instant) {
        self.lit = std::mem::take(&mut self.seen);
        self.pulses.retain(|d, p| self.lit.contains(d) && p.active(now));
        self.hover.retain(|_, f| f.on() || f.active(now));
    }

    /// Where `slot` is drawn this frame, heading for `to`. A slot appears in place unless `enter`
    /// says where it comes from; `None` removes it.
    pub fn slide(&mut self, slot: fn(&mut Motion) -> &mut Option<Slide>, to: Option<Rect>, enter: Option<Rect>, now: Instant) -> Option<Rect> {
        let dur = self.dur(SLIDE);
        let s = slot(self);
        let Some(to) = to else {
            *s = None;
            return None;
        };
        let slide = s.get_or_insert_with(|| Slide::new(enter.unwrap_or(to)));
        slide.retarget(to, now, dur);
        Some(slide.value(now))
    }

    /// Opacity of an overlay: it fades in from 40% when it opens (so the key press shows on the very
    /// first frame) and is gone at once when it closes.
    pub fn overlay(&mut self, slot: fn(&mut Motion) -> &mut Fade, open: bool, now: Instant) -> f32 {
        let dur = self.dur(SLIDE);
        let f = slot(self);
        if !open {
            *f = Fade::default();
            return 0.0;
        }
        f.set(true, now, dur);
        0.4 + 0.6 * f.value(now)
    }

    /// The hover target this frame: it fades in, the previous one fades out.
    pub fn hover(&mut self, hovered: Option<&crate::chrome::Hit>, now: Instant) {
        let dur = self.dur(FADE);
        for (h, f) in &mut self.hover {
            f.set(Some(h) == hovered, now, dur);
        }
        if let Some(h) = hovered {
            self.hover.entry(h.clone()).or_default().set(true, now, dur);
        }
    }

    /// How lit `h`'s hover wash is, 0..=1.
    pub fn hover_level(&self, h: &crate::chrome::Hit, now: Instant) -> f32 {
        self.hover.get(h).map_or(0.0, |f| f.value(now))
    }

    /// Every hover wash still showing, with its level (one frame's snapshot).
    pub fn hover_levels(&self, now: Instant) -> std::collections::HashMap<crate::chrome::Hit, f32> {
        self.hover.iter().map(|(h, f)| (h.clone(), f.value(now))).filter(|(_, v)| *v > 0.0).collect()
    }

    /// A lit attention dot drawn this frame: its pulse, 0..=1, playing once when it lights up.
    pub fn dot(&mut self, d: Dot, now: Instant) -> f32 {
        self.seen.insert(d);
        if !self.lit.contains(&d) && !self.pulses.contains_key(&d) {
            let dur = self.dur(PULSE);
            self.pulses.insert(d, Pulse::new(now, dur));
        }
        self.pulses.get(&d).map_or(0.0, |p| p.value(now))
    }

    pub fn active(&self, now: Instant) -> bool {
        [self.underline, self.ws_bar, self.sidebar].iter().flatten().any(|s| s.active(now))
            || [self.palette, self.find, self.toast].iter().any(|f| f.active(now))
            || self.hover.values().any(|f| f.active(now))
            || self.pulses.values().any(|p| p.active(now))
    }
}
