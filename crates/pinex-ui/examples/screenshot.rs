//! Write each page as a PPM, for looking at properly.
//!
//! `cargo run -p pinex-ui --example screenshot -- /tmp/out`
//!
//! Scaled up, because 128x128 judged at actual size on a desktop tells you
//! nothing about how it reads on the glass.

use std::io::Write;

use embedded_graphics::pixelcolor::RgbColor;
use pinex_proto::state::Slot;
use pinex_ui::browser::{Connection, Screen, View};
use pinex_ui::{panel, PreviewPanel};

const SCALE: usize = 4;

fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/pinex".into());
    let connected = Connection::Connected {
        firmware: "1.3.17".into(),
    };

    for (name, screen) in [
        ("ab", Screen::Slots),
        ("stomp", Screen::Stomp),
        ("gain", Screen::Gain),
    ] {
        let view = View {
            screen,
            cursor: 4,
            cursor_name: Some("TF PROTEIN - BLUE 1"),
            cursor_color: Some([47, 0, 255]),
            active: Some(5),
            active_name: Some("TF PROTEIN - BLUE 3"),
            slot_presets: Some([5, 1, 15]),
            active_slot: Some(Slot::A),
            selected: Slot::B,
            slot_names: [
                Some("TF PROTEIN - BLUE 3"),
                Some("TF MORNIING GLORY - BRIGHT 1"),
                Some("TF TILT - 1 ADV"),
            ],
            slot_colors: [Some([255, 0, 0]), Some([47, 0, 255]), Some([0, 255, 0])],
            ..View::stub(&connected)
        };

        let mut buffer = PreviewPanel::new();
        panel::draw(&mut buffer, &view).unwrap();

        let path = format!("{out}-{name}.ppm");
        let mut file = std::fs::File::create(&path)?;
        let (w, h) = (panel::WIDTH as usize, panel::HEIGHT as usize);
        write!(file, "P6\n{} {}\n255\n", w * SCALE, h * SCALE)?;
        for y in 0..h * SCALE {
            for x in 0..w * SCALE {
                let p = buffer.pixel((x / SCALE) as u32, (y / SCALE) as u32);
                // Expand 5/6/5 back to 8 bits a channel.
                file.write_all(&[
                    (p.r() as u16 * 255 / 31) as u8,
                    (p.g() as u16 * 255 / 63) as u8,
                    (p.b() as u16 * 255 / 31) as u8,
                ])?;
            }
        }
        println!("{path}");
    }
    Ok(())
}
