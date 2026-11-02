use std::path::PathBuf;

use wraith_config::edit::{HEADER, Origin, Which, origins, set, unset, write};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wraith-edit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn set_replaces_last_of_several() {
    assert_eq!(set("font-size = 10\nfont-size = 11\n", "font-size", "14", Which::Last), "font-size = 10\nfont-size = 14\n");
}

#[test]
fn set_replaces_first_for_lists() {
    assert_eq!(set("font-family = A\nfont-family = B\n", "font-family", "C", Which::First), "font-family = C\nfont-family = B\n");
}

#[test]
fn set_appends_when_missing() {
    assert_eq!(set("theme = x\n", "font-size", "14", Which::Last), "theme = x\nfont-size = 14\n");
    assert_eq!(set("theme = x", "font-size", "14", Which::Last), "theme = x\nfont-size = 14\n");
    assert_eq!(set("", "font-size", "14", Which::Last), "font-size = 14\n");
}

#[test]
fn set_keeps_bom_and_crlf() {
    assert_eq!(set("\u{feff}font-size = 10\r\n", "font-size", "14", Which::Last), "\u{feff}font-size = 14\r\n");
    assert_eq!(set("a = 1\r\n", "b", "2", Which::Last), "a = 1\r\nb = 2\r\n");
    assert_eq!(set("\u{feff}", "b", "2", Which::Last), "\u{feff}b = 2\n");
}

#[test]
fn comments_and_similar_keys_untouched() {
    let src = "# font-size = 9\nfont-size-x = 1\n  font-size=10  \n";
    assert_eq!(set(src, "font-size", "14", Which::Last), "# font-size = 9\nfont-size-x = 1\nfont-size = 14\n");
}

#[test]
fn unset_removes_every_line() {
    assert_eq!(unset("a = 1\nfont-size = 10\n# font-size = 9\nfont-size = 11\n", "font-size"), "a = 1\n# font-size = 9\n");
    assert_eq!(unset("a = 1\n", "font-size"), "a = 1\n");
    assert_eq!(unset("a = 1\r\nfont-size = 10", "font-size"), "a = 1\r\n");
}

#[test]
fn origins_follow_includes() {
    let dir = temp("origins");
    std::fs::write(dir.join("config"), "# top\nfont-size = 10\nconfig-file = inc\n").unwrap();
    std::fs::write(dir.join("inc"), "theme = x\nconfig-file = inc2\n").unwrap();
    std::fs::write(dir.join("inc2"), "cursor-style = bar\n").unwrap();
    let keys = [("font-size", Which::Last), ("theme", Which::Last), ("cursor-style", Which::Last), ("sidebar-width", Which::Last)];
    let o = origins(&dir.join("config"), &keys);
    assert_eq!(o[0], Origin::Here { line: 2 });
    assert_eq!(o[1], Origin::Included { file: dir.join("inc"), line: 1 });
    assert_eq!(o[2], Origin::Included { file: dir.join("inc2"), line: 1 });
    assert_eq!(o[3], Origin::Default);
}

#[test]
fn origins_pick_the_winning_line() {
    let dir = temp("winning");
    std::fs::write(dir.join("config"), "font-family = A\nfont-size = 1\nfont-family = B\nfont-size = 2\n").unwrap();
    let o = origins(&dir.join("config"), &[("font-family", Which::First), ("font-size", Which::Last)]);
    assert_eq!(o, vec![Origin::Here { line: 1 }, Origin::Here { line: 4 }]);
}

#[test]
fn write_creates_missing_file_with_header() {
    let p = temp("create").join("sub").join("config");
    write(&p, |s| set(s, "font-size", "14", Which::Last)).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), format!("{HEADER}\nfont-size = 14\n"));
}

#[test]
fn write_reads_the_file_fresh() {
    let p = temp("fresh").join("config");
    write(&p, |s| set(s, "a", "1", Which::Last)).unwrap();
    std::fs::write(&p, "b = 2\n").unwrap(); // edited by hand meanwhile
    write(&p, |s| set(s, "a", "3", Which::Last)).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "b = 2\na = 3\n");
}

#[test]
fn write_keeps_utf16() {
    let p = temp("u16").join("config");
    let enc = |s: &str| {
        let mut b = vec![0xFF, 0xFE];
        b.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        b
    };
    std::fs::write(&p, enc("a = 1\r\n")).unwrap();
    write(&p, |s| set(s, "b", "2", Which::Last)).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), enc("a = 1\r\nb = 2\r\n"));
}

#[test]
fn write_refuses_invalid_utf8() {
    let p = temp("bad").join("config");
    std::fs::write(&p, b"a = \xff\n").unwrap();
    let e = write(&p, |s| set(s, "b", "2", Which::Last)).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&p).unwrap(), b"a = \xff\n");
}

#[test]
fn write_fails_on_read_only_file_and_leaves_it() {
    let p = temp("ro").join("config");
    std::fs::write(&p, "a = 1\n").unwrap();
    let mut perm = std::fs::metadata(&p).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&p, perm.clone()).unwrap();
    assert!(write(&p, |s| set(s, "b", "2", Which::Last)).is_err());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a = 1\n");
    assert!(!p.with_extension("tmp").exists());
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    std::fs::set_permissions(&p, perm).unwrap();
}
