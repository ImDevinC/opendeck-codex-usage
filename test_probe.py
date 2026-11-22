import json
import threading
import unittest
from http.client import HTTPConnection
from urllib.parse import parse_qs, urlsplit

import probe


class ProbeTests(unittest.TestCase):
    def test_pkce_challenge_matches_rfc_7636(self):
        redirect = "http://127.0.0.1:1455/auth/callback"
        url = probe.authorize_url(redirect, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "state")
        params = parse_qs(urlsplit(url).query)
        self.assertEqual(params["code_challenge"], ["E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"])
        self.assertEqual(params["code_challenge_method"], ["S256"])
        self.assertEqual(params["redirect_uri"], [redirect])

    def test_callback_rejects_duplicate_state_and_code(self):
        self.assertEqual(probe.callback_code("/?state=s&state=s&code=c", "s"), (False, None))
        self.assertEqual(probe.callback_code("/?state=s&code=c&code=d", "s"), (True, None))

    def test_loopback_callback(self):
        with probe.CallbackServer((probe.CALLBACK_HOST, 0), probe.CallbackHandler) as server:
            server.oauth_state = "valid"
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                connection = HTTPConnection(*server.server_address, timeout=2)
                try:
                    for state, status in [("invalid", 400), ("valid", 200)]:
                        connection.request("GET", f"/auth/callback?state={state}&code=synthetic")
                        response = connection.getresponse()
                        self.assertEqual(response.status, status)
                        self.assertNotIn(b"synthetic", response.read())
                    self.assertEqual(server.result, (True, "synthetic"))
                finally:
                    connection.close()
            finally:
                server.shutdown()
                thread.join()

    def test_account_id_routing_claim(self):
        auth = {"chatgpt_account_id": "test-account"}
        payload = probe.base64url(json.dumps({"https://api.openai.com/auth": auth}).encode())
        self.assertEqual(probe.account_id(f"header.{payload}.signature"), "test-account")

    def test_returns_reported_windows_only(self):
        primary = {"used_percent": 0, "limit_window_seconds": 18000, "reset_at": 1735401600}
        result = probe.usage_windows({"rate_limit": {"primary_window": primary, "secondary_window": None}})
        self.assertEqual(result, [{"used_percent": 0, "window_seconds": 18000, "reset_at": 1735401600}])

    def test_missing_windows_are_not_zero_usage(self):
        with self.assertRaises(probe.ProbeError):
            probe.usage_windows({"rate_limit": {}})
