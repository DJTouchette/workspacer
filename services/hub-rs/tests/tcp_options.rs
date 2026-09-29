//! Kernel-level configuration proof for the Linux runner used by the latency
//! guard. This does not assert a timing budget or infer a production speedup.
#![cfg(target_os = "linux")]
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use workspacer_hub::{Hub, Options, Status};

fn proc_address(address: SocketAddr) -> String {
    let SocketAddr::V4(address) = address else {
        panic!("fixture must use IPv4 loopback");
    };
    format!(
        "{:08X}:{:04X}",
        u32::from_ne_bytes(address.ip().octets()),
        address.port()
    )
}
fn accepted_nodelay(local: SocketAddr, remote: SocketAddr) -> bool {
    let (local, remote) = (proc_address(local), proc_address(remote));
    let table = std::fs::read_to_string("/proc/net/tcp").unwrap();
    let inode = table
        .lines()
        .skip(1)
        .find_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.len() > 9 && fields[1] == local && fields[2] == remote && fields[3] == "01")
                .then(|| fields[9].to_owned())
        })
        .expect("live accepted fixture socket must have an established kernel row");
    let target = format!("socket:[{inode}]");
    let fd = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|entry| {
            (std::fs::read_link(entry.path())
                .ok()?
                .to_string_lossy()
                .as_ref()
                == target.as_str())
            .then(|| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .parse::<libc::c_int>()
                    .unwrap()
            })
        })
        .expect("accepted fixture socket must belong to this process");
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&value) as libc::socklen_t;
    // SAFETY: getsockopt only reads the live descriptor and writes these valid
    // sized scalar buffers. The connected client and owned server stay alive
    // throughout this call; this neither closes nor takes ownership of the fd.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            (&mut value as *mut libc::c_int).cast(),
            &mut size,
        )
    };
    assert_eq!(result, 0, "getsockopt: {}", std::io::Error::last_os_error());
    assert_eq!(size as usize, std::mem::size_of_val(&value));
    value != 0
}
async fn assert_listener_nodelay(address: SocketAddr) {
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let peer = socket.local_addr().unwrap();
    assert_ne!(peer, address);
    assert_eq!(socket.peer_addr().unwrap(), address);
    // Deliberately opposite to the required server option. Looking up the
    // reversed endpoint/client fd must fail the assertion below.
    socket.set_nodelay(false).unwrap();
    assert!(!socket.nodelay().unwrap());
    socket
        .write_all(
            format!("GET /health HTTP/1.1\r\nHost: {address}\r\nConnection: keep-alive\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut status = String::new();
    {
        let mut reader = tokio::io::BufReader::new(&mut socket);
        tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut status))
            .await
            .unwrap()
            .unwrap();
    }
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert!(
        accepted_nodelay(address, peer),
        "accepted listener {address} left Nagle enabled"
    );
}
#[tokio::test]
async fn kernel_probe_distinguishes_enabled_and_disabled_nagle_on_the_accepted_socket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(address).await.unwrap();
    let (accepted, peer) = listener.accept().await.unwrap();
    assert_eq!(peer, client.local_addr().unwrap());
    assert_eq!(accepted.local_addr().unwrap(), address);
    client.set_nodelay(false).unwrap();
    assert!(!client.nodelay().unwrap());
    accepted.set_nodelay(false).unwrap();
    assert!(!accepted_nodelay(address, peer));
    accepted.set_nodelay(true).unwrap();
    assert!(accepted_nodelay(address, peer));
}
#[tokio::test]
async fn real_hub_and_mcp_accepted_sockets_disable_nagle() {
    let mut options = Options::default();
    options.control_plane_only = true;
    options.token = "tcp-options-fixture".into();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    let hub = Hub::start(options).unwrap();
    let bus = hub.ready().await.unwrap().unwrap();
    let mcp = match *hub.handle().status().borrow() {
        Status::Ready {
            mcp_address: Some(address),
            ..
        } => address,
        _ => panic!("MCP listener must be ready"),
    };
    assert_listener_nodelay(bus).await;
    assert_listener_nodelay(mcp).await;
    hub.shutdown().unwrap();
}
