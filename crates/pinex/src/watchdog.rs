//! Telling systemd the loop is still turning.
//!
//! `Restart=always` only helps when the process *exits*. A process that is
//! alive but stuck is invisible to it — and that is not a hypothetical failure
//! here. A write to a pedal that had stopped draining its endpoint blocked the
//! main thread forever: every button dead, the panel holding its last frame,
//! the debug page still serving from its own thread. Nothing exited, so nothing
//! restarted, and it looked for all the world like broken hardware.
//!
//! That specific cause is fixed. This catches the class. The app sends
//! `WATCHDOG=1` once per loop iteration; if the loop stops turning, systemd
//! stops hearing from it and restarts the service.
//!
//! # Why it pings unconditionally
//!
//! The ping means "the loop is turning", not "everything is well". A pedal that
//! is unplugged, a panel that failed to open, a parse error — all of those are
//! states the app is *designed* to sit in, and a watchdog that fired on them
//! would reboot a working controller for showing NO PEDAL. The only thing it
//! asserts is liveness, which is the only thing systemd can act on sensibly.
//!
//! Speaks the `sd_notify` protocol directly — a datagram to the socket named by
//! `NOTIFY_SOCKET` — rather than linking libsystemd, which would be a
//! cross-compiled C dependency for one line of wire format.

use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

/// A handle to systemd's notification socket, or nothing at all.
///
/// Absent whenever `NOTIFY_SOCKET` is unset, which is every run that is not
/// under systemd: a laptop, a test, `panel_sim`. Pinging is then a no-op, so
/// callers never have to ask which case they are in.
#[derive(Debug)]
pub struct Watchdog {
    socket: Option<(UnixDatagram, Target)>,
}

#[derive(Debug)]
enum Target {
    Path(PathBuf),
    /// Linux abstract namespace, which systemd writes with a leading `@`.
    #[cfg(target_os = "linux")]
    Abstract(String),
}

impl Watchdog {
    /// Read `NOTIFY_SOCKET`. Never fails: a watchdog that refuses to start
    /// must not be the reason a pedal controller does not.
    pub fn from_env() -> Self {
        let Some(name) = std::env::var_os("NOTIFY_SOCKET") else {
            return Self { socket: None };
        };
        Self::to(&name.to_string_lossy())
    }

    /// Aim at a specific socket name, as `NOTIFY_SOCKET` would give it.
    pub fn to(name: &str) -> Self {
        if name.is_empty() {
            return Self { socket: None };
        }
        let Ok(socket) = UnixDatagram::unbound() else {
            return Self { socket: None };
        };

        // systemd uses a leading '@' for the abstract namespace, where the
        // first byte of the address is NUL rather than a path on disk.
        let target = match name.strip_prefix('@') {
            #[cfg(target_os = "linux")]
            Some(rest) => Target::Abstract(rest.to_string()),
            #[cfg(not(target_os = "linux"))]
            Some(_) => return Self { socket: None },
            None => Target::Path(PathBuf::from(name)),
        };

        Self {
            socket: Some((socket, target)),
        }
    }

    /// Whether anything is actually listening. Only for diagnostics.
    pub fn is_active(&self) -> bool {
        self.socket.is_some()
    }

    /// Report that the loop turned.
    ///
    /// Errors are dropped on purpose. A failed datagram means systemd is not
    /// listening, and the correct response to that is to carry on being a pedal
    /// controller — the watchdog exists to protect the loop, not to stop it.
    pub fn ping(&self) {
        let Some((socket, target)) = &self.socket else {
            return;
        };
        const MESSAGE: &[u8] = b"WATCHDOG=1";
        let _ = match target {
            Target::Path(path) => socket.send_to(MESSAGE, path),
            #[cfg(target_os = "linux")]
            Target::Abstract(name) => {
                use std::os::linux::net::SocketAddrExt;
                match std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()) {
                    Ok(addr) => socket.send_to_addr(MESSAGE, &addr),
                    Err(e) => Err(e),
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The wire format, checked against a socket we own — so the thing systemd
    /// would receive is asserted without systemd being involved.
    #[test]
    fn a_ping_sends_the_sd_notify_keepalive() {
        let dir = std::env::temp_dir().join(format!("pinex-wd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify");
        let _ = std::fs::remove_file(&path);

        let listener = UnixDatagram::bind(&path).unwrap();
        listener
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let watchdog = Watchdog::to(path.to_str().unwrap());
        assert!(watchdog.is_active());
        watchdog.ping();

        let mut buf = [0u8; 64];
        let n = listener
            .recv(&mut buf)
            .expect("systemd would have received");
        assert_eq!(
            &buf[..n],
            b"WATCHDOG=1",
            "systemd only accepts this exact keepalive"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// Off systemd there is no socket, and pinging must be a silent no-op
    /// rather than an error every caller has to handle.
    #[test]
    fn without_a_notify_socket_pinging_does_nothing_and_says_so() {
        let watchdog = Watchdog::to("");
        assert!(!watchdog.is_active());
        watchdog.ping();
    }

    /// A socket that has gone away must not take the controller down with it.
    #[test]
    fn a_dead_socket_does_not_stop_the_program() {
        let watchdog = Watchdog::to("/nonexistent/pinex/notify");
        for _ in 0..100 {
            watchdog.ping();
        }
    }
}
