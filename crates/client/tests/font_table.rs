use std::collections::HashSet;
use wraith_client::font_table::{FontKey, FontTable};

fn key(f: &str) -> FontKey {
    FontKey { families: vec![f.into()], size_pt: 12.0, features: vec![] }
}

fn ok(s: &'static str) -> impl FnOnce(&FontKey) -> Result<&'static str, ()> {
    move |_| Ok(s)
}

#[test]
fn same_key_same_id() {
    let mut t = FontTable::new(key("win"), "w");
    let mut loads = 0;
    let a = t.get_or_load(&key("a"), |_| { loads += 1; Ok::<_, ()>("a") }).unwrap();
    let b = t.get_or_load(&key("a"), |_| { loads += 1; Ok::<_, ()>("a") }).unwrap();
    assert_eq!(a, b);
    assert_eq!(loads, 1);
}

#[test]
fn retain_drops_unused_but_not_window() {
    let mut t = FontTable::new(key("win"), "w");
    let one = t.get_or_load(&key("a"), ok("a")).unwrap();
    let two = t.get_or_load(&key("b"), ok("b")).unwrap();
    t.retain(&HashSet::from([two]));
    assert_eq!(*t.get(one), "w");
    assert_eq!(*t.get(two), "b");
}

#[test]
fn ids_are_not_reused_after_prune() {
    let mut t = FontTable::new(key("win"), "w");
    assert_eq!(t.get_or_load(&key("a"), ok("a")).unwrap(), 1);
    t.retain(&HashSet::new());
    assert_eq!(t.get_or_load(&key("b"), ok("b")).unwrap(), 2);
}

#[test]
fn load_error_is_returned_and_nothing_inserted() {
    let mut t = FontTable::new(key("win"), "w");
    assert_eq!(t.get_or_load(&key("a"), |_| Err("boom")), Err("boom"));
    assert_eq!(t.get_or_load(&key("a"), ok("a")).unwrap(), 1);
}

#[test]
fn an_exhausted_table_hands_out_the_window_font() {
    // Ids never repeat until the table and atlas are rebuilt; past the last id the window font is the safe answer.
    let mut t = FontTable::new(key("win"), "w");
    let mut i = 0u32;
    while !t.exhausted() {
        t.get_or_load(&key(&i.to_string()), ok("x")).unwrap();
        t.retain(&HashSet::new());
        i += 1;
    }
    let mut loaded = false;
    let id = t.get_or_load(&key("late"), |_| { loaded = true; Ok::<_, ()>("late") }).unwrap();
    assert_eq!((id, *t.get(id), loaded), (0, "w", false));
}
