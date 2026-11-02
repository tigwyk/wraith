//! Scrollback search for one pane (M5), on libghostty's incremental search. All work is bounded per
//! frame so input never waits behind a large scrollback (spec §6.3).
use libghostty_vt::Terminal;
use libghostty_vt::search::{MatchBuffer, Search, Status};
use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::PointSpace;
use wraith_proto::{SearchHit, SearchView};

/// Search steps per frame: enough to finish a typical scrollback in a few frames, small enough to
/// keep the pane thread responsive.
const TICKS_PER_FRAME: usize = 64;

pub struct PaneSearch {
    search: Search<'static, 'static>,
    buf: MatchBuffer,
}

impl PaneSearch {
    pub fn new(term: &mut Terminal<'static, 'static>) -> Option<PaneSearch> {
        Some(PaneSearch { search: Search::new(term).ok()?, buf: MatchBuffer::new() })
    }

    pub fn set_needle(&mut self, term: &mut Terminal<'static, 'static>, needle: &str) {
        // An empty needle returns the search to idle (no matches).
        if let Err(e) = self.search.set_needle(term, needle.as_bytes()) {
            tracing::warn!("search: {e}");
        }
    }

    /// Select the next older (`older`) or newer match, scrolling the viewport to it.
    pub fn select(&mut self, term: &mut Terminal<'static, 'static>, older: bool) {
        let _ = if older { self.search.select_next(term) } else { self.search.select_prev(term) };
    }

    /// One frame's worth of progress. True while there is more to do (the caller sends another frame).
    pub fn step(&mut self, term: &mut Terminal<'static, 'static>) -> bool {
        for _ in 0..TICKS_PER_FRAME {
            match self.search.tick() {
                Ok(Status::Running) => {}
                Ok(Status::FeedRequired) => {
                    if self.search.feed(term).is_err() {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// The count and the viewport's matches, split per row (a match may wrap across rows).
    pub fn view(&mut self, term: &mut Terminal<'static, 'static>) -> SearchView {
        let (total, selected) = (self.search.total_matches().unwrap_or(0) as u32, self.search.selected_index().ok().flatten().map(|i| i as u32));
        let Ok(snap) = self.search.snapshot(term) else { return SearchView { total, selected, hits: Vec::new() } };
        let t = snap.terminal();
        let cols = t.cols().unwrap_or(1);
        let rows = t.rows().unwrap_or(0);
        let ends = |s: &Selection| -> Option<((u16, u32), (u16, u32))> {
            let a = t.point_from_grid_ref(&s.start(), PointSpace::Viewport).ok().flatten()?;
            let b = t.point_from_grid_ref(&s.end(), PointSpace::Viewport).ok().flatten()?;
            Some(((a.x, a.y), (b.x, b.y)))
        };
        let chosen = snap.selected_match().ok().flatten().as_ref().and_then(ends);
        let mut hits = Vec::new();
        if let Ok(matches) = snap.viewport_matches(&mut self.buf) {
            for m in matches {
                let Some(((x0, y0), (x1, y1))) = ends(&m) else { continue };
                let selected = chosen == Some(((x0, y0), (x1, y1)));
                for y in y0..=y1.min(u32::from(rows).saturating_sub(1)) {
                    let from = if y == y0 { x0 } else { 0 };
                    let to = if y == y1 { x1 } else { cols.saturating_sub(1) };
                    hits.push(SearchHit { y: y as u16, x0: from, x1: to, selected });
                }
            }
        }
        SearchView { total, selected, hits }
    }
}
