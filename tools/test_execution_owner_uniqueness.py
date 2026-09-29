import unittest

from execution_owner_uniqueness import (
    decide,
    normalize_authority,
    normalize_task_key,
    record_from_api,
)

AUTH_A = "3e932a84beec81d0bb7ac35f6e6ad35c"
AUTH_B = "3e932a84beec8173925bc086c4e82055"


def issue(number, key, authority, created):
    return record_from_api(
        {
            "number": number,
            "title": f"{key}: bounded owner",
            "body": f"Notion authority: https://app.notion.com/p/{authority}",
            "created_at": created,
        }
    )


def pr(number, key, authority, owner, created, supersedes=None):
    body = f"Owner: #{owner}\nNotion: https://app.notion.com/p/{authority}"
    if supersedes is not None:
        body += f"\nSupersedes: #{supersedes}"
    return record_from_api(
        {
            "number": number,
            "title": f"{key}: implementation",
            "body": body,
            "created_at": created,
            "pull_request": {"url": "x"},
        }
    )


class UniquenessTests(unittest.TestCase):
    def test_extract_authority_and_key(self):
        body = (
            "Notion owner: "
            "https://app.notion.com/p/3e932a84-beec-8173-925b-c086c4e82055?pvs=204"
        )
        self.assertEqual(normalize_authority(body), AUTH_B)
        self.assertEqual(
            normalize_task_key("[OPEN-FIRST-USEFUL-PAGE-HARNESS-01] replay"),
            "OPEN-FIRST-USEFUL-PAGE-HARNESS-01",
        )

    def test_table_issue_duplicate_closes_later(self):
        a = issue(1264, "VIEWER-TABLE-RENDER-01", AUTH_A, "2026-09-29T07:00:00Z")
        b = issue(1269, "VIEWER-TABLE-RENDER-01", AUTH_A, "2026-09-29T07:10:00Z")
        decision = decide([a, b], AUTH_A)
        self.assertEqual(decision.status, "duplicates_found")
        self.assertEqual(
            [(item.number, item.canonical_number) for item in decision.duplicates],
            [(1269, 1264)],
        )
        self.assertEqual(decision.ambiguities, [])

    def test_shell_issue_duplicate_closes_later(self):
        a = issue(
            1270,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:20:00Z",
        )
        b = issue(
            1273,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:25:00Z",
        )
        decision = decide([a, b], AUTH_B)
        self.assertEqual([item.number for item in decision.duplicates], [1273])

    def test_parent_issue_and_paired_pr_are_valid(self):
        issue_record = issue(
            1243,
            "VIEWER-PAINT-BRIDGE-CONSUME-01",
            AUTH_A,
            "2026-09-29T06:00:00Z",
        )
        pr_record = pr(
            1248,
            "VIEWER-PAINT-BRIDGE-CONSUME-01",
            AUTH_A,
            1243,
            "2026-09-29T06:10:00Z",
        )
        decision = decide([issue_record, pr_record], AUTH_A)
        self.assertEqual(decision.status, "valid_issue_pr_pair")
        self.assertEqual(decision.duplicates, [])
        self.assertEqual(decision.ambiguities, [])
        self.assertEqual(decision.canonical_pr, 1248)

    def test_shell_parent_and_pr_are_valid(self):
        issue_record = issue(
            1270,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:20:00Z",
        )
        pr_record = pr(
            1279,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            1270,
            "2026-09-29T07:44:00Z",
        )
        decision = decide([issue_record, pr_record], AUTH_B)
        self.assertEqual(decision.status, "valid_issue_pr_pair")
        self.assertEqual(decision.ambiguities, [])

    def test_two_prs_for_same_owner_close_later(self):
        issue_record = issue(
            1270,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:20:00Z",
        )
        first = pr(
            1279,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            1270,
            "2026-09-29T07:44:00Z",
        )
        second = pr(
            1282,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            1270,
            "2026-09-29T07:50:00Z",
        )
        decision = decide([issue_record, first, second], AUTH_B)
        self.assertEqual(
            [(item.number, item.canonical_number) for item in decision.duplicates],
            [(1282, 1279)],
        )

    def test_pr_without_owner_is_ambiguous(self):
        issue_record = issue(
            1270,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:20:00Z",
        )
        pr_record = record_from_api(
            {
                "number": 1279,
                "title": "READER-WIN-SHELL-ACTIVATION-01: implementation",
                "body": f"Notion: https://app.notion.com/p/{AUTH_B}",
                "created_at": "2026-09-29T07:44:00Z",
                "pull_request": {"url": "x"},
            }
        )
        decision = decide([issue_record, pr_record], AUTH_B)
        self.assertEqual(decision.status, "ambiguous")
        self.assertTrue(
            any("no exact Owner" in item for item in decision.ambiguities)
        )

    def test_duplicate_issue_with_attached_pr_is_ambiguous(self):
        older = issue(
            1270,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:20:00Z",
        )
        later = issue(
            1273,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:25:00Z",
        )
        attached = pr(
            1280,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            1273,
            "2026-09-29T07:30:00Z",
        )
        decision = decide([older, later, attached], AUTH_B)
        self.assertEqual(decision.status, "ambiguous")
        self.assertEqual(decision.duplicates, [])
        self.assertTrue(
            any("live implementation PR" in item for item in decision.ambiguities)
        )

    def test_live_supersession_is_ambiguous_not_autoclosed(self):
        issue_record = issue(
            1270,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            "2026-09-29T07:20:00Z",
        )
        first = pr(
            1279,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            1270,
            "2026-09-29T07:44:00Z",
        )
        second = pr(
            1282,
            "READER-WIN-SHELL-ACTIVATION-01",
            AUTH_B,
            1270,
            "2026-09-29T07:50:00Z",
            supersedes=1279,
        )
        decision = decide([issue_record, first, second], AUTH_B)
        self.assertEqual(decision.status, "ambiguous")
        self.assertEqual(decision.duplicates, [])

    def test_similar_title_distinct_authority_is_not_duplicate(self):
        a = issue(1300, "READER-SOMETHING-01", AUTH_A, "2026-09-29T08:00:00Z")
        b = issue(1301, "READER-SOMETHING-01", AUTH_B, "2026-09-29T08:01:00Z")
        decision = decide([a, b], AUTH_A)
        self.assertEqual(decision.status, "unique")
        self.assertEqual(decision.duplicates, [])


if __name__ == "__main__":
    unittest.main()
