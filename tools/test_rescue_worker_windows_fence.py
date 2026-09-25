import pathlib
import sys
import unittest

from tools.probe_rescue_worker_windows_fence import acceptance


@unittest.skipUnless(sys.platform == "win32", "Windows Job Object acceptance")
class RescueWorkerWindowsFenceTests(unittest.TestCase):
    def test_job_object_fence_emits_all_required_typed_outcomes(self):
        result = acceptance()
        self.assertTrue(result["source_unchanged"])
        self.assertEqual(
            {
                "success": "succeeded",
                "sleep": "timed_out",
                "cancel": "cancelled",
                "memory": "resource_limited",
                "cpu": "resource_limited",
                "output": "resource_limited",
                "crash": "failed",
            },
            result["typed_outcomes"],
        )
        self.assertTrue(result["job_object"]["kill_on_close"])
        self.assertTrue(result["job_object"]["process_memory_limit"])
        self.assertTrue(result["job_object"]["process_cpu_time_limit"])


if __name__ == "__main__":
    unittest.main()
