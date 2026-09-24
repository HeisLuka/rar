import inspect
import unittest

import nudge_plan_v1 as nudge
from nudge_plan_v1 import (
    BASE_NUDGE_EMU,
    COARSE_NUDGE_EMU,
    EMU_PER_INCH,
    NudgePlanError,
    plan_nudge_v1,
)


class NudgePlanV1Tests(unittest.TestCase):
    def test_base_constant_is_exactly_documented_point_thirteen_inch(self):
        self.assertEqual(EMU_PER_INCH * 13, BASE_NUDGE_EMU * 100)
        self.assertEqual(118_872, BASE_NUDGE_EMU)

    def test_coarse_constant_is_explicit_chaptera_ten_x_policy(self):
        self.assertEqual(BASE_NUDGE_EMU * 10, COARSE_NUDGE_EMU)
        self.assertEqual(1_188_720, COARSE_NUDGE_EMU)

    def test_base_vectors_are_exact_and_axis_aligned(self):
        expected = {
            "left": (-BASE_NUDGE_EMU, 0),
            "right": (BASE_NUDGE_EMU, 0),
            "up": (0, -BASE_NUDGE_EMU),
            "down": (0, BASE_NUDGE_EMU),
        }
        for direction, vector in expected.items():
            with self.subTest(direction=direction):
                plan = plan_nudge_v1(
                    direction=direction,
                    modifier_state="none",
                )
                self.assertEqual("chaptera.nudge-plan.v1", plan.protocol_version)
                self.assertEqual(direction, plan.direction)
                self.assertEqual("none", plan.modifier_state)
                self.assertEqual(BASE_NUDGE_EMU, plan.step_emu)
                self.assertEqual(vector, (plan.dx_emu, plan.dy_emu))

    def test_coarse_vectors_are_exact_and_axis_aligned(self):
        expected = {
            "left": (-COARSE_NUDGE_EMU, 0),
            "right": (COARSE_NUDGE_EMU, 0),
            "up": (0, -COARSE_NUDGE_EMU),
            "down": (0, COARSE_NUDGE_EMU),
        }
        for direction, vector in expected.items():
            with self.subTest(direction=direction):
                plan = plan_nudge_v1(
                    direction=direction,
                    modifier_state="coarse",
                )
                self.assertEqual("coarse", plan.modifier_state)
                self.assertEqual(COARSE_NUDGE_EMU, plan.step_emu)
                self.assertEqual(vector, (plan.dx_emu, plan.dy_emu))

    def test_same_input_is_deterministic(self):
        first = plan_nudge_v1(direction="right", modifier_state="coarse")
        second = plan_nudge_v1(direction="right", modifier_state="coarse")
        self.assertEqual(first, second)
        self.assertEqual(hash(first), hash(second))

    def test_policy_surface_has_no_screen_or_timing_inputs(self):
        signature = inspect.signature(plan_nudge_v1)
        self.assertEqual(
            {"direction", "modifier_state"},
            set(signature.parameters),
        )
        forbidden = {
            "zoom",
            "dpi",
            "pixel",
            "pointer",
            "time",
            "repeat",
            "selection",
            "focus",
            "snap",
        }
        source = inspect.getsource(plan_nudge_v1).lower()
        for word in forbidden:
            with self.subTest(word=word):
                self.assertNotIn(word, source)

    def test_unsupported_direction_and_modifier_fail_explicitly(self):
        with self.assertRaisesRegex(NudgePlanError, "direction"):
            plan_nudge_v1(direction="diagonal", modifier_state="none")
        for modifier in ("shift", "ctrl", "alt", "fine", "", None):
            with self.subTest(modifier=modifier):
                with self.assertRaisesRegex(NudgePlanError, "modifier"):
                    plan_nudge_v1(direction="left", modifier_state=modifier)

    def test_base_and_coarse_are_not_silently_configurable_at_call_site(self):
        signature = inspect.signature(plan_nudge_v1)
        self.assertNotIn("step_emu", signature.parameters)
        self.assertNotIn("distance", signature.parameters)
        self.assertNotIn("nudge_distance", signature.parameters)

    def test_constants_are_module_policy_not_mutable_plan_fields(self):
        plan = plan_nudge_v1(direction="down", modifier_state="none")
        with self.assertRaises(Exception):
            plan.step_emu = 1

    def test_every_plan_step_matches_vector_magnitude(self):
        for modifier in ("none", "coarse"):
            for direction in ("left", "right", "up", "down"):
                plan = plan_nudge_v1(
                    direction=direction,
                    modifier_state=modifier,
                )
                self.assertEqual(
                    plan.step_emu,
                    abs(plan.dx_emu) + abs(plan.dy_emu),
                )


if __name__ == "__main__":
    unittest.main()
