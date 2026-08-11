//! Drawing a pedal for a preset.
//!
//! # Where the art comes from
//!
//! All of it is generated here from primitives — rectangles, circles, lines.
//! Nothing is traced from, or meant to pass for, any real product. Commercial
//! pedals have distinctive trade dress and it is not ours to reproduce, so the
//! shapes are deliberately generic: an enclosure, knobs, a footswitch, an LED.
//! What makes one preset look different from another is its **archetype** and
//! its **colour**, not a likeness.
//!
//! # Where the colour comes from
//!
//! The pedal itself. Every Tonex preset carries an RGB value that lights its
//! own ring, and that is what tints the enclosure here — so the panel and the
//! hardware agree about which preset is which, without inventing a palette.
//!
//! # Where the archetype comes from
//!
//! The preset name. Capture packs are named after what they captured, so
//! "MORNING GLORY" is an overdrive and "BENSON PREAMP" is a preamp, and the
//! knob layout can follow. It is a heuristic over words, and it says so: an
//! unrecognised name gets the neutral shape rather than a guess.

use embedded_graphics::mono_font::ascii::FONT_5X8;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{
    Circle, CornerRadii, Line, PrimitiveStyle, Rectangle, RoundedRectangle,
};
use embedded_graphics::text::{Alignment, Text};

/// What kind of box a preset sounds like, inferred from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    Overdrive,
    Distortion,
    Fuzz,
    Boost,
    Preamp,
    /// An amplifier rather than a pedal — drawn as a head, not a stompbox.
    Amp,
    /// Nothing in the name said what it is.
    Unknown,
}

impl Archetype {
    /// A short word for the badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Overdrive => "OD",
            Self::Distortion => "DIST",
            Self::Fuzz => "FUZZ",
            Self::Boost => "BOOST",
            Self::Preamp => "PRE",
            Self::Amp => "AMP",
            Self::Unknown => "",
        }
    }

    /// How many control knobs to draw.
    fn knobs(self) -> usize {
        match self {
            Self::Boost => 1,
            Self::Fuzz => 2,
            Self::Overdrive | Self::Distortion | Self::Preamp => 3,
            Self::Amp => 4,
            Self::Unknown => 2,
        }
    }
}

/// Word families, most specific first.
///
/// Matched against the whole name, so "PROTEIN - BLUE" finds "PROTEIN" and a
/// name containing "BOOST" is a boost even if nothing else matches.
const FAMILIES: &[(&[&str], Archetype)] = &[
    // Preamp before Amp, because "PREAMP" contains "AMP" and a preamp pedal is
    // not an amplifier. Order is the whole mechanism here.
    (&["PREAMP", "PRE-AMP", "PRE AMP"], Archetype::Preamp),
    (
        &[
            "PLEXI", "JCM", "AC30", "DELUXE", "TWIN", "BASSMAN", "RECTI", "COMBO", "STACK", "AMP",
        ],
        Archetype::Amp,
    ),
    (
        &["FUZZ", "MUFF", "TONEBEND", "FACE", "OCTAV"],
        Archetype::Fuzz,
    ),
    (
        &["DIST", "RAT", "METAL", "SHRED", "GRUNGE", "DS-1", "DS1"],
        Archetype::Distortion,
    ),
    (
        &[
            "OVERDRIVE",
            "SCREAMER",
            "TS-",
            "TS9",
            "TS808",
            "BLUES",
            "KING OF TONE",
            "KOT",
            "TIMMY",
            "GLORY",
            "PROTEIN",
            "CENTAUR",
            "KLON",
            "ZENDRIVE",
            "DRIVE",
        ],
        Archetype::Overdrive,
    ),
    (
        &["BOOST", "TILT", "CLEAN BOOST", "TREBLE"],
        Archetype::Boost,
    ),
];

/// Guess what a preset is from its name.
pub fn classify(name: &str) -> Archetype {
    let upper = name.to_ascii_uppercase();
    for (words, archetype) in FAMILIES {
        if words.iter().any(|w| upper.contains(w)) {
            return *archetype;
        }
    }
    Archetype::Unknown
}

