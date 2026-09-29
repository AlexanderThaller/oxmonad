//! The control socket, oxmonad's `xmonadctl`.
//!
//! When `OXMONAD_CONTROL` names a path, oxmonad listens on a Unix socket
//! there. Each line a client writes is a command:
//!
//! * an action in config syntax (`view 2`, `next-layout`, `spawn foot`), queued
//!   exactly like a key press;
//! * `dump-state`, which answers with one line of JSON describing the screens,
//!   windows and focus.
//!
//! Every command is answered with one line after the render sequence that
//! follows it: `ok` for actions, the JSON for `dump-state`, or
//! `error: <reason>`. Try it with
//! `socat - UNIX-CONNECT:$OXMONAD_CONTROL`.

use std::{
    io::{
        self,
        Read,
        Write,
    },
    os::{
        fd::{
            AsFd,
            BorrowedFd,
        },
        unix::net::{
            UnixListener,
            UnixStream,
        },
    },
    path::{
        Path,
        PathBuf,
    },
};

use oxmonad_core::Action;

/// The environment variable naming the control socket path.
pub const ENV: &str = "OXMONAD_CONTROL";

/// A parsed control command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Queue an action.
    Action(Action),
    /// Answer with the state as JSON.
    DumpState,
}

/// Parses one command line.
pub fn parse(line: &str) -> Result<Command, String> {
    match line.trim() {
        "dump-state" => Ok(Command::DumpState),
        other => other
            .parse()
            .map(Command::Action)
            .map_err(|err| format!("{err}")),
    }
}

/// The listening socket; removes its path when dropped.
#[derive(Debug)]
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Listens on `path`, replacing a stale socket.
    pub fn bind(path: &Path) -> io::Result<Self> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            path: path.to_owned(),
        })
    }

    /// Accepts a pending connection.
    pub fn accept(&self) -> io::Result<Client> {
        let (stream, _) = self.listener.accept()?;
        stream.set_nonblocking(true)?;
        Ok(Client {
            stream,
            buffer: Vec::new(),
        })
    }
}

impl AsFd for Listener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.listener.as_fd()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A connected control client.
#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
    buffer: Vec<u8>,
}

/// What reading from a client produced.
#[derive(Debug, Default)]
pub struct Received {
    /// Complete lines.
    pub lines: Vec<String>,
    /// Whether the client hung up.
    pub closed: bool,
}

impl Client {
    /// The socket, for polling.
    #[must_use]
    pub fn socket(&self) -> &UnixStream {
        &self.stream
    }

    /// A handle to answer on.
    pub fn reply_handle(&self) -> io::Result<Reply> {
        Ok(Reply(self.stream.try_clone()?))
    }

    /// Reads what is available.
    pub fn receive(&mut self) -> Received {
        let mut received = Received::default();
        let mut chunk = [0; 4096];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    received.closed = true;
                    break;
                }
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    received.closed = true;
                    break;
                }
            }
        }
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line).trim().to_owned();
            if !line.is_empty() {
                received.lines.push(line);
            }
        }
        received
    }
}

/// Where to send a command's answer.
#[derive(Debug)]
pub struct Reply(UnixStream);

impl Reply {
    /// Sends one answer line; a vanished client is ignored.
    pub fn send(mut self, line: &str) {
        // Answers are small; block until written.
        let _ = self.0.set_nonblocking(false);
        let _ = writeln!(self.0, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse("dump-state"), Ok(Command::DumpState));
        assert_eq!(
            parse(" view 3 "),
            Ok(Command::Action(Action::View("3".into())))
        );
        assert_eq!(
            parse("next-layout"),
            Ok(Command::Action(Action::NextLayout))
        );
        assert!(parse("frobnicate").is_err());
    }

    #[test]
    fn socket_round_trip() {
        let path =
            std::env::temp_dir().join(format!("oxmonad-control-{}.sock", std::process::id()));
        let listener = Listener::bind(&path).expect("bind");
        let mut peer = UnixStream::connect(&path).expect("connect");
        peer.write_all(b"view 2\ndump-st").expect("write");
        let mut client = listener.accept().expect("accept");
        let received = client.receive();
        assert_eq!(received.lines, ["view 2"]);
        assert!(!received.closed);
        peer.write_all(b"ate\n").expect("write");
        let received = client.receive();
        assert_eq!(received.lines, ["dump-state"]);

        client.reply_handle().expect("reply").send("ok");
        let mut answer = [0; 3];
        peer.read_exact(&mut answer).expect("read");
        assert_eq!(&answer, b"ok\n");

        drop(peer);
        assert!(client.receive().closed);
        drop(listener);
        assert!(!path.exists());
    }
}
