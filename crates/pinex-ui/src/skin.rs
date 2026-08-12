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

/// The window of `text` visible at `tick`, scrolling back and forth.
///
/// Text that fits is returned whole and never moves — motion on a stage display
/// is a cost, and paying it for a name that already fits is pure distraction.
///
/// Longer text ping-pongs rather than wrapping around: a name that scrolls off
/// one edge and reappears at the other reads as two different names at a
/// glance. Back-and-forth keeps the word order intelligible, and it pauses at
/// each end so the beginning and the end can both actually be read.
///
/// `tick` is a frame counter; the caller decides how fast frames come.
pub fn marquee(text: &str, cols: usize, tick: u32) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= cols || cols == 0 {
        return text.to_string();
    }

    let travel = chars.len() - cols;
    // Frames spent holding still at each end.
    const PAUSE: u32 = 8;
    let leg = travel as u32 + PAUSE;
    let cycle = leg * 2;
    let phase = tick % cycle;

    let offset = if phase < PAUSE {
        0
    } else if phase < leg {
        (phase - PAUSE).min(travel as u32)
    } else if phase < leg + PAUSE {
        travel as u32
    } else {
        travel as u32 - (phase - leg - PAUSE).min(travel as u32)
    };

    chars[offset as usize..(offset as usize + cols).min(chars.len())]
        .iter()
        .collect()
}

/// Whether `text` would scroll at `cols`, so a caller can decide to keep
/// redrawing.
pub fn scrolls(text: &str, cols: usize) -> bool {
    text.chars().count() > cols && cols > 0
}

