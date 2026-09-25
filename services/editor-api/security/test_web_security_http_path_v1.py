#!/usr/bin/env python3
import json
import pathlib
import sys
import threading
import unittest
import urllib.request
from http.server import ThreadingHTTPServer

HERE=pathlib.Path(__file__).resolve().parent
EDITOR=HERE.parent
if str(EDITOR) not in sys.path:
    sys.path.insert(0,str(EDITOR))

from web_shell_http_harness import Handler


class WebSecurityHttpPathV1Tests(unittest.TestCase):
    def setUp(self):
        self.server=ThreadingHTTPServer(("127.0.0.1",0),Handler)
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True)
        self.thread.start()
        self.base=f"http://127.0.0.1:{self.server.server_address[1]}"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def request(self,path):
        request=urllib.request.Request(
            self.base+path,
            headers={"x-chaptera-principal-id":"synthetic-viewer"},
        )
        return urllib.request.urlopen(request,timeout=3)

    def test_scene_route_carries_security_headers_and_source_neutral_payload(self):
        with self.request("/v1/scenes/current") as response:
            body=json.loads(response.read().decode("utf-8"))
            self.assertEqual(200,response.status)
            csp=response.headers["content-security-policy"]
            self.assertIn("object-src 'none'",csp)
            self.assertIn("connect-src 'self'",csp)
            self.assertEqual("nosniff",response.headers["x-content-type-options"])
            self.assertEqual("no-referrer",response.headers["referrer-policy"])
            self.assertEqual("same-origin",response.headers["cross-origin-resource-policy"])
            encoded=json.dumps(body,sort_keys=True)
            self.assertNotIn("raw_pub_bytes",encoded)
            self.assertNotIn("source_path",encoded)
            self.assertNotIn("cfb_path",encoded)

    def test_health_route_also_gets_default_security_headers(self):
        with urllib.request.urlopen(self.base+"/health",timeout=3) as response:
            self.assertEqual("nosniff",response.headers["x-content-type-options"])
            self.assertIn("frame-ancestors 'none'",response.headers["content-security-policy"])


if __name__=="__main__":
    unittest.main()
