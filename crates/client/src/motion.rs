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
/// One slow breath of `agent-tab-pulse`.
pub const BREATHE: Duration = Duration::from_millis(1600);
/// Frame pacing: running motion, and breathing alone (20 Hz is smooth for a 1.6 s wave).
const FRAME: Duration = Duration::from_millis(16);
const BREATHE_FRAME: Duration = Duration::from_millis(50);

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

/// Which attention dot: on a tab (its bar badge and sidebar row), a workspace row, or an agent row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dot {
    Tab(u64),
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

/// The chrome pieces that slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlideSlot {
    /// The active tab's underline.
    Underline,
    /// The current workspace's accent bar in the sidebar.
    WsBar,
    /// The hover-revealed (floating) sidebar.
    Sidebar,
}

/// Everything in the chrome that moves, keyed by what it belongs to. Asked for values while a frame
/// is drawn; `active` says whether another frame is wanted.
#[derive(Default)]
pub struct Motion {
    /// Windows' "Show animations" setting.
    pub on: bool,
    slides: [Option<Slide>; 3],
    /// Slots that jump to their next target this frame (the layout changed under them).
    snap: [bool; 3],
    pub palette: Fade,
    pub find: Fade,
    pub toast: Fade,
    hover: std::collections::HashMap<crate::chrome::Hit, Fade>,
    pulses: std::collections::HashMap<Dot, Pulse>,
    /// Dots lit in the model; `None` until the first model arrives (those dots light without a pulse).
    lit: Option<std::collections::HashSet<Dot>>,
    layout: Option<Layout>,
    /// `agent-tab-pulse`: when the breathing started, while some tab needs attention.
    breathing: Option<Instant>,
}

impl Motion {
    pub fn new(on: bool) -> Motion {
        Motion { on, ..Motion::default() }
    }

    fn dur(&self, base: Duration) -> Duration {
        duration(base, self.on)
    }

    /// Start of a frame. Window size, dock or font changes snap every slide; another workspace has
    /// other tabs, so it snaps the underline only (the workspace bar slides between rows).
    pub fn begin(&mut self, layout: Layout) {
        if let Some(old) = self.layout {
            if (Layout { workspace: old.workspace, ..layout }) != old {
                self.snap = [true; 3];
            } else if layout.workspace != old.workspace {
                self.snap[SlideSlot::Underline as usize] = true;
            }
        }
        self.layout = Some(layout);
    }

    /// End of a frame.
    pub fn end(&mut self, now: Instant) {
        self.snap = [false; 3];
        self.pulses.retain(|_, p| p.active(now));
        self.hover.retain(|_, f| f.on() || f.active(now));
    }

    /// Where `slot` is drawn this frame, heading for `to`. A new slot appears in place unless `enter`
    /// says where it comes from; `None` removes it.
    pub fn slide(&mut self, slot: SlideSlot, to: Option<Rect>, enter: Option<Rect>, now: Instant) -> Option<Rect> {
        let dur = self.dur(SLIDE);
        let snap = self.snap[slot as usize];
        let s = &mut self.slides[slot as usize];
        let Some(to) = to else {
            *s = None;
            return None;
        };
        match s {
            Some(sl) if snap => sl.snap(to),
            Some(sl) => sl.retarget(to, now, dur),
            None => {
                let mut sl = Slide::new(enter.unwrap_or(to));
                sl.retarget(to, now, dur);
                *s = Some(sl);
            }
        }
        s.map(|sl| sl.value(now))
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

    /// Every hover wash still showing, with its level (one frame's snapshot).
    pub fn hover_levels(&self, now: Instant) -> std::collections::HashMap<crate::chrome::Hit, f32> {
        self.hover.iter().map(|(h, f)| (h.clone(), f.value(now))).filter(|(_, v)| *v > 0.0).collect()
    }

    /// The dots the model lights now, drawn or not: those that just lit up start their pulse.
    pub fn set_lit(&mut self, lit: std::collections::HashSet<Dot>, now: Instant) {
        if let Some(prev) = &self.lit {
            let dur = duration(PULSE, self.on);
            for d in lit.difference(prev) {
                self.pulses.insert(*d, Pulse::new(now, dur));
            }
        }
        self.pulses.retain(|d, _| lit.contains(d));
        self.lit = Some(lit);
    }

    /// A lit dot's pulse, 0..=1: it plays once when its attention starts.
    pub fn dot(&self, d: Dot, now: Instant) -> f32 {
        self.pulses.get(&d).map_or(0.0, |p| p.value(now))
    }

    /// Tabs needing attention breathe while `on`; the wave starts over when breathing restarts.
    pub fn set_breathing(&mut self, on: bool, now: Instant) {
        self.breathing = if on { self.breathing.or(Some(now)) } else { None };
    }

    /// The breathing level, 0..=1: a slow wave, or a steady half when animations are off.
    pub fn breathe(&self, now: Instant) -> f32 {
        match self.breathing {
            None => 0.0,
            Some(_) if !self.on => 0.5,
            Some(start) => {
                let t = now.saturating_duration_since(start).as_secs_f32() / BREATHE.as_secs_f32();
                0.5 - 0.5 * (t * std::f32::consts::TAU).cos()
            }
        }
    }

    /// How soon the next frame is wanted: a frame for running motion, a slower one for breathing alone.
    pub fn frame_interval(&self, now: Instant) -> Option<Duration> {
        if self.active(now) {
            Some(FRAME)
        } else if self.breathing.is_some() && self.on {
            Some(BREATHE_FRAME)
        } else {
            None
        }
    }

    /// Draw now? Running motion always; breathing alone once its deadline `next_frame` has come.
    pub fn due(&self, now: Instant, next_frame: Instant) -> bool {
        self.active(now) || (self.breathing.is_some() && self.on && now >= next_frame)
    }

    /// After a frame: whether to mark the window dirty at once (running motion redraws at vsync,
    /// paced by the swap chain), and how soon to wake for the next frame.
    pub fn after_frame(&self, now: Instant, next_frame: Instant) -> (bool, Option<Duration>) {
        if self.active(now) {
            (true, Some(FRAME))
        } else if self.breathing.is_some() && self.on {
            (false, Some(next_frame.saturating_duration_since(now) + Duration::from_millis(1)))
        } else {
            (false, None)
        }
    }

    pub fn active(&self, now: Instant) -> bool {
        self.slides.iter().flatten().any(|s| s.active(now))
            || [self.palette, self.find, self.toast].iter().any(|f| f.active(now))
            || self.hover.values().any(|f| f.active(now))
            || self.pulses.values().any(|p| p.active(now))
    }
}
