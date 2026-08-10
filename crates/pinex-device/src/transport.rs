//! Byte transport to the pedal, and a PTY-backed stand-in for tests.
//!
//! The pedal is plain USB CDC-ACM, so talking to it is just opening a tty and
//! putting it in raw mode. That is little enough that the interesting question
//! is not how to write it but how to *test* it without a pedal on the desk —
//! hence [`pty_pair`], which hands back a real tty. `TtyTransport::open` cannot
//! tell it from `/dev/tonex`, so the open/termios/read path under test is the
//! same one that will run on the Pi.
//!
//! What a PTY does not reproduce: USB enumeration, device disappearance on
//! unplug, and the pedal's own timing. Those wait for hardware.

use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

/// A bidirectional byte channel to the pedal.
///
/// Exists so the reader thread can be driven by a PTY, a simulator, or a real
/// tty without knowing which.
pub trait Transport: Send {
    /// Read available bytes. May return 0 on an idle timeout — that is normal,
    /// not end-of-stream, and is what drives `FrameAccumulator::flush_stale`.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;
}

/// A tty in raw mode. This is the production transport.
#[derive(Debug)]
pub struct TtyTransport {
    fd: OwnedFd,
}

impl TtyTransport {
    /// Open a tty and put it in raw mode.
    ///
    /// Baud rate is deliberately not set: USB CDC-ACM ignores it.
    pub fn open(path: &Path) -> io::Result<Self> {
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;

        let raw = open(path, OFlag::O_RDWR | OFlag::O_NOCTTY, Mode::empty())?;
        // SAFETY: `open` just handed us this descriptor and nothing else holds
        // it, so OwnedFd takes sole responsibility for closing it.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let this = Self { fd };
        this.set_raw_mode()?;
        Ok(this)
    }

    /// A second handle to the same port.
    ///
    /// The reader thread takes ownership of what it reads, so writes need their
    /// own descriptor. Sharing one behind a lock would let a blocked read delay
    /// a footswitch press.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            fd: self.fd.try_clone()?,
        })
    }

    /// `cfmakeraw` plus a 1 s inter-byte timeout.
    ///
    /// `VMIN = 0` with `VTIME = 10` means a read returns as soon as any bytes
    /// arrive, or empty after a second of silence. The empty return is the
    /// point: it is the tick that lets the reader decide a partial frame is
    /// stale, matching `FrameAccumulator::flush_stale`.
    fn set_raw_mode(&self) -> io::Result<()> {
        use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg, SpecialCharacterIndices};

        let mut attrs = tcgetattr(self.fd.as_fd())?;
        cfmakeraw(&mut attrs);
        attrs.control_chars[SpecialCharacterIndices::VMIN as usize] = 0;
        attrs.control_chars[SpecialCharacterIndices::VTIME as usize] = 10;
        tcsetattr(self.fd.as_fd(), SetArg::TCSANOW, &attrs)?;
        Ok(())
    }
}

impl Transport for TtyTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Ok(nix::unistd::read(self.fd.as_raw_fd(), buf)?)
    }

    fn write_all(&mut self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match nix::unistd::write(self.fd.as_fd(), buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "tty accepted no bytes",
                    ))
                }
                Ok(n) => buf = &buf[n..],
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

/// Open a PTY pair, returning the host end and the path of the device end.
///
/// The device end is a genuine tty that [`TtyTransport::open`] opens by path,
/// exactly as it will open `/dev/tonex`. Write to the host end to play the part
/// of the pedal.
///
/// Available outside tests so integration tests and the simulator can use it.
pub fn pty_pair() -> io::Result<(nix::pty::PtyMaster, PathBuf)> {
    use nix::fcntl::OFlag;
    use nix::pty::{grantpt, posix_openpt, ptsname, unlockpt};

    let master = posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY)?;
    grantpt(&master)?;
    unlockpt(&master)?;

    // SAFETY: ptsname returns a pointer to a static buffer, so concurrent calls
    // race. The lock below makes our own calls exclusive, and the returned
    // String is copied out before the guard drops. There is no portable
    // alternative — ptsname_r is Linux-only and this must build on macOS.
    let name = {
        use std::sync::Mutex;
        static PTSNAME_LOCK: Mutex<()> = Mutex::new(());
        let _guard = PTSNAME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { ptsname(&master) }?
    };

    Ok((master, PathBuf::from(name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A PTY pair is a real tty, so this exercises the same open/termios/read
    /// path a pedal would — only the bytes are ours.
    #[test]
    fn tty_transport_reads_bytes_written_to_the_other_end() {
        let (mut host, device_path) = pty_pair().unwrap();
        let mut transport = TtyTransport::open(&device_path).unwrap();

        host.write_all(b"hello").unwrap();
        host.flush().unwrap();

        let mut buf = [0u8; 16];
        let n = transport.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn tty_transport_writes_reach_the_other_end() {
        use std::io::Read;

        let (mut host, device_path) = pty_pair().unwrap();
        let mut transport = TtyTransport::open(&device_path).unwrap();

        transport.write_all(b"ping").unwrap();

        let mut buf = [0u8; 16];
        let n = host.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");
    }

    /// The idle return is a feature, not an error: it is how the reader learns
    /// that a partial frame has gone stale.
    #[test]
    fn a_silent_port_returns_zero_bytes_rather_than_blocking_forever() {
        let (_host, device_path) = pty_pair().unwrap();
        let mut transport = TtyTransport::open(&device_path).unwrap();

        let start = std::time::Instant::now();
        let mut buf = [0u8; 16];
        let n = transport.read(&mut buf).unwrap();

        assert_eq!(n, 0, "expected an idle timeout, not data");
        assert!(
            start.elapsed() >= std::time::Duration::from_millis(500),
            "VTIME should hold the read for ~1s, waited {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn opening_a_path_that_is_not_a_tty_fails() {
        assert!(TtyTransport::open(Path::new("/nonexistent/tonex")).is_err());
    }
}