/// Blend two colours, `num`/`den` of the way from `a` to `b`.
fn mix(a: Rgb565, b: Rgb565, num: u32, den: u32) -> Rgb565 {
    let lerp = |x: u8, y: u8| ((x as u32 * (den - num) + y as u32 * num) / den) as u8;
    Rgb565::new(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// Toward white, for lit edges and highlights.
fn lighten(c: Rgb565, num: u32, den: u32) -> Rgb565 {
    mix(c, Rgb565::WHITE, num, den)
}

/// Toward black, for shaded edges and shadows.
fn darken(c: Rgb565, num: u32, den: u32) -> Rgb565 {
    mix(c, Rgb565::BLACK, num, den)
}

/// Fill `area` with a top-to-bottom gradient, one horizontal line per row.
///
/// A flat fill reads as a coloured rectangle; a gradient reads as a moulded
/// object catching light from above, which is the whole difference between a
/// diagram of a pedal and something that looks like it has a lid.
fn gradient<D>(target: &mut D, area: Rectangle, top: Rgb565, bottom: Rgb565) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // One `fill_contiguous` rather than a line per row. A row-at-a-time
    // gradient is a separate SPI window-set and write for every row, and on the
    // Pi that cost 36% of a core to redraw two boxes five times a second. This
    // streams the whole region through a single window.
    let height = area.size.height.max(1);
    let width = area.size.width.max(1);
    target.fill_contiguous(
        &area,
        (0..height).flat_map(move |row| {
            let color = mix(top, bottom, row, height);
            core::iter::repeat_n(color, width as usize)
        }),
    )
}

/// A knob that looks turned rather than printed.
///
/// Body gradient for the moulding, a bright crescent up-left where the light
/// would land, a dark rim below it, and a pointer. At this size the highlight
/// is what sells it — without it the knob is just a dark disc.
fn knob<D>(target: &mut D, center: Point, radius: i32, tilt: i32, lit: bool) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let base = if lit {
        Rgb565::new(6, 12, 6)
    } else {
        Rgb565::new(3, 6, 3)
    };
    let diameter = (radius * 2) as u32;

    // Seat: a dark ring the knob sits in, so it reads as raised.
    Circle::with_center(center, diameter + 2)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(target)?;

    // Body, shaded top-to-bottom, drawn a span at a time.
    //
    // Not a bounding-box fill: that has to paint *something* in the corners
    // outside the circle, and whatever colour is chosen is wrong somewhere. On
    // a coloured enclosure black corners were invisible; on a brushed faceplate
    // every knob sat in an obvious black square. A row of spans touches only
    // the knob, and a knob is small enough that the extra draws do not matter —
    // they happen on a change, not on every animation frame.
    let top_shade = lighten(base, 3, 5);
    let bottom_shade = darken(base, 2, 3);
    for row in -radius..=radius {
        let half = ((radius * radius - row * row).max(0) as f32).sqrt() as i32;
        if half <= 0 {
            continue;
        }
        let shade = mix(
            top_shade,
            bottom_shade,
            (row + radius) as u32,
            (radius * 2).max(1) as u32,
        );
        Line::new(
            Point::new(center.x - half, center.y + row),
            Point::new(center.x + half, center.y + row),
        )
        .into_styled(PrimitiveStyle::with_stroke(shade, 1))
        .draw(target)?;
    }

    // Specular crescent, up and to the left.
    if radius >= 4 {
        Circle::with_center(
            Point::new(center.x - radius / 3, center.y - radius / 3),
            (radius as u32).max(2),
        )
        .into_styled(PrimitiveStyle::with_stroke(lighten(base, 4, 5), 1))
        .draw(target)?;
    }

    // Pointer.
    Line::new(center, Point::new(center.x + tilt, center.y - radius + 1))
        .into_styled(PrimitiveStyle::with_stroke(
            if lit {
                Rgb565::WHITE
            } else {
                Rgb565::CSS_DIM_GRAY
            },
            1,
        ))
        .draw(target)?;
    Ok(())
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
///
/// Built up the way a real object catches light: a shadow beneath it, a body
/// graded from lit top to shaded bottom, a gloss over the upper half, a bright
/// bevel on the top edge and a dark one on the bottom. None of it is a texture
/// or a bitmap — it is all primitives, so it costs nothing to ship and scales
/// with whatever colour the pedal reports.
pub fn draw<D>(target: &mut D, area: Rectangle, pedal: &Pedal<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let base = if pedal.lit {
        pedal.color
    } else {
        // Unplayed boxes recede rather than disappear: still identifiable, but
        // never mistaken for the one making sound. This is where the contrast
        // comes from, since the lit box cannot go brighter than its own colour.
        dim(pedal.color, 1, 4)
    };
    let corners = CornerRadii::new(Size::new(4, 4));

    // Cast shadow, down and right. Two pixels is enough to lift the box off the
    // background at this size; more reads as a mistake.
    let shadow = Rectangle::new(
        Point::new(area.top_left.x + 2, area.top_left.y + 2),
        area.size,
    );
    RoundedRectangle::new(shadow, corners)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(1, 2, 1)))
        .draw(target)?;

    // Body: lighter at the top, darker at the foot.
    RoundedRectangle::new(area, corners)
        .into_styled(PrimitiveStyle::with_fill(base))
        .draw(target)?;
    let inner = Rectangle::new(
        Point::new(area.top_left.x + 1, area.top_left.y + 1),
        Size::new(area.size.width - 2, area.size.height - 2),
    );
    // Gentle. A saturated colour is already at the top of its channel — red is
    // (31,0,0) — so "brighter" can only mean mixing toward white, which drains
    // the hue. A vivid red washed to pink reads as *dimmer*, not brighter,
    // which is exactly the complaint this fixes. Keep the body close to the
    // pedal's own colour and let contrast with the unlit box do the work.
    gradient(target, inner, lighten(base, 1, 6), darken(base, 1, 4))?;

    // Gloss across the upper third, fading out — the iOS lozenge highlight.
    let gloss_height = (area.size.height / 3).max(3);
    let gloss = Rectangle::new(
        Point::new(area.top_left.x + 2, area.top_left.y + 1),
        Size::new(area.size.width - 4, gloss_height),
    );
    gradient(target, gloss, lighten(base, 3, 5), lighten(base, 1, 6))?;

    // Bevel: light along the top edge, dark along the bottom.
    let top_edge = Point::new(area.top_left.x + 3, area.top_left.y);
    Line::new(
        top_edge,
        Point::new(
            area.top_left.x + area.size.width as i32 - 4,
            area.top_left.y,
        ),
    )
    .into_styled(PrimitiveStyle::with_stroke(lighten(base, 4, 5), 1))
    .draw(target)?;
    let foot = area.top_left.y + area.size.height as i32 - 1;
    Line::new(
        Point::new(area.top_left.x + 3, foot),
        Point::new(area.top_left.x + area.size.width as i32 - 4, foot),
    )
    .into_styled(PrimitiveStyle::with_stroke(darken(base, 3, 5), 1))
    .draw(target)?;

    // Corner screws, if the box is big enough for them to read as screws
    // rather than as dirt.
    if area.size.width >= 40 {
        for (dx, dy) in [(4, 4), (-5, 4), (4, -5), (-5, -5)] {
            let cx = if dx > 0 {
                area.top_left.x + dx
            } else {
                area.top_left.x + area.size.width as i32 + dx
            };
            let cy = if dy > 0 {
                area.top_left.y + dy
            } else {
                area.top_left.y + area.size.height as i32 + dy
            };
            Circle::with_center(Point::new(cx, cy), 3)
                .into_styled(PrimitiveStyle::with_fill(darken(base, 3, 5)))
                .draw(target)?;
            Line::new(Point::new(cx - 1, cy), Point::new(cx + 1, cy))
                .into_styled(PrimitiveStyle::with_stroke(lighten(base, 3, 5), 1))
                .draw(target)?;
        }
    }

    if pedal.archetype == Archetype::Amp {
        draw_amp_face(target, area, base, pedal.lit)?;
    } else {
        draw_stomp_face(target, area, pedal, base)?;
    }

    // Name plate: a recessed dark strip, bevelled the opposite way to the body
    // so it reads as engraved rather than raised.
    let w = area.size.width as i32;
    let h = area.size.height as i32;
    let x = area.top_left.x;
    let y = area.top_left.y;

    let cols_for_band = (((w - 8) / 5).max(1)) as usize;
    let band_lines = wrap(badge_name(pedal.name), cols_for_band)
        .len()
        .clamp(1, 2) as u32;
    let band_y = y + h / 2 - 5;
    let plate = Rectangle::new(
        Point::new(x + 3, band_y),
        Size::new(w as u32 - 6, 2 + band_lines * 8),
    );
    plate
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(2, 4, 2)))
        .draw(target)?;
    Line::new(Point::new(x + 3, band_y), Point::new(x + w - 4, band_y))
        .into_styled(PrimitiveStyle::with_stroke(darken(base, 4, 5), 1))
        .draw(target)?;
    Line::new(
        Point::new(x + 3, band_y + plate.size.height as i32 - 1),
        Point::new(x + w - 4, band_y + plate.size.height as i32 - 1),
    )
    .into_styled(PrimitiveStyle::with_stroke(lighten(base, 2, 5), 1))
    .draw(target)?;

    let label = badge_name(pedal.name);
    let text_color = if pedal.lit {
        Rgb565::WHITE
    } else {
        Rgb565::CSS_DIM_GRAY
    };
    for (line, chunk) in wrap(label, cols_for_band).iter().take(2).enumerate() {
        Text::with_alignment(
            chunk,
            Point::new(x + w / 2, band_y + 8 + line as i32 * 8),
            MonoTextStyle::new(&FONT_5X8, text_color),
            Alignment::Center,
        )
        .draw(target)?;
    }

    Ok(())
}

