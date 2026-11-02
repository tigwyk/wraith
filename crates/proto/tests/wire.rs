use std::io::{Cursor, ErrorKind};
use std::time::Duration;

use wraith_proto::model::{ModelCmd, ModelSnapshot};
use wraith_proto::pane::{PaneCmd, PaneEvent, ScrollTo, SpawnOpts};
use wraith_proto::wire::{ClientKind, ClientMsg, MAX_FRAME, PaneInfo, ServerMsg, read_frame, write_frame};
use wraith_proto::*;

fn rt<T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + PartialEq>(v: T) {
    let mut buf = Vec::new();
    write_frame(&mut buf, &v).unwrap();
    assert_eq!(read_frame::<T>(&mut Cursor::new(buf)).unwrap(), v);
}

fn spawn() -> SpawnOpts {
    SpawnOpts {
        cmdline: "wsl.exe -u lee".into(),
        cwd: Some("C:\\Users\\Lee".into()),
        env: vec![("TERM_PROGRAM".into(), "wraith".into())],
        cols: 120,
        rows: 40,
        cell_w_px: 10,
        cell_h_px: 20,
        scrollback_lines: 10_000,
        default_fg: (1, 2, 3),
        default_bg: (4, 5, 6),
        palette: vec![(7, 8, 9); 256],
        image_storage_limit: 320 * 1024 * 1024,
    }
}

fn frame() -> FrameDelta {
    FrameDelta {
        seq: 9,
        full: true,
        cols: 120,
        rows: 40,
        dirty_rows: vec![Row {
            y: 0,
            wrapped: false,
            runs: vec![StyleRun { fg: Color::Rgb(1, 2, 3), bg: Color::Default, flags: StyleFlags(StyleFlags::SELECTED), hyperlink_id: 0, text: "日本 ▐▛".into() }],
        }],
        cursor: CursorState { x: 1, y: 2, visible: true, style: CursorShape::Bar },
        colors: Colors { fg: (1, 1, 1), bg: (0, 0, 0), cursor: None, palette: vec![] },
        scrollbar: Scrollbar { total: 50, offset: 10, len: 40 },
        title: None,
        images: vec![],
    }
}

#[test]
fn roundtrip_every_message() {
    rt(ClientMsg::Hello { proto_version: wraith_proto::wire::PROTO_VERSION, kind: ClientKind::Gui });
    rt(ClientMsg::AttachWindow { workspace: None });
    rt(ClientMsg::AttachWindow { workspace: Some(3) });
    rt(ClientMsg::DetachWindow);
    rt(ClientMsg::Subscribe { panes: vec![1, 2, 3] });
    rt(ClientMsg::Model(ModelCmd::NewTab { workspace: 1, profile: "pwsh".into(), spawn: spawn() }));
    rt(ClientMsg::Cmd { pane: 7, cmd: PaneCmd::Scroll(ScrollTo::PageUp) });
    rt(ClientMsg::Cmd { pane: 7, cmd: PaneCmd::Reattach });
    rt(ClientMsg::Kill { pane: 7 });
    rt(ClientMsg::List);
    rt(ClientMsg::ServerStop);
    rt(ServerMsg::Welcome { server_version: "0.1.0".into() });
    rt(ServerMsg::VersionMismatch { server_version: "0.0.9".into() });
    rt(ServerMsg::WindowAttached { workspace: 2 });
    rt(ServerMsg::Snapshot(ModelSnapshot::default()));
    rt(ServerMsg::WorkspaceTaken { workspace: 2 });
    rt(ServerMsg::Event { pane: 7, ev: PaneEvent::Cwd("C:\\x".into()) });
    rt(ServerMsg::Event { pane: 7, ev: PaneEvent::Frame(frame()) });
    rt(ServerMsg::Event { pane: 7, ev: PaneEvent::InputWritten { waited: Duration::from_micros(130) } });
    rt(ServerMsg::Panes(vec![PaneInfo { id: 7, title: "lee@host:~".into(), attached: false, exited: Some(3), shell_pid: 4242, workspace: 1 }]));
    rt(ServerMsg::Error("nope".into()));
}

#[test]
fn model_cmd_round_trips() {
    use wraith_proto::model::{Axis, Dir};
    for c in [
        ModelCmd::NewWorkspace { name: Some("api".into()), root_dir: None, profile: "pwsh".into(), layout: "grid".into(), spawn: spawn() },
        ModelCmd::CloseWorkspace(1),
        ModelCmd::RenameWorkspace(1, "x".into()),
        ModelCmd::CloseTab(2),
        ModelCmd::SelectTab(2),
        ModelCmd::MoveTab { tab: 2, to: 1, index: Some(0) },
        ModelCmd::RenameTab(2, None),
        ModelCmd::Split { pane: 3, axis: Axis::Vertical, new_first: true, profile: "p".into(), spawn: spawn() },
        ModelCmd::ClosePane(3),
        ModelCmd::FocusPane(3),
        ModelCmd::SetRatio { tab: 2, path: vec![true, false], ratio: 0.3 },
        ModelCmd::NudgeSplit { pane: 3, dir: Dir::Left, delta: 0.05 },
        ModelCmd::Equalize(2),
        ModelCmd::ToggleZoom(2),
    ] {
        rt(ClientMsg::Model(c));
    }
}

#[test]
fn frames_are_back_to_back() {
    let mut buf = Vec::new();
    for pane in 1..=3u64 {
        write_frame(&mut buf, &ClientMsg::Kill { pane }).unwrap();
    }
    let mut r = Cursor::new(buf);
    for pane in 1..=3u64 {
        assert_eq!(read_frame::<ClientMsg>(&mut r).unwrap(), ClientMsg::Kill { pane });
    }
    assert_eq!(read_frame::<ClientMsg>(&mut r).unwrap_err().kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn oversize_frame_is_rejected_before_alloc() {
    let mut buf = u32::MAX.to_le_bytes().to_vec();
    buf.extend_from_slice(&[0; 16]);
    assert_eq!(read_frame::<ClientMsg>(&mut Cursor::new(buf)).unwrap_err().kind(), ErrorKind::InvalidData);
    let mut buf = (MAX_FRAME + 1).to_le_bytes().to_vec();
    buf.extend_from_slice(&[0; 16]);
    assert_eq!(read_frame::<ClientMsg>(&mut Cursor::new(buf)).unwrap_err().kind(), ErrorKind::InvalidData);
}

#[test]
fn truncated_frame_errors() {
    let mut buf = Vec::new();
    write_frame(&mut buf, &ServerMsg::Event { pane: 1, ev: PaneEvent::Frame(frame()) }).unwrap();
    buf.truncate(buf.len() / 2);
    assert_eq!(read_frame::<ServerMsg>(&mut Cursor::new(buf)).unwrap_err().kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn garbage_body_is_invalid_data() {
    let mut buf = 4u32.to_le_bytes().to_vec();
    buf.extend_from_slice(&[0xff; 4]);
    assert_eq!(read_frame::<ClientMsg>(&mut Cursor::new(buf)).unwrap_err().kind(), ErrorKind::InvalidData);
}
