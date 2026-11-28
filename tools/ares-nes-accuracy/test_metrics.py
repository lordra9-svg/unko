"""Unit tests for the palette-index agreement metric.

The scoring rule must ignore frames where both engines render a uniform
frame (hardware-unspecified warm-up/blank frames) without hiding a divergence
where only one engine is uniform.
"""

import unittest
from array import array

import rerun_index

PIXELS = rerun_index.PIXELS


def uniform(value):
    return array('H', [value]) * PIXELS


def content(seed):
    # Non-uniform, and seeds 0/1 use disjoint value ranges so unrelated
    # content frames share no matching pixels.
    return array('H', (16 + seed * 16 + (index * 7) % 16 for index in range(PIXELS)))


# A small renderable area inside a larger "border", mimicking the PAL mask:
# rows 1..4 and columns 2..9 are included, everything else is excluded.
MASK = [y * 256 + x for y in range(1, 5) for x in range(2, 10)]


def bordered(inside, border, mask=MASK):
    """Frame whose masked pixels are *inside* and all others are *border*."""
    frame = array('H', [border]) * PIXELS
    for offset in mask:
        frame[offset] = inside
    return frame


class AgreementTests(unittest.TestCase):
    def test_both_uniform_frame_is_excluded(self):
        # The two blank frames use different colours, so counting them would
        # contribute a mismatch. Exclusion removes them from both sides.
        ours = uniform(5) + content(0)
        ref = uniform(7) + content(0)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=2)
        self.assertEqual(excluded, 1)
        self.assertEqual(scored, 1)
        self.assertEqual(total, PIXELS)
        self.assertEqual(same, PIXELS)

    def test_one_uniform_against_content_is_scored_as_zero(self):
        ours = uniform(5)
        ref = content(0)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=1)
        self.assertEqual(excluded, 0)
        self.assertEqual(scored, 1)
        self.assertEqual(total, PIXELS)
        self.assertEqual(same, 0)

    def test_content_frames_are_scored_normally(self):
        ours = content(0) + content(0)
        ref = content(0) + content(1)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=2)
        self.assertEqual(excluded, 0)
        self.assertEqual(scored, 2)
        self.assertEqual(total, 2 * PIXELS)
        self.assertEqual(same, PIXELS)

    def test_equal_uniform_frame_stays_scored(self):
        ours = uniform(5) + content(0)
        ref = uniform(5) + content(0)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=2)
        self.assertEqual(excluded, 0)
        self.assertEqual(scored, 2)
        self.assertEqual(total, 2 * PIXELS)
        self.assertEqual(same, 2 * PIXELS)

    def test_differing_uniform_after_prefix_is_scored(self):
        # Frame 0 is an equal blank (stays scored). Frame 1 has content and
        # stops the warm-up prefix. Frame 2 is a differing uniform frame after
        # the prefix, so it must be scored (as a miss), not excluded.
        ours = uniform(5) + content(0) + uniform(7)
        ref = uniform(5) + content(0) + uniform(9)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=3)
        self.assertEqual(excluded, 0)
        self.assertEqual(scored, 3)
        self.assertEqual(total, 3 * PIXELS)
        self.assertEqual(same, 2 * PIXELS)

    def test_all_uniform_frames_do_not_score_100(self):
        ours = uniform(5) + uniform(5)
        ref = uniform(7) + uniform(7)
        metrics = rerun_index.index_metrics(ours, ref, start=0, end=2)
        self.assertEqual(metrics['excluded_frames'], 2)
        self.assertEqual(metrics['scored_frames'], 0)
        self.assertIsNone(metrics['video_pct'])
        self.assertNotEqual(metrics['video_pct'], 100.0)
        # The final-frame diagnostic stays raw: it is 0.0, never n/a.
        self.assertEqual(metrics['final_video_pct'], 0.0)


class MaskedUniformityTests(unittest.TestCase):
    def test_is_uniform_ignores_excluded_border(self):
        # Uniform inside the mask, differing only outside it.
        self.assertTrue(rerun_index.is_uniform(bordered(5, 1), MASK))
        self.assertFalse(rerun_index.is_uniform(content(0), MASK))
        # No mask keeps the original whole-frame test.
        self.assertFalse(rerun_index.is_uniform(bordered(5, 1)))
        self.assertTrue(rerun_index.is_uniform(uniform(5)))

    def test_differing_uniform_colours_inside_mask_are_excluded(self):
        # Both frames are uniform over the included pixels, with different
        # colours there. The excluded border differs too, but must not stop
        # the warm-up blank from being detected.
        ours = bordered(5, 1)
        ref = bordered(7, 2)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=1, mask=MASK)
        self.assertEqual(excluded, 1)
        self.assertEqual(scored, 0)
        self.assertEqual(total, 0)
        self.assertEqual(same, 0)

    def test_equal_uniform_inside_mask_is_scored_as_100(self):
        # Uniform and equal over the included pixels while the excluded border
        # differs: this is a real (blank) frame and must stay in the score.
        ours = bordered(5, 1)
        ref = bordered(5, 7)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=1, mask=MASK)
        self.assertEqual(excluded, 0)
        self.assertEqual(scored, 1)
        self.assertEqual(total, len(MASK))
        self.assertEqual(same, len(MASK))

    def test_uniform_inside_mask_against_content_is_scored_as_zero(self):
        ours = bordered(5, 1)
        ref = content(0)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=1, mask=MASK)
        self.assertEqual(excluded, 0)
        self.assertEqual(scored, 1)
        self.assertEqual(total, len(MASK))
        self.assertEqual(same, 0)

    def test_mask_prefix_stops_at_content_like_unmasked(self):
        # Same prefix rule: a differing uniform inside the mask is excluded,
        # but once either side renders content later differing blanks score.
        ours = bordered(5, 1) + content(0) + bordered(7, 1)
        ref = bordered(9, 2) + content(0) + bordered(11, 2)
        same, total, excluded, scored = rerun_index.agreement(
            ours, ref, 0, start=0, end=3, mask=MASK)
        self.assertEqual(excluded, 1)
        self.assertEqual(scored, 2)
        self.assertEqual(total, 2 * len(MASK))
        self.assertEqual(same, len(MASK))


if __name__ == '__main__':
    unittest.main()