/// Knobs along the top, a chrome footswitch and a glowing LED at the foot.
fn draw_stomp_face<D>(
    target: &mut D,
    area: Rectangle,
    pedal: &Pedal<'_>,
    base: Rgb565,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let w = area.size.width as i32;
    let h = area.size.height as i32;
    let x = area.top_left.x;
    let y = area.top_left.y;

    // Knobs across the upper third, each pointing slightly differently so the
    // row reads as a set of controls rather than a repeated stamp.
    let count = pedal.archetype.knobs() as i32;
    let radius = if w >= 50 { 5 } else { 4 };
    let knob_y = y + 12;
    for index in 0..count {
        let cx = x + (w * (index + 1)) / (count + 1);
        knob(
            target,
            Point::new(cx, knob_y),
            radius,
            index - count / 2,
            pedal.lit,
        )?;
    }

    // Footswitch: a chrome dome in a dark well.
    let switch_y = y + h - 11;
    let switch = Point::new(x + w / 2, switch_y);
    Circle::with_center(switch, 15)
        .into_styled(PrimitiveStyle::with_fill(darken(base, 4, 5)))
        .draw(target)?;
    let dome = Rectangle::new(Point::new(switch.x - 5, switch.y - 5), Size::new(11, 11));
    target.fill_contiguous(
        &dome,
        (0..11i32).flat_map(move |row| {
            let dy = row - 5;
            let half = ((25 - dy * dy).max(0) as f32).sqrt() as i32;
            let chrome = mix(
                Rgb565::new(24, 48, 24),
                Rgb565::new(6, 12, 6),
                row as u32,
                11,
            );
            (0..11i32).map(move |col| {
                if (col - 5).abs() <= half {
                    chrome
                } else {
                    darken(base, 4, 5)
                }
            })
        }),
    )?;
    Circle::with_center(Point::new(switch.x - 1, switch.y - 2), 4)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
        .draw(target)?;

    // Status LED, with a halo when lit so it reads as emitting rather than
    // painted on.
    let led_at = Point::new(x + w / 2, y + h - 24);
    if pedal.lit {
        Circle::with_center(led_at, 9)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::new(10, 6, 0)))
            .draw(target)?;
        Circle::with_center(led_at, 6)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::CSS_ORANGE_RED))
            .draw(target)?;
        Circle::with_center(Point::new(led_at.x - 1, led_at.y - 1), 2)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::new(31, 55, 20)))
            .draw(target)?;
    } else {
        Circle::with_center(led_at, 6)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::new(6, 4, 2)))
            .draw(target)?;
    }
    Ok(())
}

