//! Render every screen to the terminal, for judging layout by eye.
//!
//! `cargo run -p pinex-ui --example gallery`

use pinex_proto::state::Slot;
use pinex_ui::browser::{Connection, Screen, View};
use pinex_ui::{panel, PreviewPanel};

fn main() {
    let connected = Connection::Connected {
        firmware: "1.3.17".into(),
    };
    let offline = Connection::Disconnected;

    let base = |screen| View {
        screen,
        cursor: 4,
        cursor_name: Some("TF PROTEIN - BLUE 1"),
        cursor_color: Some([47, 0, 255]),
        active: Some(0),
        active_name: Some("TF BENSON PREAMP - 1"),
        slot_presets: Some([0, 9, 15]),
        active_slot: Some(Slot::A),
        selected: Slot::B,
        slot_names: [
            Some("TF BENSON PREAMP - 1"),
            Some("TF MORNIING GLORY - BRIGHT 1"),
            Some("TF TILT - 1 ADV"),
        ],
        slot_colors: [Some([255, 63, 0]), Some([47, 0, 255]), Some([0, 255, 0])],
        ..View::stub(&connected)
    };

    for (title, view) in [
        ("A/B — both slots", base(Screen::Slots)),
        ("STOMP — slot C", base(Screen::Stomp)),
        (
            "GAIN",
            View {
                gain_db: -4.5,
                ..base(Screen::Gain)
            },
        ),
        ("NO PEDAL", View::stub(&offline)),
    ] {
        let mut buffer = PreviewPanel::new();
        panel::draw(&mut buffer, &view).unwrap();
        println!("\n=== {title} ===");
        print!("{}", buffer.to_ansi_framed(2));
    }
}