/// Strip only the capture-pack prefix, keeping everything that identifies the
/// sound.
///
/// "TF MORNIING GLORY - BRIGHT 1" becomes "MORNIING GLORY - BRIGHT 1". The
/// variant is **not** dropped: "PROTEIN - BLUE 1" and "PROTEIN - GREEN 2" are
/// different sounds, and rendering both as "PROTEIN" made the panel show them
/// identically. The pedal's own spelling is kept, typos included — it is the
/// player's rig, not ours to correct.
pub fn short_name(name: &str) -> &str {
    let trimmed = name.trim();
    // Drop a short pack prefix like "TF". Requiring no vowels is what keeps it
    // from eating real words: "TF" goes, "BIG" in "BIG MUFF" stays.
    let without_prefix = match trimmed.split_once(' ') {
        Some((head, rest))
            if head.len() <= 3
                && !head.is_empty()
                && head.chars().all(|c| c.is_ascii_uppercase())
                && !head
                    .chars()
                    .any(|c| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U')) =>
        {
            rest
        }
        _ => trimmed,
    };
    without_prefix.trim()
}

/// The part to put on a small badge, where the full name will not fit.
///
/// Drops the variant, which the caller is expected to show elsewhere.
pub fn badge_name(name: &str) -> &str {
    match short_name(name).split_once(" - ") {
        Some((head, _)) => head.trim(),
        None => short_name(name),
    }
}

/// Dim a colour towards black, for the slot that is not playing.
pub fn dim(color: Rgb565, numerator: u8, denominator: u8) -> Rgb565 {
    let scale = |v: u8| ((v as u16 * numerator as u16) / denominator as u16) as u8;
    Rgb565::new(scale(color.r()), scale(color.g()), scale(color.b()))
}

/// Convert the pedal's 8-bit RGB into the panel's 5/6/5.
pub fn from_rgb8(rgb: [u8; 3]) -> Rgb565 {
    Rgb565::new(rgb[0] >> 3, rgb[1] >> 2, rgb[2] >> 3)
}

/// How to draw one pedal.
pub struct Pedal<'a> {
    pub name: &'a str,
    pub color: Rgb565,
    pub archetype: Archetype,
    /// Lit: this is what is playing. Unlit boxes are drawn dark.
    pub lit: bool,
}

impl<'a> Pedal<'a> {
    /// Build from a preset name and the pedal's own colour for it.
    pub fn new(name: &'a str, rgb: Option<[u8; 3]>, lit: bool) -> Self {
        Self {
            name,
            color: rgb.map(from_rgb8).unwrap_or(Rgb565::CSS_DIM_GRAY),
            archetype: classify(name),
            lit,
        }
    }
}

