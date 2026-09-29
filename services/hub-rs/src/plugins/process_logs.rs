//! Pollable process output with bounded lines and no reader threads.
use anyhow::Result;
use std::sync::Arc;
pub type LogSink = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub(super) struct LogPipe {
    reader: Box<dyn std::io::Read + Send>,
    stream: &'static str,
    sink: LogSink,
    pending: Vec<u8>,
    #[cfg(windows)]
    handle: usize,
}
impl LogPipe {
    #[cfg(unix)]
    pub(super) fn new<R: std::io::Read + std::os::fd::AsRawFd + Send + 'static>(
        reader: R,
        stream: &'static str,
        sink: LogSink,
    ) -> Result<Self> {
        let fd = reader.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self {
            reader: Box::new(reader),
            stream,
            sink,
            pending: vec![],
        })
    }
    #[cfg(windows)]
    pub(super) fn new<R: std::io::Read + std::os::windows::io::AsRawHandle + Send + 'static>(
        reader: R,
        stream: &'static str,
        sink: LogSink,
    ) -> Result<Self> {
        let handle = reader.as_raw_handle() as usize;
        Ok(Self {
            reader: Box::new(reader),
            stream,
            sink,
            pending: vec![],
            handle,
        })
    }
    pub(super) fn drain(&mut self, flush: bool) {
        let mut bytes = [0u8; 8192];
        for _ in 0..8 {
            #[cfg(windows)]
            {
                let mut available = 0;
                let ok = unsafe {
                    windows_sys::Win32::System::Pipes::PeekNamedPipe(
                        self.handle as _,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 || available == 0 {
                    break;
                }
            }
            match self.reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    for byte in &bytes[..n] {
                        if *byte == b'\n' {
                            self.emit()
                        } else {
                            self.pending.push(*byte);
                            if self.pending.len() >= 65536 {
                                self.emit()
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        if flush && !self.pending.is_empty() {
            self.emit()
        }
    }
    fn emit(&mut self) {
        let line = String::from_utf8_lossy(&self.pending);
        (self.sink)(self.stream, line.trim_end_matches('\r'));
        self.pending.clear();
    }
}
