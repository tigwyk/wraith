#![cfg(windows)]
use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use wraith_ipc::security::{current_user_sid, pipe_sddl, process_owner_sid};
use wraith_ipc::{PipeListener, connect, pipe_name};
use wraith_proto::wire::{read_frame, write_frame};

fn unique() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\wraith-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

#[test]
fn default_name_is_per_user() {
    // SAFETY-free: only reads the variable we may have set; tests that need the override pass names directly.
    if std::env::var_os("WRAITH_PIPE_NAME").is_none() {
        assert_eq!(pipe_name(), format!(r"\\.\pipe\wraith-{}", current_user_sid()));
    }
}

#[test]
fn echo_roundtrip() {
    let name = unique();
    let mut l = PipeListener::bind(&name).unwrap();
    let server = thread::spawn(move || {
        let (mut r, mut w) = l.accept().unwrap().split();
        let v: Vec<u8> = read_frame(&mut r).unwrap();
        write_frame(&mut w, &v).unwrap();
    });
    let (mut r, mut w) = connect(&name).unwrap().split();
    write_frame(&mut w, &b"hello pipe".to_vec()).unwrap();
    let back: Vec<u8> = read_frame(&mut r).unwrap();
    assert_eq!(back, b"hello pipe");
    server.join().unwrap();
}

#[test]
fn full_duplex_does_not_deadlock() {
    const N: usize = 2000;
    let name = unique();
    let mut l = PipeListener::bind(&name).unwrap();
    let start = Instant::now();
    let server = thread::spawn(move || {
        let (mut r, mut w) = l.accept().unwrap().split();
        let writer = thread::spawn(move || {
            for i in 0..N {
                write_frame(&mut w, &vec![i as u8; 4096]).unwrap();
            }
        });
        for _ in 0..N {
            let _: Vec<u8> = read_frame(&mut r).unwrap();
        }
        writer.join().unwrap();
    });
    let (mut r, mut w) = connect(&name).unwrap().split();
    let writer = thread::spawn(move || {
        for i in 0..N {
            write_frame(&mut w, &vec![i as u8; 4096]).unwrap();
        }
    });
    for i in 0..N {
        let v: Vec<u8> = read_frame(&mut r).unwrap();
        assert_eq!(v[0], i as u8);
    }
    writer.join().unwrap();
    server.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(10), "took {:?}", start.elapsed());
}

#[test]
fn connect_without_server_is_not_found() {
    assert_eq!(connect(&unique()).unwrap_err().kind(), ErrorKind::NotFound);
}

#[test]
fn second_listener_is_addr_in_use() {
    let name = unique();
    let _first = PipeListener::bind(&name).unwrap();
    assert_eq!(PipeListener::bind(&name).unwrap_err().kind(), ErrorKind::AddrInUse);
}

#[test]
fn many_clients_each_get_a_connection() {
    let name = unique();
    let mut l = PipeListener::bind(&name).unwrap();
    let server = thread::spawn(move || {
        for _ in 0..5 {
            let (mut r, mut w) = l.accept().unwrap().split();
            thread::spawn(move || {
                let v: u32 = read_frame(&mut r).unwrap();
                write_frame(&mut w, &(v * 2)).unwrap();
            });
        }
    });
    let clients: Vec<_> = (0..5u32)
        .map(|i| {
            let name = name.clone();
            thread::spawn(move || {
                let (mut r, mut w) = connect(&name).unwrap().split();
                write_frame(&mut w, &i).unwrap();
                let v: u32 = read_frame(&mut r).unwrap();
                assert_eq!(v, i * 2);
            })
        })
        .collect();
    for c in clients {
        c.join().unwrap();
    }
    server.join().unwrap();
}

#[test]
fn peer_drop_reads_eof() {
    let name = unique();
    let mut l = PipeListener::bind(&name).unwrap();
    let server = thread::spawn(move || {
        let (mut r, _w) = l.accept().unwrap().split();
        let mut buf = [0u8; 16];
        r.read(&mut buf).unwrap()
    });
    let conn = connect(&name).unwrap();
    thread::sleep(Duration::from_millis(100));
    drop(conn);
    assert_eq!(server.join().unwrap(), 0);
}

#[test]
fn write_to_dropped_peer_is_broken_pipe() {
    let name = unique();
    let mut l = PipeListener::bind(&name).unwrap();
    let server = thread::spawn(move || l.accept().unwrap());
    let (_r, mut w) = connect(&name).unwrap().split();
    drop(server.join().unwrap());
    let err = (0..100).find_map(|_| w.write_all(&[0u8; 1024]).err()).expect("write must fail once the server is gone");
    assert_eq!(err.kind(), ErrorKind::BrokenPipe);
}

#[test]
fn sddl_names_only_current_user() {
    let sid = current_user_sid();
    assert!(sid.starts_with("S-1-5-"), "{sid}");
    let sddl = pipe_sddl();
    assert_eq!(sddl.matches(&sid).count(), 1, "{sddl}");
    // Everyone, Authenticated Users, Builtin Users, Interactive: none may appear.
    for broad in ["WD", "AU", "BU", "IU", "S-1-1-0"] {
        assert!(!sddl.contains(broad), "{sddl} grants {broad}");
    }
}

#[test]
fn client_checks_server_owner() {
    assert_eq!(process_owner_sid(std::process::id()).unwrap(), current_user_sid());
}