/// An amp head: a lit control panel over a grille.
fn draw_amp_face<D>(
    target: &mut D,
    area: Rectangle,
    base: Rgb565,
    lit: bool,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let w = area.size.width as i32;
    let h = area.size.height as i32;
    let x = area.top_left.x;
    let y = area.top_left.y;

    // Control panel, recessed and brushed.
    let panel = Rectangle::new(Point::new(x + 4, y + 5), Size::new(w as u32 - 8, 14));
    gradient(target, panel, Rgb565::new(7, 14, 7), Rgb565::new(3, 6, 3))?;
    for index in 0..4 {
        let cx = x + 8 + index * ((w - 16) / 3).max(1);
        knob(target, Point::new(cx, y + 12), 3, index - 2, lit)?;
    }

    // Grille cloth: diagonal weave over a dark ground.
    let grille = Rectangle::new(
        Point::new(x + 4, y + h / 2 + 6),
        Size::new(w as u32 - 8, (h / 2 - 10).max(1) as u32),
    );
    grille
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(2, 4, 2)))
        .draw(target)?;
    let weave = lighten(base, 1, 4);
    let mut offset = -h;
    while offset < w {
        Line::new(
            Point::new(x + 4 + offset, y + h - 5),
            Point::new(x + 4 + offset + h / 3, y + h / 2 + 6),
        )
        .into_styled(PrimitiveStyle::with_stroke(weave, 1))
        .draw(target)?;
        offset += 4;
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

/// A chicken-head knob whose pointer angle encodes a value.
///
/// The angle is the information, not decoration: a player learns "the second
/// one is at about ten o'clock" the way they do on a real amp, and can read the
/// setting from further away than any number at this size.
///
/// Sweeps the usual 300 degrees, seven o'clock round to five o'clock, leaving
/// the dead zone at the bottom where a real pot has its end stops.
pub fn chicken_head<D>(
    target: &mut D,
    center: Point,
    radius: i32,
    fraction: f32,
    face: Rgb565,
    lit: bool,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // Skirt, shaded so the knob sits proud of the panel.
    let body = if lit {
        Rgb565::new(4, 8, 4)
    } else {
        Rgb565::new(2, 4, 2)
    };
    // Coloured skirt: a ring of the slot's own colour, wide enough to read.
    Circle::with_center(center, (radius * 2 + 5) as u32)
        .into_styled(PrimitiveStyle::with_fill(darken(face, 1, 3)))
        .draw(target)?;
    Circle::with_center(center, (radius * 2 + 5) as u32)
        .into_styled(PrimitiveStyle::with_stroke(face, 1))
        .draw(target)?;

    // Body, shaded, with a specular crescent up-left.
    for row in -radius..=radius {
        let half = ((radius * radius - row * row).max(0) as f32).sqrt() as i32;
        if half <= 0 {
            continue;
        }
        let shade = mix(
            lighten(body, 2, 5),
            darken(body, 1, 2),
            (row + radius) as u32,
            (radius * 2).max(1) as u32,
        );
        Line::new(
            Point::new(center.x - half, center.y + row),
            Point::new(center.x + half, center.y + row),
        )
        .into_styled(PrimitiveStyle::with_stroke(shade, 1))
        .draw(target)?;
    }
    // Specular highlight: a small filled blob up-left, not a ring. A stroked
    // circle reads as something printed on the cap; a blob reads as light
    // landing on it, which is the entire difference.
    Circle::with_center(
        Point::new(center.x - radius / 3, center.y - radius / 3),
        (radius / 2).max(2) as u32,
    )
    .into_styled(PrimitiveStyle::with_fill(lighten(body, 3, 5)))
    .draw(target)?;

    // Pointer: 300 degrees of sweep starting at seven o'clock.
    let sweep = 300.0f32.to_radians();
    let start = 150.0f32.to_radians();
    let angle = start + fraction.clamp(0.0, 1.0) * sweep;
    let (sin, cos) = angle.sin_cos();
    let tip = Point::new(
        center.x + (sin * (radius as f32 + 2.0)) as i32,
        center.y - (cos * (radius as f32 + 2.0)) as i32,
    );
    let pointer = if lit {
        Rgb565::WHITE
    } else {
        Rgb565::CSS_DIM_GRAY
    };
    Line::new(center, tip)
        .into_styled(PrimitiveStyle::with_stroke(pointer, 2))
        .draw(target)?;
    // The beak: a small blob at the tip, which is what makes it chicken-head
    // rather than a plain line.
    Circle::with_center(tip, 3)
        .into_styled(PrimitiveStyle::with_fill(pointer))
        .draw(target)?;
    Ok(())
}

/// Tolex: a dark diagonal weave, so the background is not a flat void.
pub fn tolex<D>(target: &mut D, area: Rectangle, tint: Rgb565) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    area.into_styled(PrimitiveStyle::with_fill(darken(tint, 4, 5)))
        .draw(target)?;
    let weave = darken(tint, 3, 5);
    let height = area.size.height as i32;
    let mut offset = -height;
    while offset < area.size.width as i32 {
        Line::new(
            Point::new(area.top_left.x + offset, area.top_left.y + height),
            Point::new(area.top_left.x + offset + height, area.top_left.y),
        )
        .into_styled(PrimitiveStyle::with_stroke(weave, 1))
        .draw(target)?;
        offset += 6;
    }
    Ok(())
}

