//! Debug web UI — M1's "see the truth" surface.
//!
//! Shows connection status, firmware, the active preset, all twenty names, and
//! recent frames with their raw hex. **Parse failures are first-class here, not
//! log lines:** when a Tonex firmware update breaks our parsing, this page is
//! how it gets discovered, with the offending bytes in hand.
//!
//! The design doc specified `tiny_http`. This uses `std::net` instead — the
//! server is a blocking accept loop serving one static page, which is about
//! sixty lines either way, and a dependency-free build is worth more on a Pi
//! than the convenience. Same thread model: no async runtime.
//!
//! [`render_page`] is a pure function of a [`Snapshot`], so what the page says
//! is testable without binding a socket.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How many recent frames to keep for the page.
pub const FRAME_LOG_LEN: usize = 20;

/// One decoded frame, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRecord {
    pub summary: String,
    pub raw_hex: String,
    pub is_error: bool,
}

/// Everything the page renders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub connected: bool,
    pub firmware: Option<String>,
    pub active_preset: Option<u8>,
    pub active_name: Option<String>,
    pub cursor: u8,
    pub names: Vec<Option<String>>,
    /// Per-preset RGB, mirroring the colour the pedal lights for each preset.
    pub colors: Vec<[u8; 3]>,
    pub frames: Vec<FrameRecord>,
}

impl Snapshot {
    /// Record a frame, keeping only the most recent [`FRAME_LOG_LEN`].
    pub fn push_frame(&mut self, record: FrameRecord) {
        self.frames.push(record);
        if self.frames.len() > FRAME_LOG_LEN {
            let excess = self.frames.len() - FRAME_LOG_LEN;
            self.frames.drain(..excess);
        }
    }
}

/// Format bytes as lowercase hex, for the raw column.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render the debug page. Pure, so the content is testable on its own.
pub fn render_page(snapshot: &Snapshot) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html><meta charset=utf-8><title>Pinex debug</title>");
    html.push_str(
        "<style>body{font:14px system-ui;margin:2rem;max-width:60rem}\
         table{border-collapse:collapse}td,th{padding:.2rem .6rem;text-align:left}\
         .err{color:#b00}.hex{font-family:ui-monospace,monospace;font-size:12px}\
         .now{font-weight:700}</style>",
    );

    html.push_str("<h1>Pinex</h1>");
    if snapshot.connected {
        html.push_str(&format!(
            "<p>Connected — firmware {}</p>",
            escape(snapshot.firmware.as_deref().unwrap_or("unknown"))
        ));
    } else {
        html.push_str("<p class=err><strong>NO PEDAL</strong></p>");
    }

    html.push_str("<p class=now>Now playing: ");
    match (snapshot.active_preset, snapshot.active_name.as_deref()) {
        (Some(i), Some(name)) => html.push_str(&format!("{:02} {}", i + 1, escape(name))),
        (Some(i), None) => html.push_str(&format!("{:02}", i + 1)),
        (None, _) => html.push('—'),
    }
    html.push_str("</p>");

    html.push_str("<h2>Presets</h2><table>");
    for (i, name) in snapshot.names.iter().enumerate() {
        let marker = if Some(i as u8) == snapshot.active_preset {
            "&#9654;"
        } else {
            ""
        };
        // The swatch is the pedal's own colour for this preset, not ours.
        let swatch = match snapshot.colors.get(i) {
            Some([r, g, b]) => format!(
                "<span style=\"display:inline-block;width:.8rem;height:.8rem;\
                 border-radius:2px;background:rgb({r},{g},{b})\"></span>"
            ),
            None => String::new(),
        };
        html.push_str(&format!(
            "<tr><td>{marker}</td><td>{swatch}</td><td>{:02}</td><td>{}</td></tr>",
            i + 1,
            escape(name.as_deref().unwrap_or("—"))
        ));
    }
    html.push_str("</table>");

    html.push_str("<h2>Recent frames</h2><table>");
    for frame in snapshot.frames.iter().rev() {
        let class = if frame.is_error { " class=err" } else { "" };
        html.push_str(&format!(
            "<tr{class}><td>{}</td><td class=hex>{}</td></tr>",
            escape(&frame.summary),
            escape(&frame.raw_hex)
        ));
    }
    html.push_str("</table>");
    html
}

/// A blocking HTTP server serving [`render_page`] on every request.
pub struct DebugServer {
    addr: SocketAddr,
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
}

