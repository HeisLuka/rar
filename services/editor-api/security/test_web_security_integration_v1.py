#!/usr/bin/env python3
import json
import pathlib
import sys
import tempfile
import unittest

HERE=pathlib.Path(__file__).resolve().parent
ROOT=HERE.parents[2]
sys.path.insert(0,str(HERE))
sys.path.insert(0,str(ROOT/"tools"))

from authz_v1 import AuthzDenied, AuthzKernel
from tenant_isolation import issue_artifact_grant, issue_artifact_handle
from web_security_integration_v1 import (
    WebSecurityIntegrationError,
    assert_external_fetch_denied_v1,
    authorize_browser_resource_v1,
    browser_security_headers_v1,
    guard_browser_payload_v1,
    run_isolated_parse_worker_v1,
    sanitize_browser_active_content_v1,
)


SECRET=b"web-security-integration-test-secret"
TENANT="tenant-a"
DOC="doc-1"
PRINCIPAL="viewer-1"
ARTIFACT="resource-1"
HASH="a"*64


class WebSecurityIntegrationV1Tests(unittest.TestCase):
    def authz(self):
        authz=AuthzKernel()
        authz.set_role(
            tenant_id=TENANT,
            document_id=DOC,
            principal_id=PRINCIPAL,
            role="viewer",
        )
        return authz

    def grant(self,tenant=TENANT):
        handle=issue_artifact_handle(
            secret=SECRET,
            tenant_id=tenant,
            artifact_id=ARTIFACT,
            artifact_kind="resource",
            content_hash=HASH,
        )
        return issue_artifact_grant(
            secret=SECRET,
            tenant_id=tenant,
            artifact_handle=handle,
            now_epoch=1000,
            ttl_seconds=60,
        )

    def test_browser_headers_are_default_deny_for_active_external_capabilities(self):
        headers=browser_security_headers_v1()
        csp=headers["content-security-policy"]
        self.assertIn("object-src 'none'",csp)
        self.assertIn("frame-src 'none'",csp)
        self.assertIn("base-uri 'none'",csp)
        self.assertIn("connect-src 'self'",csp)
        self.assertEqual("nosniff",headers["x-content-type-options"])
        self.assertNotIn("https:",csp)
        self.assertNotIn("http:",csp)

    def test_browser_payload_fence_rejects_raw_source_carriers_recursively(self):
        guard_browser_payload_v1({
            "protocol_version":"chaptera.scene.v1",
            "resources":[{"resource_id":"r1"}],
        })
        for payload in (
            {"raw_pub_bytes":"AAAA"},
            {"node":{"source_path":"/secret/file.pub"}},
            {"items":[{"carrier":{"stream":"Contents"}}]},
        ):
            with self.subTest(payload=payload):
                with self.assertRaises(WebSecurityIntegrationError) as caught:
                    guard_browser_payload_v1(payload)
                self.assertEqual("browser_payload_rejected",caught.exception.code)

    def test_svg_and_html_are_sanitized_before_browser_delivery(self):
        safe=sanitize_browser_active_content_v1(
            kind="svg",
            body=b'<svg xmlns="http://www.w3.org/2000/svg"><rect x="0" y="0" width="1" height="1"/></svg>',
        )
        self.assertFalse(safe.receipt["network_fetch_allowed"])
        self.assertFalse(safe.receipt["active_content_allowed"])

        for kind,body in (
            ("svg",b'<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>'),
            ("svg",b'<svg xmlns="http://www.w3.org/2000/svg"><image href="https://evil.invalid/x"/></svg>'),
            ("html",b'<div onclick="fetch(\'https://evil.invalid\')">x</div>'),
            ("html",b'<iframe src="http://127.0.0.1/"></iframe>'),
        ):
            with self.subTest(kind=kind):
                with self.assertRaises(WebSecurityIntegrationError) as caught:
                    sanitize_browser_active_content_v1(kind=kind,body=body)
                self.assertEqual("active_content_rejected",caught.exception.code)

    def test_resource_delivery_requires_document_authz_and_tenant_grant(self):
        grant=self.grant()
        payload=authorize_browser_resource_v1(
            authz=self.authz(),
            tenant_secret=SECRET,
            tenant_id=TENANT,
            document_id=DOC,
            principal_id=PRINCIPAL,
            artifact_grant=grant,
            artifact_id=ARTIFACT,
            now_epoch=1050,
        )
        self.assertEqual(TENANT,payload["tenant_id"])

        with self.assertRaises(Exception):
            authorize_browser_resource_v1(
                authz=self.authz(),
                tenant_secret=SECRET,
                tenant_id="tenant-b",
                document_id=DOC,
                principal_id=PRINCIPAL,
                artifact_grant=grant,
                artifact_id=ARTIFACT,
                now_epoch=1050,
            )

        with self.assertRaises(Exception):
            authorize_browser_resource_v1(
                authz=self.authz(),
                tenant_secret=SECRET,
                tenant_id=TENANT,
                document_id=DOC,
                principal_id="unknown",
                artifact_grant=grant,
                artifact_id=ARTIFACT,
                now_epoch=1050,
            )

    def test_external_fetch_policy_denies_public_and_ssrf_targets(self):
        for url in (
            "https://example.com/x",
            "http://127.0.0.1/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/private",
            "file:///etc/passwd",
        ):
            assert_external_fetch_denied_v1(url)

    @unittest.skipUnless(sys.platform=="linux","real seccomp worker proof requires Linux")
    def test_parse_worker_runs_through_real_no_network_seccomp_harness(self):
        with tempfile.TemporaryDirectory() as td:
            root=pathlib.Path(td)
            input_path=root/"fixture.pub"
            input_path.write_bytes(bytes.fromhex("d0cf11e0a1b11ae1")+b"fixture")
            final=root/"published"
            script=(
                "import json,os,pathlib,socket;"
                "denied=False;"
                "\ntry:\n socket.socket()\nexcept PermissionError:\n denied=True\n"
                "\nif not denied: raise SystemExit(41)\n"
                "out=pathlib.Path(os.environ['CHAPTERA_WORKER_OUTPUT_DIR']);"
                "(out/'receipt.json').write_text(json.dumps({'network_denied':denied}),encoding='utf-8')"
            )
            receipt=run_isolated_parse_worker_v1(
                command=[sys.executable,"-c",script],
                input_path=input_path,
                final_output_dir=final,
                timeout_seconds=5,
            )
            self.assertEqual("success",receipt.status)
            self.assertEqual("seccomp_default_deny",receipt.network_policy)
            self.assertIn("receipt.json",receipt.outputs)
            body=json.loads((final/"receipt.json").read_text(encoding="utf-8"))
            self.assertTrue(body["network_denied"])
            self.assertLessEqual(receipt.limits["address_space_bytes"],512*1024*1024)
            self.assertLessEqual(receipt.limits["cpu_seconds"],10)


if __name__=="__main__":
    unittest.main()