/// A jewel pilot lamp, glowing the colour it is given.
pub fn jewel<D>(target: &mut D, center: Point, color: Rgb565, lit: bool) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // Chrome bezel.
    Circle::with_center(center, 13)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(10, 20, 10)))
        .draw(target)?;
    Circle::with_center(center, 11)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(4, 8, 4)))
        .draw(target)?;

    let glass = if lit { color } else { dim(color, 1, 5) };
    Circle::with_center(center, 9)
        .into_styled(PrimitiveStyle::with_fill(glass))
        .draw(target)?;
    if lit {
        // Hot centre and a specular pin, which is what makes glass read as lit
        // rather than merely coloured.
        Circle::with_center(center, 5)
            .into_styled(PrimitiveStyle::with_fill(lighten(glass, 2, 5)))
            .draw(target)?;
        Circle::with_center(Point::new(center.x - 2, center.y - 2), 3)
            .into_styled(PrimitiveStyle::with_fill(Rgb565::WHITE))
            .draw(target)?;
    }
    Ok(())
}

/// A brushed-metal faceplate: a vertical gradient with fine horizontal grain.
pub fn brushed<D>(target: &mut D, area: Rectangle) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    gradient(target, area, Rgb565::new(17, 34, 17), Rgb565::new(9, 18, 9))?;
    // Grain: every third row a shade off, which reads as brushed metal at this
    // size where an actual fine texture would just alias into noise.
    let mut row = area.top_left.y + 2;
    while row < area.top_left.y + area.size.height as i32 {
        Line::new(
            Point::new(area.top_left.x, row),
            Point::new(area.top_left.x + area.size.width as i32 - 1, row),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(13, 26, 13), 1))
        .draw(target)?;
        row += 3;
    }
    // Bevel top and bottom.
    Line::new(
        area.top_left,
        Point::new(
            area.top_left.x + area.size.width as i32 - 1,
            area.top_left.y,
        ),
    )
    .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(24, 48, 24), 1))
    .draw(target)?;
    let foot = area.top_left.y + area.size.height as i32 - 1;
    Line::new(
        Point::new(area.top_left.x, foot),
        Point::new(area.top_left.x + area.size.width as i32 - 1, foot),
    )
    .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(3, 6, 3), 1))
    .draw(target)?;
    Ok(())
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
    fn text_that_fits_never_moves() {
        for tick in 0..50 {
            assert_eq!(marquee("SHORT", 10, tick), "SHORT");
        }
        assert!(!scrolls("SHORT", 10));
    }

    #[test]
    fn long_text_starts_at_the_beginning_and_reaches_the_end() {
        let text = "MORNING GLORY - BRIGHT CUT 2";
        let cols = 11;
        assert!(scrolls(text, cols));

        // It opens on the start, held for a beat.
        assert_eq!(marquee(text, cols, 0), "MORNING GLO");
        assert_eq!(marquee(text, cols, 3), "MORNING GLO");

        // Somewhere in the cycle the tail is visible.
        let seen: Vec<String> = (0..200).map(|t| marquee(text, cols, t)).collect();
        assert!(
            seen.iter().any(|w| w.ends_with("CUT 2")),
            "the end of the name is never shown"
        );
        assert!(
            seen.iter().any(|w| w.starts_with("MORNING")),
            "the start of the name is never shown"
        );
    }

    /// Ping-pong, not wrap-around: the window must always be a contiguous run
    /// of the original, or the name reads as a different one mid-scroll.
    #[test]
    fn the_window_is_always_a_real_substring() {
        let text = "PROTEIN - GREEN 3";
        for tick in 0..300 {
            let window = marquee(text, 8, tick);
            assert!(
                text.contains(&window),
                "tick {tick} produced {window:?}, which is not part of the name"
            );
            assert_eq!(window.chars().count(), 8);
        }
    }

    /// It must come back, or a name scrolled away is gone for good.
    #[test]
    fn scrolling_returns_to_the_start() {
        let text = "A VERY LONG PRESET NAME INDEED";
        let first = marquee(text, 10, 0);
        let later: Vec<String> = (1..400).map(|t| marquee(text, 10, t)).collect();
        assert!(
            later.contains(&first),
            "the scroll never returns to the beginning"
        );
    }

    /// A saturated colour must stay saturated when lit.
    ///
    /// The gloss mixes toward white, and overdoing it turned a vivid red into
    /// pink — which reads as dimmer, not brighter, and was reported as "the red
    /// pedal on the screen is dimmed" while the hardware showed vivid red.
    #[test]
    fn a_lit_saturated_colour_keeps_its_hue() {
        use embedded_graphics::primitives::Rectangle;

        let red = from_rgb8([255, 0, 0]);
        let mut lit = TestTarget::new();
        draw(
            &mut lit,
            Rectangle::new(Point::new(0, 0), Size::new(59, 76)),
            &Pedal {
                name: "TEST",
                color: red,
                archetype: Archetype::Overdrive,
                lit: true,
            },
        )
        .unwrap();

        // Across the body, red must dominate the other channels: a washed-out
        // pastel has green and blue creeping up to meet it.
        let body: Vec<Rgb565> = lit.pixels.iter().filter(|p| p.r() > 8).copied().collect();
        assert!(!body.is_empty(), "nothing red was drawn");
        let washed = body
            .iter()
            .filter(|p| p.g() as u16 * 2 > p.r() as u16 * 3)
            .count();
        assert!(
            washed * 4 < body.len(),
            "{washed} of {} red pixels are washed toward white",
            body.len()
        );
    }

    /// Lit and unlit must be plainly different, since that is what tells a
    /// player which box is making sound.
    #[test]
    fn a_lit_box_is_clearly_brighter_than_an_unlit_one() {
        use embedded_graphics::primitives::Rectangle;

        let area = Rectangle::new(Point::new(0, 0), Size::new(59, 76));
        let brightness = |lit: bool| {
            let mut target = TestTarget::new();
            draw(
                &mut target,
                area,
                &Pedal {
                    name: "TEST",
                    color: from_rgb8([255, 0, 0]),
                    archetype: Archetype::Overdrive,
                    lit,
                },
            )
            .unwrap();
            target.pixels.iter().map(|p| p.r() as u32).sum::<u32>()
        };
        let on = brightness(true);
        let off = brightness(false);
        assert!(
            on > off * 2,
            "lit ({on}) should be clearly brighter than unlit ({off})"
        );
    }

    /// A tiny in-memory target, so drawing is checkable without a panel.
    struct TestTarget {
        pixels: Vec<Rgb565>,
    }

    impl TestTarget {
        fn new() -> Self {
            Self {
                pixels: vec![Rgb565::BLACK; 64 * 80],
            }
        }
    }

    impl Dimensions for TestTarget {
        fn bounding_box(&self) -> embedded_graphics::primitives::Rectangle {
            embedded_graphics::primitives::Rectangle::new(Point::zero(), Size::new(64, 80))
        }
    }

    impl DrawTarget for TestTarget {
        type Color = Rgb565;
        type Error = core::convert::Infallible;

        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = Pixel<Self::Color>>,
        {
            for Pixel(point, color) in pixels {
                if (0..64).contains(&point.x) && (0..80).contains(&point.y) {
                    self.pixels[point.y as usize * 64 + point.x as usize] = color;
                }
            }
            Ok(())
        }
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
