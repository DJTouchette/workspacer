//! One running app per data folder. A second launch asks the first to come
//! forward and exits, instead of failing on the session database's lock.
//!
//! The owner listens on a loopback port and records it, with a random token,
//! in `instance.json` inside the data folder. Only a process that can read
//! that file can ask, and the only thing it can ask for is the window.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

const FILE: &str = "instance.json";
const TIMEOUT: Duration = Duration::from_millis(750);

pub enum Claim {
    /// This process owns the data folder. Each message is another launch
    /// asking for the window.
    Owner(async_channel::Receiver<()>),
    /// Another copy already runs and has been asked to come forward.
    Running,
}

fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from the OS for every instance.
    (0..2)
        .map(|_| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u32(std::process::id());
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

fn ask(directory: &Path) -> Option<()> {
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join(FILE)).ok()?).ok()?;
    let port = u16::try_from(record["port"].as_u64()?).ok()?;
    let token = record["token"].as_str()?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    writeln!(stream, "{token} activate").ok()?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    (reply.trim() == "ok").then_some(())
}

/// Ask a running copy for its window, or become the owner. A stale record
/// (the owner crashed, or its port now belongs to something else) is
/// replaced. Failing to listen is not fatal: the app starts unguarded.
pub fn claim(directory: &Path) -> Claim {
    if ask(directory).is_some() {
        return Claim::Running;
    }
    let (asked, requests) = async_channel::bounded(4);
    let listening = (|| -> std::io::Result<()> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let token = token();
        let record = serde_json::json!({
            "port": listener.local_addr()?.port(),
            "token": token,
            "pid": std::process::id(),
        });
        std::fs::create_dir_all(directory)?;
        let staged = directory.join(format!("{FILE}.{}", std::process::id()));
        std::fs::write(&staged, record.to_string())?;
        std::fs::rename(&staged, directory.join(FILE))?;
        std::thread::Builder::new()
            .name("wks-instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let _ = answer(stream, &token, &asked);
                }
            })?;
        Ok(())
    })();
    if let Err(error) = listening {
        eprintln!("Could not guard against a second copy of the app: {error}");
    }
    Claim::Owner(requests)
}

fn answer(
    mut stream: TcpStream,
    token: &str,
    asked: &async_channel::Sender<()>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = String::new();
    // A request is one short line; anything longer is not one.
    BufReader::new((&stream).take(256)).read_line(&mut line)?;
    if line.trim() == format!("{token} activate") {
        let _ = asked.try_send(());
        stream.write_all(b"ok\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wks-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_launch_asks_the_first_for_its_window() {
        let dir = directory("second");
        let Claim::Owner(requests) = claim(&dir) else {
            panic!("the first launch owns the folder")
        };
        assert!(matches!(claim(&dir), Claim::Running));
        assert!(requests.recv_blocking().is_ok());
        assert!(requests.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_stale_or_forged_record_does_not_stop_a_launch() {
        let dir = directory("stale");
        // A port nothing listens on any more.
        let gone = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = gone.local_addr().unwrap().port();
        drop(gone);
        std::fs::write(
            dir.join(FILE),
            serde_json::json!({"port":port,"token":"x"}).to_string(),
        )
        .unwrap();
        let Claim::Owner(requests) = claim(&dir) else {
            panic!("a dead owner is replaced")
        };
        // The wrong token is not a request, and gets no "ok".
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(FILE)).unwrap()).unwrap();
        let mut stream =
            TcpStream::connect((Ipv4Addr::LOCALHOST, record["port"].as_u64().unwrap() as u16))
                .unwrap();
        writeln!(stream, "wrong activate").unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert!(reply.is_empty());
        assert!(requests.is_empty());
        std::fs::write(dir.join(FILE), b"not json").unwrap();
        assert!(ask(&dir).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
