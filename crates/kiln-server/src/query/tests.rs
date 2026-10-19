use super::*;
use kiln_link::RosterEntry;
use uuid::Uuid;

fn info(players: &[&str]) -> Arc<ServerInfo> {
    let info = Arc::new(ServerInfo::new("A Minecraft Server", 20, "world"));
    for (i, p) in players.iter().enumerate() {
        info.roster.write().unwrap().push(RosterEntry { conn: i as u64, uuid: Uuid::from_u128(i as u128), name: (*p).to_owned(), allows_listing: true });
    }
    info
}

fn start(players: &[&str]) -> (QueryServer, UdpSocket, SocketAddr) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port() as i32;
    let facts = Arc::new(LiveFacts { info: info(players), port: 25565, host_ip: "10.1.2.3".into() });
    let s = QueryServer::from_socket(socket, "127.0.0.1", port, facts, Arc::new(|| 0x123456)).unwrap();
    let addr = s.local_addr();
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    (s, client, addr)
}

fn ask(c: &UdpSocket, to: SocketAddr, data: &[u8]) -> Option<Vec<u8>> {
    c.send_to(data, to).unwrap();
    let mut buf = [0u8; 2048];
    c.recv_from(&mut buf).ok().map(|(n, _)| buf[..n].to_vec())
}

const SESSION: [u8; 4] = [0, 0, 0, 1];

fn handshake(c: &UdpSocket, to: SocketAddr) -> i32 {
    let mut req = vec![0xfe, 0xfd, 9];
    req.extend_from_slice(&SESSION);
    let reply = ask(c, to, &req).expect("a challenge");
    assert_eq!(reply[0], 9);
    assert_eq!(&reply[1..5], &SESSION);
    assert_eq!(*reply.last().unwrap(), 0);
    String::from_utf8(reply[5..reply.len() - 1].to_vec()).unwrap().parse().unwrap()
}

fn stat_request(token: i32, full: bool) -> Vec<u8> {
    let mut req = vec![0xfe, 0xfd, 0];
    req.extend_from_slice(&SESSION);
    req.extend_from_slice(&token.to_be_bytes());
    if full {
        req.extend_from_slice(&[0; 4]);
    }
    req
}

#[test]
fn the_challenge_is_a_decimal_token() {
    let (mut s, c, addr) = start(&[]);
    assert_eq!(handshake(&c, addr), 0x123456);
    s.stop();
}

#[test]
fn basic_stat_bytes() {
    let (mut s, c, addr) = start(&["Alice", "Bob"]);
    let token = handshake(&c, addr);
    let reply = ask(&c, addr, &stat_request(token, false)).unwrap();
    let mut want = vec![0];
    want.extend_from_slice(&SESSION);
    want.extend_from_slice(b"A Minecraft Server\0SMP\0world\02\020\0");
    want.extend_from_slice(&25565i16.to_le_bytes());
    want.extend_from_slice(b"10.1.2.3\0");
    assert_eq!(reply, want);
    s.stop();
}

#[test]
fn full_stat_bytes() {
    let (mut s, c, addr) = start(&["Alice", "Bob"]);
    let token = handshake(&c, addr);
    let reply = ask(&c, addr, &stat_request(token, true)).unwrap();
    let mut want = vec![0];
    want.extend_from_slice(&SESSION);
    want.extend_from_slice(b"splitnum\0\x80\0");
    want.extend_from_slice(b"hostname\0A Minecraft Server\0gametype\0SMP\0game_id\0MINECRAFT\0version\026.3\0plugins\0\0map\0world\0");
    want.extend_from_slice(b"numplayers\02\0maxplayers\020\0hostport\025565\0hostip\010.1.2.3\0");
    want.extend_from_slice(b"\0\x01player_\0\0Alice\0Bob\0\0");
    assert_eq!(reply, want);
    s.stop();
}

#[test]
fn no_reply_without_the_token() {
    let (mut s, c, addr) = start(&[]);
    c.set_read_timeout(Some(Duration::from_millis(700))).unwrap();
    // A stat request before any handshake, and one with the wrong token.
    assert!(ask(&c, addr, &stat_request(5, false)).is_none());
    let token = handshake(&c, addr);
    assert!(ask(&c, addr, &stat_request(token + 1, false)).is_none());
    assert!(ask(&c, addr, &[0xfe, 0xfd, 7, 0, 0, 0, 1]).is_none(), "unknown type");
    assert!(ask(&c, addr, &[1, 2, 3, 4]).is_none(), "not a query packet");
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert!(ask(&c, addr, &stat_request(token, false)).is_some());
    s.stop();
}

#[test]
fn the_full_stat_is_repeated_for_five_seconds() {
    let info = info(&["Alice"]);
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let facts = Arc::new(LiveFacts { info: info.clone(), port: 25565, host_ip: "h".into() });
    let mut s = QueryServer::from_socket(socket, "127.0.0.1", 0, facts, Arc::new(|| 1)).unwrap();
    let addr = s.local_addr();
    let c = UdpSocket::bind("127.0.0.1:0").unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let token = handshake(&c, addr);
    let first = ask(&c, addr, &stat_request(token, true)).unwrap();
    info.roster.write().unwrap().push(RosterEntry { conn: 9, uuid: Uuid::nil(), name: "Late".into(), allows_listing: true });
    let second = ask(&c, addr, &stat_request(token, true)).unwrap();
    assert_eq!(first, second, "a player who joined meanwhile is not in the cached full stat");
    // The basic stat is never cached.
    let basic = ask(&c, addr, &stat_request(token, false)).unwrap();
    assert!(String::from_utf8_lossy(&basic).contains("\02\0"));
    s.stop();
}

#[test]
fn a_port_out_of_range_disables_it() {
    let facts = Arc::new(LiveFacts { info: info(&[]), port: 1, host_ip: String::new() });
    assert!(QueryServer::start("", 0, facts.clone(), Arc::new(|| 0)).is_none());
    assert!(QueryServer::start("", 65536, facts, Arc::new(|| 0)).is_none());
}
