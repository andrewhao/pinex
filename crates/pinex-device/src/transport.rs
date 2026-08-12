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
use std::time::{Duration, Instant};

/// How long a write may take before we give up on it.
///
/// The pedal can stop servicing its USB endpoint — a burst of requests is known
/// to leave it silent until it is power-cycled. Its tty buffer then fills and
/// never drains, and an unbounded `write` blocks forever. Writes happen on the
/// app's main thread, so that single blocked call takes the footswitch, the
/// panel and the reconnect logic down with it: every input goes dead while the
/// debug page carries on serving a snapshot frozen at the last good frame.
///
/// Failing the write is strictly better. The caller surfaces the error and the
/// loop keeps running, so the panel still says what it knows and the buttons
/// still respond.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

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

/// The error for a write the far end would not take.
///
/// Names the bytes left over, because "how much of the frame got out" decides
/// whether the pedal saw a partial message.
fn stalled(remaining: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "the port accepted nothing for {}s with {remaining} bytes left — \
             the pedal has stopped draining its endpoint",
            WRITE_TIMEOUT.as_secs()
        ),
    )
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

    /// A duplicate of this descriptor, sharing its open file description.
    ///
    /// Shares status flags, so it cannot be made non-blocking independently.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            fd: self.fd.try_clone()?,
        })
    }

    /// A second handle to the same port, for writing.
    ///
    /// The reader thread takes ownership of what it reads, so writes need their
    /// own descriptor. Sharing one behind a lock would let a blocked read delay
    /// a footswitch press.
    ///
    /// Deliberately a fresh `open` rather than a `dup` of the reader's
    /// descriptor. `O_NONBLOCK` is a property of the open file description, so
    /// a duplicate would force it on the reader too — and a non-blocking read
    /// returns `EAGAIN` instantly instead of honouring `VMIN`/`VTIME`, turning
    /// the reader's one-second idle tick into a hot spin. Two descriptions keep
    /// the two directions independent: blocking reads, bounded writes.
    pub fn open_writer(path: &Path) -> io::Result<Self> {
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;

        let raw = open(
            path,
            OFlag::O_RDWR | OFlag::O_NOCTTY | OFlag::O_NONBLOCK,
            Mode::empty(),
        )?;
        // SAFETY: as in `open` — this descriptor is ours alone.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // Line discipline belongs to the tty, not the descriptor, so the
        // reader's raw mode already applies. Setting it again is idempotent and
        // means a writer opened first is still correct.
        let this = Self { fd };
        this.set_raw_mode()?;
        Ok(this)
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

    /// Write everything, or give up after [`WRITE_TIMEOUT`].
    ///
    /// Only correct on a descriptor from [`TtyTransport::open_writer`], which is
    /// non-blocking; on a blocking one the kernel never returns `EAGAIN` and
    /// there is nothing to time out. See [`WRITE_TIMEOUT`] for why an unbounded
    /// write here is fatal to the whole controller.
    ///
    /// The deadline covers the *whole* buffer rather than each pass: a port
    /// that accepts one byte per second is just as stuck, and resetting the
    /// clock on every byte would never notice.
    ///
    /// Retries on `EAGAIN` rather than waiting on `poll`. macOS reports a tty
    /// writable whether or not it is, so a `poll`-then-write would sail past
    /// the wait and block in `write` anyway — which is precisely the hang this
    /// bounds. Asking the kernel to write and believing its answer is the
    /// portable version.
    fn write_all(&mut self, mut buf: &[u8]) -> io::Result<()> {
        /// Long enough not to spin a core, short against a 2 s deadline.
        const RETRY_AFTER: Duration = Duration::from_millis(2);

        let deadline = Instant::now() + WRITE_TIMEOUT;
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
                // The buffer is full. On a healthy port it drains in
                // microseconds; on a stalled one it never does.
                Err(nix::errno::Errno::EAGAIN) => {
                    if Instant::now() >= deadline {
                        return Err(stalled(buf.len()));
                    }
                    std::thread::sleep(RETRY_AFTER);
                }
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

    /// The hang that takes the whole controller down.
    ///
    /// A pedal that has stopped draining its endpoint leaves the tty buffer
    /// full. Writes run on the app's main thread, so an unbounded `write` there
    /// freezes input and rendering too — the panel keeps its last picture and
    /// every button goes dead, which reads as broken hardware.
    ///
    /// The write must fail instead. Run on a thread so that a regression fails
    /// this test rather than hanging the suite forever.
    #[test]
    fn a_write_the_far_end_never_drains_fails_instead_of_blocking_forever() {
        // Held for the whole test: dropping the host end would make the device
        // end return EIO, which would pass for the wrong reason.
        let (_host, path) = pty_pair().unwrap();
        // The writer descriptor, which is the one `Pedal` actually writes on.
        let mut transport = TtyTransport::open_writer(&path).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Far more than any tty buffer, and nothing is reading the host end.
            let _ = tx.send(transport.write_all(&vec![0u8; 1 << 20]));
        });

        match rx.recv_timeout(WRITE_TIMEOUT * 4) {
            Ok(Err(e)) => assert_eq!(
                e.kind(),
                io::ErrorKind::TimedOut,
                "a stalled write should report a timeout, got {e:?}"
            ),
            Ok(Ok(())) => panic!("the write claimed to succeed with nothing draining it"),
            Err(_) => panic!(
                "write_all never returned — it blocked forever, which on the Pi \
                 freezes the app loop and kills every button"
            ),
        }
    }

    /// ...but an ordinary write to a far end that *is* draining must still go
    /// through untouched. A timeout that also breaks the normal path is no fix.
    #[test]
    fn a_normal_write_still_succeeds() {
        let (host, path) = pty_pair().unwrap();
        let mut transport = TtyTransport::open_writer(&path).unwrap();

        // Drain the host end continuously, as a healthy pedal does.
        let drain = std::thread::spawn(move || {
            let mut host = host;
            let mut sink = [0u8; 4096];
            let mut total = 0;
            while total < 64 * 1024 {
                match std::io::Read::read(&mut host, &mut sink) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => total += n,
                }
            }
            total
        });

        transport
            .write_all(&vec![0xA5; 64 * 1024])
            .expect("a draining far end must accept the whole buffer");
        assert!(drain.join().unwrap() > 0, "the bytes should have arrived");
    }

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
