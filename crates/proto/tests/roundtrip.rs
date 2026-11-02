use wraith_proto::*;

fn rt<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: T) {
    let bytes = postcard::to_allocvec(&v).unwrap();
    assert_eq!(postcard::from_bytes::<T>(&bytes).unwrap(), v);
}

#[test]
fn roundtrip_all_types() {
    rt(FrameDelta {
        seq: 42,
        full: true,
        cols: 120,
        rows: 40,
        dirty_rows: vec![Row {
            y: 3,
            wrapped: true,
            runs: vec![StyleRun {
                fg: Color::Rgb(1, 2, 3),
                bg: Color::Palette(17),
                flags: StyleFlags(StyleFlags::BOLD | StyleFlags::CURLY_UNDERLINE),
                hyperlink_id: 9,
                text: "héllo 日本 😀".into(),
            }],
        }],
        cursor: CursorState { x: 5, y: 3, visible: true, style: CursorShape::Bar },
        colors: Colors { fg: (1, 1, 1), bg: (2, 2, 2), cursor: Some((3, 3, 3)), palette: vec![(4, 4, 4); 256] },
        scrollbar: Scrollbar { total: 10_000, offset: 9_960, len: 40 },
        title: Some("pwsh".into()),
        images: vec![ImagePlacement { image_id: 7, col: 2, row: -3, cols: 10, rows: 5, x_off: 1, y_off: 2, src: (0, 4, 80, 76), px: (80, 76), z: -1 }],
        search: None,
    });
    rt(SelRange { start: (1, 2), end: (30, 4), rectangle: true });
    rt(KeyEvent {
        code: KeyCode::KeyA,
        mods: Mods(Mods::CTRL | Mods::SHIFT),
        text: Some("A".into()),
        action: KeyAction::Repeat,
        composing: false,
    });
    rt(MouseEvent { x_px: 10.5, y_px: 3.25, button: Some(MouseButton::WheelUp), action: MouseAction::Press, mods: Mods(Mods::ALT) });
    rt(Color::Default);
}
