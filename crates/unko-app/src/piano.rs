//! On-screen Miracle Piano keyboard.
//!
//! The piano's 49 keys are C2-C6. The widget is intentionally small: the
//! Devices window shows it when a Miracle Piano is attached, and pointer
//! press/drag sends Note On/Off through `Core::set_piano_note`. It is a
//! presentation input, not a model of the piano's keybed matrix.

use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, Ui};

/// Lowest piano note (C2).
pub const LOW: u8 = 0x24;
/// Highest piano note (C6).
pub const HIGH: u8 = 0x54;

/// Whether a MIDI note is a black key.
#[must_use]
pub fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

#[cfg(test)]
fn white_count() -> usize {
    (LOW..=HIGH).filter(|note| !is_black(*note)).count()
}

type KeyRects = (Vec<(u8, Rect)>, Vec<(u8, Rect)>);

fn key_rects(rect: Rect) -> KeyRects {
    let whites: Vec<u8> = (LOW..=HIGH).filter(|note| !is_black(*note)).collect();
    let width = rect.width() / whites.len().max(1) as f32;
    let mut white_rects = Vec::with_capacity(whites.len());
    for (index, note) in whites.iter().enumerate() {
        white_rects.push((
            *note,
            Rect::from_min_size(
                pos2(rect.left() + index as f32 * width, rect.top()),
                vec2(width, rect.height()),
            ),
        ));
    }
    let black_width = width * 0.62;
    let black_height = rect.height() * 0.62;
    let mut black_rects = Vec::new();
    for note in LOW..=HIGH {
        if !is_black(note) {
            continue;
        }
        let whites_before = (LOW..note).filter(|n| !is_black(*n)).count();
        black_rects.push((
            note,
            Rect::from_min_size(
                pos2(
                    rect.left() + whites_before as f32 * width - black_width * 0.5,
                    rect.top(),
                ),
                vec2(black_width, black_height),
            ),
        ));
    }
    (white_rects, black_rects)
}

/// Returns the note under `pos`, preferring black keys where they overlap.
#[must_use]
pub fn key_at(rect: Rect, pos: egui::Pos2) -> Option<u8> {
    if !rect.contains(pos) {
        return None;
    }
    let (whites, blacks) = key_rects(rect);
    for (note, key) in &blacks {
        if key.contains(pos) {
            return Some(*note);
        }
    }
    for (note, key) in &whites {
        if key.contains(pos) {
            return Some(*note);
        }
    }
    None
}

/// Draws the keyboard and returns Note On/Off transitions.
///
/// The caller owns `last` so a pointer release or a closed window emits the
/// matching Note Off on the next frame.
pub fn keyboard(ui: &mut Ui, last: &mut Option<u8>) -> Vec<(u8, bool)> {
    let width = ui.available_width().clamp(280.0, 720.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 120.0), Sense::click_and_drag());
    let (whites, blacks) = key_rects(rect);
    let painter = ui.painter_at(rect);
    for (note, key) in &whites {
        let active = *last == Some(*note);
        painter.rect_filled(
            *key,
            2.0,
            if active {
                Color32::from_rgb(200, 220, 255)
            } else {
                Color32::WHITE
            },
        );
        painter.rect_stroke(
            *key,
            2.0,
            Stroke::new(1.0_f32, Color32::BLACK),
            egui::StrokeKind::Inside,
        );
    }
    for (note, key) in &blacks {
        let active = *last == Some(*note);
        painter.rect_filled(
            *key,
            2.0,
            if active {
                Color32::from_rgb(70, 90, 130)
            } else {
                Color32::BLACK
            },
        );
    }

    let pointer = response
        .is_pointer_button_down_on()
        .then(|| response.interact_pointer_pos())
        .flatten();
    let note = pointer.and_then(|pos| key_at(rect, pos));
    let mut events = Vec::new();
    if note != *last {
        if let Some(old) = *last {
            events.push((old, false));
        }
        if let Some(new) = note {
            events.push((new, true));
        }
        *last = note;
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard_rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(490.0, 100.0))
    }

    #[test]
    fn keyboard_covers_49_keys() {
        let (whites, blacks) = key_rects(keyboard_rect());
        assert_eq!(whites.len() + blacks.len(), 49);
        assert_eq!(whites.first().map(|(note, _)| *note), Some(LOW));
        assert_eq!(whites.last().map(|(note, _)| *note), Some(HIGH));
        assert_eq!(white_count(), whites.len());
    }

    #[test]
    fn hit_test_prefers_black_keys() {
        let rect = keyboard_rect();
        let (whites, blacks) = key_rects(rect);
        let (white_note, white_key) = whites[1];
        assert_eq!(key_at(rect, white_key.center()), Some(white_note));
        let (black_note, black_key) = blacks[0];
        assert_eq!(key_at(rect, black_key.center()), Some(black_note));
        assert_eq!(
            key_at(rect, pos2(rect.right() + 1.0, rect.center().y)),
            None
        );
    }

    #[test]
    fn note_names_mark_black_keys() {
        let black: Vec<u8> = (LOW..=HIGH).filter(|note| is_black(*note)).collect();
        assert!(black.contains(&0x25)); // C#2
        assert!(!black.contains(&0x24)); // C2
    }
}
