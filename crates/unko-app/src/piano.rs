//! On-screen Miracle Piano keyboard.
//!
//! The piano's 49 keys are C2-C6. The widget is intentionally small: the
//! Devices window shows it when a Miracle Piano is attached, and pointer
//! press/drag sends Note On/Off through `NesCore::set_piano_note`. It is a
//! presentation input, not a model of the piano's keybed matrix.

use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, Ui};

/// Lowest piano note (C2).
pub const LOW: u8 = 0x24;
/// Highest piano note (C6).
pub const HIGH: u8 = 0x54;

/// Computer-key to note mapping, chromatic across the four main rows.
///
/// The last key (Space) is only captured while the Devices window is open and
/// the piano keyboard toggle is on, so it does not also pause the game.
pub const KEY_NOTES: &[(egui::Key, u8)] = &[
    (egui::Key::Num1, LOW),
    (egui::Key::Num2, LOW + 1),
    (egui::Key::Num3, LOW + 2),
    (egui::Key::Num4, LOW + 3),
    (egui::Key::Num5, LOW + 4),
    (egui::Key::Num6, LOW + 5),
    (egui::Key::Num7, LOW + 6),
    (egui::Key::Num8, LOW + 7),
    (egui::Key::Num9, LOW + 8),
    (egui::Key::Num0, LOW + 9),
    (egui::Key::Minus, LOW + 10),
    (egui::Key::Equals, LOW + 11),
    (egui::Key::Q, LOW + 12),
    (egui::Key::W, LOW + 13),
    (egui::Key::E, LOW + 14),
    (egui::Key::R, LOW + 15),
    (egui::Key::T, LOW + 16),
    (egui::Key::Y, LOW + 17),
    (egui::Key::U, LOW + 18),
    (egui::Key::I, LOW + 19),
    (egui::Key::O, LOW + 20),
    (egui::Key::P, LOW + 21),
    (egui::Key::OpenBracket, LOW + 22),
    (egui::Key::CloseBracket, LOW + 23),
    (egui::Key::A, LOW + 24),
    (egui::Key::S, LOW + 25),
    (egui::Key::D, LOW + 26),
    (egui::Key::F, LOW + 27),
    (egui::Key::G, LOW + 28),
    (egui::Key::H, LOW + 29),
    (egui::Key::J, LOW + 30),
    (egui::Key::K, LOW + 31),
    (egui::Key::L, LOW + 32),
    (egui::Key::Semicolon, LOW + 33),
    (egui::Key::Quote, LOW + 34),
    (egui::Key::Backslash, LOW + 35),
    (egui::Key::Z, LOW + 36),
    (egui::Key::X, LOW + 37),
    (egui::Key::C, LOW + 38),
    (egui::Key::V, LOW + 39),
    (egui::Key::B, LOW + 40),
    (egui::Key::N, LOW + 41),
    (egui::Key::M, LOW + 42),
    (egui::Key::Comma, LOW + 43),
    (egui::Key::Period, LOW + 44),
    (egui::Key::Slash, LOW + 45),
    (egui::Key::Backtick, LOW + 46),
    (egui::Key::F1, LOW + 47),
    (egui::Key::Space, LOW + 48),
];

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
    fn keyboard_mapping_is_chromatic_and_unique() {
        assert_eq!(KEY_NOTES.len(), 49);
        let notes: Vec<u8> = KEY_NOTES.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes.first(), Some(&LOW));
        assert_eq!(notes.last(), Some(&HIGH));
        let mut seen = std::collections::BTreeSet::new();
        for note in &notes {
            assert!(seen.insert(*note), "duplicate note {note}");
        }
        let keys: Vec<egui::Key> = KEY_NOTES.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys.len(), 49);
    }

    #[test]
    fn note_names_mark_black_keys() {
        let black: Vec<u8> = (LOW..=HIGH).filter(|note| is_black(*note)).collect();
        assert!(black.contains(&0x25)); // C#2
        assert!(!black.contains(&0x24)); // C2
    }
}
