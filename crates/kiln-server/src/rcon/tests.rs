use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn server(runner: Runner) -> (RconServer, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let s = RconServer::from_listener(listener, "hunter2", runner).unwrap();
    let addr = s.local_addr();
    (s, addr)
}

fn echo() -> Runner {
    Arc::new(|c| format!("ran {c}"))
}

fn read_packet(s: &mut TcpStream) -> Option<(i32, i32, String)> {
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut head = [0u8; 4];
    s.read_exact(&mut head).ok()?;
    let len = i32::from_le_bytes(head) as usize;
    let mut rest = vec![0u8; len];
    s.read_exact(&mut rest).ok()?;
    let id = i32::from_le_bytes(rest[0..4].try_into().unwrap());
    let ty = i32::from_le_bytes(rest[4..8].try_into().unwrap());
    Some((id, ty, String::from_utf8_lossy(&rest[8..len - 2]).into_owned()))
}

fn closed(s: &mut TcpStream) -> bool {
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut b = [0u8; 1];
    matches!(s.read(&mut b), Ok(0) | Err(_))
}

#[test]
fn packets_are_little_endian_with_two_nuls() {
    assert_eq!(packet(7, 2, ""), [10, 0, 0, 0, 7, 0, 0, 0, 2, 0, 0, 0, 0, 0]);
    assert_eq!(packet(-1, 2, "a"), [11, 0, 0, 0, 255, 255, 255, 255, 2, 0, 0, 0, b'a', 0, 0]);
}

#[test]
fn login_then_command() {
    let (mut s, addr) = server(echo());
    let mut c = TcpStream::connect(addr).unwrap();
    // A command before the login is refused with id -1.
    c.write_all(&packet(4, TYPE_COMMAND, "list")).unwrap();
    assert_eq!(read_packet(&mut c), Some((-1, TYPE_AUTH_RESPONSE, String::new())));
    // A wrong password too, and it logs a client out that was logged in.
    c.write_all(&packet(5, TYPE_LOGIN, "nope")).unwrap();
    assert_eq!(read_packet(&mut c), Some((-1, TYPE_AUTH_RESPONSE, String::new())));
    c.write_all(&packet(6, TYPE_LOGIN, "hunter2")).unwrap();
    assert_eq!(read_packet(&mut c), Some((6, TYPE_AUTH_RESPONSE, String::new())));
    c.write_all(&packet(7, TYPE_COMMAND, "say hi")).unwrap();
    assert_eq!(read_packet(&mut c), Some((7, TYPE_RESPONSE, "ran say hi".to_owned())));
    c.write_all(&packet(8, TYPE_LOGIN, "")).unwrap();
    assert_eq!(read_packet(&mut c), Some((-1, TYPE_AUTH_RESPONSE, String::new())));
    c.write_all(&packet(9, TYPE_COMMAND, "list")).unwrap();
    assert_eq!(read_packet(&mut c), Some((-1, TYPE_AUTH_RESPONSE, String::new())), "the empty password logged this client out");
    s.stop();
}

#[test]
fn a_long_response_comes_in_pieces_of_4096() {
    let (mut s, addr) = server(Arc::new(|_| "x".repeat(10_000)));
    let mut c = TcpStream::connect(addr).unwrap();
    c.write_all(&packet(1, TYPE_LOGIN, "hunter2")).unwrap();
    read_packet(&mut c).unwrap();
    c.write_all(&packet(2, TYPE_COMMAND, "big")).unwrap();
    let lens: Vec<usize> = (0..3).map(|_| read_packet(&mut c).unwrap()).map(|(id, ty, body)| {
        assert_eq!((id, ty), (2, TYPE_RESPONSE));
        body.len()
    }).collect();
    assert_eq!(lens, [4096, 4096, 1808]);
    // An empty reply is still one packet.
    assert_eq!(response_packets(3, "").len(), 1);
    assert_eq!(response_packets(3, &"y".repeat(4096)).len(), 1);
    assert_eq!(response_packets(3, &"y".repeat(4097)).len(), 2);
    s.stop();
}

#[test]
fn unknown_types_and_bad_packets() {
    let (mut s, addr) = server(echo());
    let mut c = TcpStream::connect(addr).unwrap();
    c.write_all(&packet(1, TYPE_LOGIN, "hunter2")).unwrap();
    read_packet(&mut c).unwrap();
    c.write_all(&packet(2, 255, "what")).unwrap();
    assert_eq!(read_packet(&mut c), Some((2, TYPE_RESPONSE, "Unknown request ff".to_owned())));
    // A length that does not match what arrived closes the connection, as does anything under 10 bytes.
    let mut bad = packet(3, TYPE_COMMAND, "list");
    bad[0] = 99;
    c.write_all(&bad).unwrap();
    assert!(closed(&mut c));
    let mut c = TcpStream::connect(addr).unwrap();
    c.write_all(&[1, 2, 3]).unwrap();
    assert!(closed(&mut c));
    s.stop();
}

#[test]
fn string_reading_stops_at_the_first_nul_or_before_the_last_byte() {
    assert_eq!(string_at(b"abc\0\0", 0, 5), "abc");
    assert_eq!(string_at(b"abcd", 0, 4), "abc", "no NUL: the last byte is not part of it");
    assert_eq!(string_at(b"ab\0cd\0\0", 3, 7), "cd");
    assert_eq!(int_le(&[1, 0, 0, 0], 0, 4), 1);
    assert_eq!(int_le(&[1, 0, 0], 0, 3), 0);
}

#[test]
fn invalid_settings_disable_it() {
    assert!(RconServer::start("", 0, "pw", echo()).is_none(), "port 0");
    assert!(RconServer::start("", 70000, "pw", echo()).is_none(), "port too high");
    assert!(RconServer::start("", 25575, "", echo()).is_none(), "no password");
}