impl DebugServer {
    /// Bind and start serving. Port 0 asks the OS for a free port.
    pub fn start(port: u16, snapshot: Arc<Mutex<Snapshot>>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));

        let (thread_snapshot, thread_stop) = (Arc::clone(&snapshot), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("pinex-web".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if thread_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    let _ = serve(stream, &thread_snapshot);
                }
            })?;

        Ok(Self {
            addr,
            snapshot,
            stop,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn snapshot(&self) -> &Arc<Mutex<Snapshot>> {
        &self.snapshot
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop so the thread can notice the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

fn serve(mut stream: TcpStream, snapshot: &Arc<Mutex<Snapshot>>) -> std::io::Result<()> {
    // Read the request line and discard the rest; every path serves the page.
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    let body = {
        let guard = snapshot.lock().unwrap_or_else(|e| e.into_inner());
        render_page(&guard)
    };

    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn sample() -> Snapshot {
        Snapshot {
            connected: true,
            firmware: Some("1.3.17".into()),
            active_preset: Some(1),
            active_name: Some("TF MORNIING GLORY - BRIGHT 1".into()),
            cursor: 0,
            names: vec![
                Some("TF BENSON PREAMP - 1".into()),
                Some("TF MORNIING GLORY - BRIGHT 1".into()),
            ],
            colors: vec![[255, 63, 0], [47, 0, 255]],
            frames: vec![FrameRecord {
                summary: "Hello".into(),
                raw_hex: "7e b9 03".into(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn a_disconnected_pedal_is_unmistakable_on_the_page() {
        let page = render_page(&Snapshot::default());
        assert!(page.contains("NO PEDAL"));
        assert!(!page.contains("Connected"));
    }

    #[test]
    fn the_page_shows_firmware_the_active_preset_and_the_names() {
        let page = render_page(&sample());
        assert!(page.contains("firmware 1.3.17"));
        assert!(page.contains("02 TF MORNIING GLORY - BRIGHT 1"));
        assert!(page.contains("TF BENSON PREAMP - 1"));
    }

    /// The reason this page exists: a broken parse must be visible with bytes.
    /// The swatch must be the pedal's own colour, so the page and the hardware
    /// cannot disagree about which preset is which.
    #[test]
    fn preset_colours_render_as_swatches() {
        let page = render_page(&sample());
        assert!(page.contains("rgb(255,63,0)"), "preset 1 swatch missing");
        assert!(page.contains("rgb(47,0,255)"), "preset 2 swatch missing");
    }

    /// Colours can be absent (no state yet). The page must still render.
    #[test]
    fn a_page_without_colours_still_renders_its_names() {
        let snapshot = Snapshot {
            names: vec![Some("TF TILT - 1 ADV".into())],
            ..Default::default()
        };
        let page = render_page(&snapshot);
        assert!(page.contains("TF TILT - 1 ADV"));
        assert!(!page.contains("rgb("));
    }

    #[test]
    fn parse_errors_appear_with_their_raw_bytes() {
        let mut snapshot = sample();
        snapshot.push_frame(FrameRecord {
            summary: "unrecognised message type 0x9999".into(),
            raw_hex: "de ad be ef".into(),
            is_error: true,
        });

        let page = render_page(&snapshot);
        assert!(page.contains("unrecognised message type 0x9999"));
        assert!(page.contains("de ad be ef"), "raw bytes must be shown");
        assert!(page.contains("class=err"), "and marked as an error");
    }

    #[test]
    fn the_frame_log_keeps_only_the_most_recent_entries() {
        let mut snapshot = Snapshot::default();
        for i in 0..FRAME_LOG_LEN + 5 {
            snapshot.push_frame(FrameRecord {
                summary: format!("frame {i}"),
                raw_hex: String::new(),
                is_error: false,
            });
        }
        assert_eq!(snapshot.frames.len(), FRAME_LOG_LEN);
        assert_eq!(snapshot.frames[0].summary, "frame 5", "oldest dropped");
    }

    /// Preset names come from the pedal and are rendered into HTML, so they must
    /// be escaped — untrusted device input should never be able to inject markup.
    #[test]
    fn device_supplied_text_is_escaped() {
        let snapshot = Snapshot {
            names: vec![Some("<script>alert(1)</script>".into())],
            ..Default::default()
        };
        let page = render_page(&snapshot);
        assert!(!page.contains("<script>"), "markup must not survive");
        assert!(page.contains("&lt;script&gt;"));
    }

    #[test]
    fn the_server_answers_an_http_request_with_the_page() {
        let snapshot = Arc::new(Mutex::new(sample()));
        let server = DebugServer::start(0, Arc::clone(&snapshot)).unwrap();

        let mut stream = TcpStream::connect(server.addr()).unwrap();
        write!(stream, "GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("firmware 1.3.17"));
        assert!(response.contains("TF BENSON PREAMP - 1"));
    }
}
