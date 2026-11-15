//! Field-sequential stereo output.
//!
//! Cores that emulate active-shutter glasses report which eye each completed
//! field belongs to. A host without shutter glasses can combine two
//! consecutive fields into an anaglyph, which is the display model this
//! helper provides.

/// Which eye a field was rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StereoEye {
    /// Left eye.
    Left,
    /// Right eye.
    Right,
}

/// Composes a red/cyan anaglyph from a left and a right RGBA8 field.
///
/// The red channel comes from `left`; green and blue come from `right`. Alpha
/// is forced opaque. Bytes beyond the shortest input or output are ignored, so
/// callers can pass frames whose stride was already packed to width * 4.
pub fn compose_anaglyph(left: &[u8], right: &[u8], out: &mut [u8]) {
    let pixels = left.len().min(right.len()).min(out.len()) / 4;
    for index in 0..pixels {
        let offset = index * 4;
        out[offset] = left[offset];
        out[offset + 1] = right[offset + 1];
        out[offset + 2] = right[offset + 2];
        out[offset + 3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anaglyph_takes_red_from_left_and_cyan_from_right() {
        let left = [10, 20, 30, 255, 40, 50, 60, 255];
        let right = [70, 80, 90, 255, 100, 110, 120, 255];
        let mut out = [0u8; 8];
        compose_anaglyph(&left, &right, &mut out);
        assert_eq!(out, [10, 80, 90, 255, 40, 110, 120, 255]);
    }

    #[test]
    fn anaglyph_ignores_trailing_bytes() {
        let left = [1, 2, 3, 4, 5];
        let right = [6, 7, 8, 9, 10];
        let mut out = [0u8; 4];
        compose_anaglyph(&left, &right, &mut out);
        assert_eq!(out, [1, 7, 8, 255]);
    }
}
