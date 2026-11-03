//! Loaded fonts by id, so each pane can have its own and the glyph atlas can key glyphs by font.

use std::collections::HashSet;

pub type FontId = u16;

#[derive(Clone, Debug, PartialEq)]
pub struct FontKey {
    pub families: Vec<String>,
    pub size_pt: f32,
    pub features: Vec<String>,
}

/// Generic over the loaded font so it is testable without DirectWrite; the app uses `FontTable<FontStack>`.
pub struct FontTable<F> {
    entries: Vec<(FontId, FontKey, F)>,
    next: FontId,
}

impl<F> FontTable<F> {
    /// Entry 0 is the window font; it is never dropped.
    pub fn new(window: FontKey, font: F) -> Self {
        Self { entries: vec![(0, window, font)], next: 1 }
    }

    /// Once ids run out (`exhausted`), new keys get the window font until the table and atlas are rebuilt.
    pub fn get_or_load<E>(&mut self, key: &FontKey, load: impl FnOnce(&FontKey) -> Result<F, E>) -> Result<FontId, E> {
        if let Some((id, ..)) = self.entries.iter().find(|e| e.1 == *key) {
            return Ok(*id);
        }
        if self.exhausted() {
            return Ok(0);
        }
        let font = load(key)?;
        let id = self.next;
        self.next += 1;
        self.entries.push((id, key.clone(), font));
        Ok(id)
    }

    /// Unknown id → entry 0.
    pub fn get(&self, id: FontId) -> &F {
        &self.entries.iter().find(|e| e.0 == id).unwrap_or(&self.entries[0]).2
    }

    /// Ids are never reused, so atlas entries of a dropped font can't be mistaken for a new one's.
    pub fn retain(&mut self, used: &HashSet<FontId>) {
        self.entries.retain(|e| e.0 == 0 || used.contains(&e.0));
    }

    /// True when the next new id would wrap; the caller then rebuilds table and atlas together.
    pub fn exhausted(&self) -> bool {
        self.next == FontId::MAX
    }
}
