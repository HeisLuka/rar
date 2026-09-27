import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
GUARD_PATH = ROOT / "tools" / "check_public_boundary.py"


def load_guard():
    spec = importlib.util.spec_from_file_location("rar_public_boundary_guard", GUARD_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load public boundary guard")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PublicBoundaryGuardTests(unittest.TestCase):
    def setUp(self):
        self.guard = load_guard()

    def test_frozen_repository_mirror_path_is_rejected(self):
        violations = self.guard.path_violations("scratch/carlton-yab/Cargo.toml")
        self.assertTrue(any("yab" in item for item in violations))

    def test_normal_public_tool_path_is_allowed(self):
        self.assertEqual(self.guard.path_violations("tools/corpus/public_safe.py"), [])

    def test_private_pub_payload_location_is_rejected(self):
        violations = self.guard.path_violations("research/autonomus/private/sample.pub")
        self.assertTrue(any("raw .pub" in item for item in violations))

    def test_explicit_synthetic_pub_fixture_location_is_allowed(self):
        self.assertEqual(
            self.guard.path_violations(
                "research/autonomus/fixtures/synthetic/minimal.pub"
            ),
            [],
        )

    def test_windows_local_path_is_rejected(self):
        leaked = "D:" + "\\Downloads\\pub-evidence\\sample.pub"
        violations = self.guard.content_violations(
            "research/autonomus/receipt.json", '{"path": "' + leaked + '"}'
        )
        self.assertTrue(any("Windows absolute path" in item for item in violations))

    def test_standard_windows_system_path_is_not_treated_as_private(self):
        system_path = "C:" + "\\Windows\\System32\\kernel32.dll"
        self.assertEqual(
            self.guard.content_violations(
                "tools/platform_probe.py", 'DLL = r"' + system_path + '"'
            ),
            [],
        )

    def test_program_files_path_with_spaces_is_allowed(self):
        program_files = "C:" + "\\Program Files\\Chaptera\\tool.exe"
        self.assertEqual(
            self.guard.content_violations(
                "tools/platform_probe.py", 'EXE = r"' + program_files + '"'
            ),
            [],
        )

    def test_user_home_path_is_rejected_but_github_runner_path_is_allowed(self):
        leaked = "/home/" + "alice/Downloads/pub/sample.pub"
        violations = self.guard.content_violations(
            "packages/research/example/result.json", '{"path": "' + leaked + '"}'
        )
        self.assertTrue(any("user-home" in item for item in violations))

        self.assertEqual(
            self.guard.content_violations(
                "tools/ci_probe.py",
                'ROOT = "/home/runner/work/rar/rar"',
            ),
            [],
        )

    def test_high_confidence_token_literal_is_rejected(self):
        token = "ghp_" + ("A" * 40)
        violations = self.guard.content_violations(
            "research/autonomus/receipt.json", '{"token": "' + token + '"}'
        )
        self.assertTrue(any("credential/token" in item for item in violations))

    def test_private_key_block_is_rejected(self):
        key = "-----BEGIN " + "PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----"
        violations = self.guard.content_violations(
            "tools/research_key_probe.py", 'VALUE = """' + key + '"""'
        )
        self.assertTrue(any("private-key" in item for item in violations))

    def test_large_inline_pub_payload_is_rejected(self):
        payload = "ab" * 300
        violations = self.guard.content_violations(
            "research/autonomus/receipt.json",
            '{"pub_payload": "' + payload + '"}',
        )
        self.assertTrue(any("raw payload" in item for item in violations))

    def test_source_free_hash_metadata_is_allowed(self):
        digest = "a" * 64
        text = (
            '{"task_id": "PUB-T-746", "input_sha256": "' + digest + '", '
            '"scope": "source-free synthetic fixture"}'
        )
        self.assertEqual(
            self.guard.content_violations("research/autonomus/receipt.json", text),
            [],
        )

    def test_markdown_policy_docs_are_not_content_scanned(self):
        leaked = "D:" + "\\Downloads\\example.pub"
        self.assertEqual(
            self.guard.content_violations(
                "research/autonomus/POLICY.md", "example only: " + leaked
            ),
            [],
        )


if __name__ == "__main__":
    unittest.main()
