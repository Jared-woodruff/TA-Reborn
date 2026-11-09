"""Regression checks for tree identity while a moving camera changes detail level.

Run from the repository root: python -m unittest discover -s tools/features -v
"""
import unittest
from unittest.mock import patch

import numpy as np

import build


class TreeLodTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.models, cls.leaves, cls.branches = {}, {}, {}
        place, tube = build.place_cards, build.tube
        for name, generate in build.catalogue():
            if not name.startswith(("broadleaf_", "conifer_", "jungle_", "tuber_", "tropical_", "dead_", "bush_", "fern_")):
                continue
            cls.models[name], cls.leaves[name], cls.branches[name] = [], [], []
            for lod in range(3):
                leaves, branches = {}, []

                def record_leaves(model, crown, cards, layer, **kwargs):
                    leaves[layer] = (cards, kwargs.get("hang", False), kwargs.get("fold", 0.0))
                    return place(model, crown, cards, layer, **kwargs)

                def record_tube(part, pts, *args, **kwargs):
                    branches.append(np.asarray(pts).copy())
                    return tube(part, pts, *args, **kwargs)

                with patch.object(build, "place_cards", side_effect=record_leaves), patch.object(build, "tube", side_effect=record_tube):
                    cls.models[name].append(generate(lod))
                cls.leaves[name].append(leaves)
                cls.branches[name].append(branches)

    @staticmethod
    def identities(cards, hang):
        return {tuple(np.round(np.concatenate([build.card_centre(c, hang), build.unit(c[1]), build.unit(c[2])]), 6)): build.card_phase(c, hang) for c in cards}

    def test_all_tree_levels_keep_card_positions_orientations_and_wind_phases(self):
        for name, levels in self.leaves.items():
            self.assertTrue(levels[0], name)
            for lod in (1, 2):
                self.assertEqual(set(levels[lod]), set(levels[0]), name)
                for layer, (cards, hang, _) in levels[lod].items():
                    with self.subTest(model=name, lod=lod, layer=layer):
                        full = self.identities(levels[0][layer][0], hang)
                        previous = self.identities(levels[lod - 1][layer][0], hang)
                        current = self.identities(cards, hang)
                        self.assertLessEqual(current.keys(), previous.keys())
                        for identity, phase in current.items():
                            self.assertEqual(phase, full[identity])
                        colours = self.models[name][lod].parts[layer].col
                        self.assertEqual(len(colours), len(cards))
                        for card, colour in zip(cards, colours):
                            np.testing.assert_array_equal(colour[:, 3], np.full(len(colour), build.card_phase(card, hang)))

    def test_all_tree_levels_keep_crown_bounds(self):
        for name, levels in self.leaves.items():
            for layer, (cards, hang, fold) in levels[0].items():
                full = np.concatenate([build.card_corners(c, hang, fold) for c in cards])
                for lod in (1, 2):
                    with self.subTest(model=name, lod=lod, layer=layer):
                        corners = np.concatenate([build.card_corners(c, hang, fold) for c in levels[lod][layer][0]])
                        np.testing.assert_allclose(corners.min(axis=0), full.min(axis=0), atol=1e-9)
                        np.testing.assert_allclose(corners.max(axis=0), full.max(axis=0), atol=1e-9)

    def test_jungle_roots_and_omitted_twigs_do_not_move_remaining_branches(self):
        for name, levels in self.branches.items():
            if not name.startswith(("jungle_", "dead_jungle_", "dead_tall_")):
                continue
            full = {tuple(p.ravel()) for p in levels[0]}
            for lod in (1, 2):
                with self.subTest(model=name, lod=lod):
                    self.assertLessEqual({tuple(p.ravel()) for p in levels[lod]}, full)

    def test_lower_levels_still_reduce_triangle_counts(self):
        for name, levels in self.models.items():
            counts = [m.triangles() for m in levels]
            with self.subTest(model=name, counts=counts):
                self.assertLess(counts[1], counts[0])
                self.assertLessEqual(counts[2], counts[1])
                self.assertLess(counts[2], counts[0] * 0.75)


if __name__ == "__main__":
    unittest.main()