/// Draw a stompbox (or amp head) filling `area`.
pub fn draw<D>(target: &mut D, area: Rectangle, pedal: &Pedal<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let body = if pedal.lit {
        pedal.color
    } else {
        // Unplayed boxes recede rather than disappear: still identifiable, but
        // never mistaken for the one making sound.
        dim(pedal.color, 1, 4)
    };
    let ink = if pedal.lit {
        Rgb565::BLACK
    } else {
        Rgb565::CSS_DIM_GRAY
    };
    let trim = if pedal.lit {
        Rgb565::WHITE
    } else {
        dim(Rgb565::WHITE, 1, 3)
    };

    let w = area.size.width as i32;
    let h = area.size.height as i32;
    let x = area.top_left.x;
    let y = area.top_left.y;

    // Enclosure.
    RoundedRectangle::new(area, CornerRadii::new(Size::new(3, 3)))
        .into_styled(PrimitiveStyle::with_fill(body))
        .draw(target)?;
    RoundedRectangle::new(area, CornerRadii::new(Size::new(3, 3)))
        .into_styled(PrimitiveStyle::with_stroke(trim, 1))
        .draw(target)?;

    if pedal.archetype == Archetype::Amp {
        draw_amp_face(target, area, ink, trim)?;
    } else {
        draw_stomp_face(target, area, pedal, ink, trim)?;
    }

    // Name band across the middle, on a dark strip so it reads over any colour.
    let cols_for_band = (((w - 6) / 5).max(1)) as usize;
    let band_lines = wrap(short_name(pedal.name), cols_for_band)
        .len()
        .clamp(1, 2) as u32;
    let band_y = y + h / 2 - 4;
    Rectangle::new(
        Point::new(x + 2, band_y),
        Size::new(w as u32 - 4, 1 + band_lines * 8),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
    .draw(target)?;

    // Wrap rather than truncate: "MORNIING GLORY" cut to "MORNIIN" is not a
    // name a player recognises at a glance.
    let label = badge_name(pedal.name);
    let cols = ((w - 6) / 5).max(1) as usize;
    let text_color = if pedal.lit { Rgb565::WHITE } else { trim };
    for (line, chunk) in wrap(label, cols).iter().take(2).enumerate() {
        Text::with_alignment(
            chunk,
            Point::new(x + w / 2, band_y + 7 + line as i32 * 8),
            MonoTextStyle::new(&FONT_5X8, text_color),
            Alignment::Center,
        )
        .draw(target)?;
    }

    Ok(())
}

/// Knobs along the top, footswitch and LED at the bottom.
fn draw_stomp_face<D>(
    target: &mut D,
    area: Rectangle,
    pedal: &Pedal<'_>,
    ink: Rgb565,
    trim: Rgb565,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let w = area.size.width as i32;
    let h = area.size.height as i32;
    let x = area.top_left.x;
    let y = area.top_left.y;

    // Knobs, evenly spaced across the upper third.
    let count = pedal.archetype.knobs() as i32;
    let knob_r = if w >= 50 { 5 } else { 4 };
    let knob_y = y + 9;
    for index in 0..count {
        let cx = x + (w * (index + 1)) / (count + 1);
        Circle::with_center(Point::new(cx, knob_y), (knob_r * 2) as u32)
            .into_styled(PrimitiveStyle::with_fill(ink))
            .draw(target)?;
        // Pointer, each knob at a slightly different angle so the row does not
        // read as a repeated stamp.
        let tilt = index - count / 2;
        Line::new(
            Point::new(cx, knob_y),
            Point::new(cx + tilt, knob_y - knob_r),
        )
        .into_styled(PrimitiveStyle::with_stroke(trim, 1))
        .draw(target)?;
    }

    // Footswitch and status LED at the foot of the box.
    let switch_y = y + h - 9;
    Circle::with_center(Point::new(x + w / 2, switch_y), 9)
        .into_styled(PrimitiveStyle::with_fill(ink))
        .draw(target)?;
    Circle::with_center(Point::new(x + w / 2, switch_y), 5)
        .into_styled(PrimitiveStyle::with_stroke(trim, 1))
        .draw(target)?;

    let led = if pedal.lit {
        Rgb565::CSS_ORANGE_RED
    } else {
        dim(Rgb565::CSS_ORANGE_RED, 1, 5)
    };
    Circle::with_center(Point::new(x + w / 2, y + h - 20), 4)
        .into_styled(PrimitiveStyle::with_fill(led))
        .draw(target)?;

    Ok(())
}

/// An amp head: a control row over a grille.
fn draw_amp_face<D>(
    target: &mut D,
    area: Rectangle,
    ink: Rgb565,
    trim: Rgb565,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let w = area.size.width as i32;
    let h = area.size.height as i32;
    let x = area.top_left.x;
    let y = area.top_left.y;

    // Control panel strip.
    Rectangle::new(Point::new(x + 3, y + 3), Size::new(w as u32 - 6, 10))
        .into_styled(PrimitiveStyle::with_fill(ink))
        .draw(target)?;
    for index in 0..4 {
        let cx = x + 6 + index * ((w - 12) / 3).max(1);
        Circle::with_center(Point::new(cx, y + 8), 5)
            .into_styled(PrimitiveStyle::with_stroke(trim, 1))
            .draw(target)?;
    }

    // Grille cloth: diagonal hatching, which reads as texture at this size
    // where a fine weave would just alias into noise.
    let grille = Rectangle::new(
        Point::new(x + 3, y + h / 2 + 7),
        Size::new(w as u32 - 6, (h / 2 - 10).max(1) as u32),
    );
    grille
        .into_styled(PrimitiveStyle::with_fill(ink))
        .draw(target)?;
    let mut offset = -h;
    while offset < w {
        Line::new(
            Point::new(x + 3 + offset, y + h - 4),
            Point::new(x + 3 + offset + h / 3, y + h / 2 + 7),
        )
        .into_styled(PrimitiveStyle::with_stroke(trim, 1))
        .draw(target)?;
        offset += 5;
    }
    Ok(())
}

/// Break `text` into lines of at most `cols` characters, on word boundaries
/// where it can and mid-word where a single word is too long.
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if word.len() > cols {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            for chunk in word.as_bytes().chunks(cols) {
                lines.push(String::from_utf8_lossy(chunk).into_owned());
            }
            continue;
        }
        let would_be = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if would_be > cols {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names on the test rig, which are all drive pedals.
    #[test]
    fn names_wrap_on_word_boundaries_and_split_only_when_forced() {
        assert_eq!(super::wrap("TF TILT - 1 ADV", 21), vec!["TF TILT - 1 ADV"]);
        assert_eq!(
            super::wrap("TF MORNIING GLORY - BRIGHT 1", 21),
            vec!["TF MORNIING GLORY -", "BRIGHT 1"]
        );
        // A single word longer than the line is cut, not lost.
        assert_eq!(super::wrap("ABCDEFGHIJ", 4), vec!["ABCD", "EFGH", "IJ"]);
        assert!(super::wrap("", 10).is_empty());
    }

    #[test]
    fn real_preset_names_classify_the_way_a_player_would_expect() {
        assert_eq!(classify("TF BENSON PREAMP - 1"), Archetype::Preamp);
        assert_eq!(
            classify("TF MORNIING GLORY - BRIGHT 1"),
            Archetype::Overdrive
        );
        assert_eq!(classify("TF PROTEIN - BLUE 1"), Archetype::Overdrive);
        assert_eq!(classify("TF PROTEIN - GREEN 2"), Archetype::Overdrive);
        assert_eq!(classify("TF TILT - 1 ADV"), Archetype::Boost);
        assert_eq!(classify("TF TILT - BOOST FULL"), Archetype::Boost);
    }

    #[test]
    fn common_pedal_and_amp_families_are_recognised() {
        assert_eq!(classify("BIG MUFF PI"), Archetype::Fuzz);
        assert_eq!(classify("RAT DISTORTION"), Archetype::Distortion);
        assert_eq!(classify("TUBE SCREAMER TS808"), Archetype::Overdrive);
        assert_eq!(classify("PLEXI 100W"), Archetype::Amp);
        assert_eq!(classify("AC30 TOP BOOST"), Archetype::Amp);
    }

    /// An unrecognised name must get the neutral shape, not a plausible guess.
    /// Showing a fuzz box for something that is not a fuzz is worse than
    /// showing a plain one.
    #[test]
    fn an_unrecognised_name_is_not_guessed_at() {
        assert_eq!(classify("CAPTURE 17"), Archetype::Unknown);
        assert_eq!(classify(""), Archetype::Unknown);
        assert_eq!(Archetype::Unknown.label(), "");
    }

    /// Amp wins over drive when a name contains both, because the amp is the
    /// bigger object: "PLEXI DRIVE" is an amp with drive, not a drive pedal.
    #[test]
    fn the_more_specific_family_wins() {
        assert_eq!(classify("PLEXI DRIVE"), Archetype::Amp);
        assert_eq!(classify("BENSON PREAMP DRIVE"), Archetype::Preamp);
    }

    #[test]
    fn the_badge_name_drops_the_pack_prefix_and_the_variant() {
        // The variant survives: it is what tells two presets apart.
        assert_eq!(
            short_name("TF MORNIING GLORY - BRIGHT 1"),
            "MORNIING GLORY - BRIGHT 1"
        );
        assert_eq!(short_name("TF PROTEIN - BLUE 1"), "PROTEIN - BLUE 1");
        assert_eq!(short_name("TF PROTEIN - GREEN 2"), "PROTEIN - GREEN 2");
        assert_ne!(
            short_name("TF PROTEIN - BLUE 1"),
            short_name("TF PROTEIN - GREEN 2"),
            "two different sounds must not render identically"
        );

        // The badge drops the variant, because it has no room for it.
        assert_eq!(badge_name("TF PROTEIN - BLUE 1"), "PROTEIN");
        assert_eq!(badge_name("TF TILT - 1 ADV"), "TILT");
        // A real word is not a pack prefix: "BIG" has a vowel, "TF" does not.
        assert_eq!(short_name("BIG MUFF"), "BIG MUFF");
        assert_eq!(short_name("JHS MORNING GLORY"), "MORNING GLORY");
        // The pedal's own spelling is kept, typo and all.
        assert!(short_name("TF MORNIING GLORY - BRIGHT 1").contains("MORNIING"));
    }

    #[test]
    fn colour_comes_from_the_pedal_and_dims_when_not_playing() {
        let bright = from_rgb8([255, 63, 0]);
        let dimmed = dim(bright, 1, 4);
        assert!(dimmed.r() < bright.r(), "dimming must darken");
        assert_ne!(dimmed, Rgb565::BLACK, "but not to nothing");

        // No colour reported yet is grey, not black — a box with no colour is
        // still a box.
        let unknown = Pedal::new("X", None, true);
        assert_eq!(unknown.color, Rgb565::CSS_DIM_GRAY);
    }

    #[test]
    fn knob_counts_differ_by_archetype_so_boxes_are_distinguishable() {
        assert_eq!(Archetype::Boost.knobs(), 1);
        assert_eq!(Archetype::Overdrive.knobs(), 3);
        assert_eq!(Archetype::Amp.knobs(), 4);
        assert_ne!(Archetype::Boost.knobs(), Archetype::Overdrive.knobs());
    }
}
