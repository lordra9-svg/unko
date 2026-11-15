//! Mapper test-ROM verdict parsing.
//!
//! The public mapper ROMs use three different oracles:
//!
//! - blargg-style text on screen and at `$6000` (`Passed` / `Failed`);
//! - Holy Mapperel's result line, a four-digit hex code after a colon
//!   (`0000` means pass);
//! - visual-only ROMs with no machine-readable verdict, which are smoke-tested
//!   for a rendered, multi-colour frame.
//!
//! These helpers keep the parsing out of the example so it can be unit-tested.

/// Classifies a blargg-style verdict from screen and `$6000` text.
///
/// Case-insensitive. When both words appear, the later one wins, so a ROM that
/// prints an instruction containing "fail" earlier still reports its final
/// result correctly. Returns `None` when neither word is present.
#[must_use]
pub fn classify_blargg(text: &str) -> Option<bool> {
    let lower = text.to_ascii_lowercase();
    let pass = lower.rfind("passed").or_else(|| lower.rfind("pass"));
    let fail = lower.rfind("failed").or_else(|| lower.rfind("fail"));
    match (pass, fail) {
        (Some(pass), Some(fail)) => Some(pass > fail),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

/// Parses Holy Mapperel's result line.
///
/// The line has the form `...: 0000`; only a four-digit hex token after a
/// colon is accepted. Returns the code (`0` is a pass).
#[must_use]
pub fn parse_holy_mapperel_result(rows: &[String]) -> Option<u32> {
    for row in rows {
        let Some((_, tail)) = row.rsplit_once(':') else {
            continue;
        };
        let token = tail.trim();
        if token.len() == 4 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            return u32::from_str_radix(token, 16).ok();
        }
    }
    None
}

/// Extracts the iNES/NES 2.0 mapper number from a header.
#[must_use]
pub fn mapper_number(rom: &[u8]) -> Option<u16> {
    if rom.len() < 16 || &rom[0..4] != b"NES\x1a" {
        return None;
    }
    let low = u16::from(rom[6] >> 4);
    let high = u16::from(rom[7] & 0xF0);
    let upper = if rom[7] & 0x08 != 0 {
        u16::from(rom[8] & 0x0F) << 8
    } else {
        0
    };
    Some(low | high | upper)
}

/// Counts distinct RGB colours in a rendered frame (alpha ignored).
#[must_use]
pub fn distinct_colors(video: &[u8]) -> usize {
    let mut colors = std::collections::BTreeSet::new();
    for pixel in video.as_chunks::<4>().0 {
        colors.insert((pixel[0], pixel[1], pixel[2]));
    }
    colors.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blargg_verdicts_are_case_insensitive() {
        assert_eq!(classify_blargg("5-MMC3\\nPassed."), Some(true));
        assert_eq!(classify_blargg("  PASSED"), Some(true));
        assert_eq!(classify_blargg("Failed. See screen"), Some(false));
        assert_eq!(classify_blargg("running... sub-test 3"), None);
        // The last verdict wins when a ROM recaps earlier sub-tests.
        assert_eq!(
            classify_blargg("1 pass\\n2 fail\\nfinal: Passed"),
            Some(true)
        );
        assert_eq!(
            classify_blargg("instructions may fail\\nall tests Passed"),
            Some(true)
        );
    }

    #[test]
    fn holy_mapperel_reads_the_hex_result_line() {
        let rows = vec![
            "  000 ????".to_string(),
            "  ????????? ???? ??????: 0000".to_string(),
            "  ???? ?? ???????".to_string(),
        ];
        assert_eq!(parse_holy_mapperel_result(&rows), Some(0));
        let rows = vec!["  ????????? ???? ??????: 0301".to_string()];
        assert_eq!(parse_holy_mapperel_result(&rows), Some(0x0301));
    }

    #[test]
    fn holy_mapperel_ignores_non_result_lines() {
        let rows = vec![
            "  000 ????".to_string(),
            "  32? ??? ???".to_string(),
            "  ??????: nope".to_string(),
            "  ??????: 12".to_string(),
        ];
        assert_eq!(parse_holy_mapperel_result(&rows), None);
        assert_eq!(parse_holy_mapperel_result(&[]), None);
    }

    #[test]
    fn mapper_numbers_cover_ines_and_nes20_high_bits() {
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"NES\x1a");
        header[6] = 0x40; // mapper 4
        assert_eq!(mapper_number(&header), Some(4));
        header[6] = 0x50; // mapper 5
        assert_eq!(mapper_number(&header), Some(5));
        header[6] = 0x61; // mapper 22 in the low nibble + high nibble
        header[7] = 0x10;
        assert_eq!(mapper_number(&header), Some(0x16));
        // NES 2.0 byte 8 extends the mapper into bits 8-11.
        header[6] = 0x00;
        header[7] = 0x08;
        header[8] = 0x02;
        assert_eq!(mapper_number(&header), Some(0x200));
        assert_eq!(mapper_number(b"not a nes"), None);
    }

    #[test]
    fn distinct_colors_counts_rgb_triples() {
        let frame = [0, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255];
        assert_eq!(distinct_colors(&frame), 2);
    }
}
