//! Extracting preset names from preset-details responses.
//!
//! The name is not at a fixed offset. `Builty/TonexOneController` locates it by
//! scanning for a fixed byte marker and taking the 32 bytes that follow, which is
//! what we do here.

/// Byte sequence immediately preceding a preset name.
pub const NAME_MARKER: [u8; 6] = [0xB9, 0x04, 0xB9, 0x02, 0xBC, 0x21];

/// Preset names occupy a fixed 32-byte field.
pub const NAME_LEN: usize = 32;

/// Find the preset name in a decoded preset-details body.
///
/// Returns `None` if the marker is absent or the field is truncated — never a
/// partial or garbage name, so a caller can distinguish "no name here" from a
/// name that happens to be empty.
pub fn extract_name(body: &[u8]) -> Option<String> {
    let start = find_marker(body)? + NAME_MARKER.len();
    let field = body.get(start..start + NAME_LEN)?;
    Some(decode_name(field))
}

/// Offset of the name marker within `body`.
fn find_marker(body: &[u8]) -> Option<usize> {
    body.windows(NAME_MARKER.len())
        .position(|w| w == NAME_MARKER)
}

/// Decode a fixed-width name field: trim at the first NUL, then strip padding.
///
/// Lossy UTF-8 because a firmware quirk or a misaligned marker must not be able
/// to fail a read that is otherwise fine.
fn decode_name(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_with_name(name: &[u8]) -> Vec<u8> {
        let mut body = vec![0xAA; 12];
        body.extend_from_slice(&NAME_MARKER);
        let mut field = name.to_vec();
        field.resize(NAME_LEN, 0);
        body.extend_from_slice(&field);
        body.extend_from_slice(&[0xBB; 40]);
        body
    }

    #[test]
    fn extracts_a_nul_padded_name() {
        assert_eq!(
            extract_name(&body_with_name(b"Crunch Rhythm")).unwrap(),
            "Crunch Rhythm"
        );
    }

    #[test]
    fn extracts_a_name_that_fills_the_field() {
        let full = vec![b'X'; NAME_LEN];
        assert_eq!(
            extract_name(&body_with_name(&full)).unwrap(),
            "X".repeat(NAME_LEN)
        );
    }

    #[test]
    fn trims_space_padding() {
        assert_eq!(extract_name(&body_with_name(b"Clean   ")).unwrap(), "Clean");
    }

    #[test]
    fn an_empty_field_yields_an_empty_name_not_none() {
        assert_eq!(extract_name(&body_with_name(b"")).unwrap(), "");
    }

    #[test]
    fn missing_marker_yields_none() {
        assert_eq!(extract_name(&[0x00; 128]), None);
        assert_eq!(extract_name(&[]), None);
    }

    #[test]
    fn truncated_name_field_yields_none_rather_than_a_partial_name() {
        let mut body = vec![0xAA; 4];
        body.extend_from_slice(&NAME_MARKER);
        body.extend_from_slice(b"Half a nam"); // shorter than NAME_LEN
        assert_eq!(extract_name(&body), None);
    }

    #[test]
    fn invalid_utf8_does_not_fail_the_read() {
        let name = extract_name(&body_with_name(b"Bad\xFF\xFEName")).unwrap();
        assert!(name.starts_with("Bad"), "got {name:?}");
        assert!(name.ends_with("Name"), "got {name:?}");
    }

    #[test]
    fn finds_the_marker_when_it_is_not_at_the_start() {
        let body = body_with_name(b"Lead");
        assert_eq!(find_marker(&body), Some(12));
    }
}
