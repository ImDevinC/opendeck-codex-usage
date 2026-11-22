"""One-shot validation of direct Codex sign-in and quota retrieval."""

import argparse
import base64
import hashlib
import hmac
import http.server
import json
import re
import secrets
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from datetime import datetime, timezone
from typing import Any

ISSUER = "https://auth.openai.com"
CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"  # Public legacy Codex client ID.
USAGE_URL = "https://chatgpt.com/backend-api/wham/usage"
CALLBACK_HOST = "127.0.0.1"
CALLBACK_PORT = 1455


class ProbeError(Exception):
    """A diagnostic safe to display without exposing credentials."""


def base64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def callback_code(target: str, state: str) -> tuple[bool, str | None]:
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(target).query)
    actual_state = query.get("state", [])
    if (
        len(actual_state) != 1
        or not actual_state[0].isascii()
        or not hmac.compare_digest(actual_state[0], state)
    ):
        return False, None
    codes = query.get("code", [])
    if "error" in query or len(codes) != 1:
        return True, None
    return True, codes[0]


class CallbackServer(http.server.HTTPServer):
    oauth_state: str
    result: tuple[bool, str | None] = (False, None)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(2)
        return connection, address


class CallbackHandler(http.server.BaseHTTPRequestHandler):
    server: CallbackServer

    def do_GET(self) -> None:
        if self.headers.get("Host") != f"{CALLBACK_HOST}:{self.server.server_port}":
            self.reply(400, "Invalid callback host.")
        elif self.path.partition("?")[0] != "/auth/callback":
            self.reply(404, "Not found.")
        else:
            try:
                result = callback_code(self.path, self.server.oauth_state)
            except ValueError:
                self.reply(400, "Invalid callback request.")
                return
            accepted, code = result
            if not accepted:
                self.reply(400, "Invalid OAuth state. Return to the sign-in tab.")
                return
            self.server.result = result
            if code is None:
                self.reply(400, "OpenAI sign-in did not complete.")
            else:
                self.reply(200, "Sign-in received. Check the terminal for the quota result.")

    def reply(self, status: int, message: str) -> None:
        body = message.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Referrer-Policy", "no-referrer")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        # The request path can contain the authorization code.
        pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def request_json(request: urllib.request.Request, label: str) -> dict[str, Any]:
    try:
        opener = urllib.request.build_opener(NoRedirect())
        with opener.open(request, timeout=15) as response:
            payload = json.loads(response.read(1024 * 1024))
    except urllib.error.HTTPError as exc:
        with exc:
            raise ProbeError(f"{label} returned HTTP {exc.code}.") from None
    except (urllib.error.URLError, TimeoutError):
        raise ProbeError(f"{label} could not connect to OpenAI.") from None
    except ValueError:
        raise ProbeError(f"{label} returned invalid JSON.") from None
    if not isinstance(payload, dict):
        raise ProbeError(f"{label} returned an unexpected response.")
    return payload


def usage_windows(payload: dict[str, Any]) -> list[dict[str, int]]:
    limits = payload.get("rate_limit")
    if not isinstance(limits, dict):
        raise ProbeError("OpenAI did not return a quota limit for this account.")
    windows = []
    for name in ("primary_window", "secondary_window"):
        window = limits.get(name)
        if window is None:
            continue
        if not isinstance(window, dict):
            raise ProbeError("OpenAI returned an invalid quota window.")
        percent, seconds, reset = (
            window.get(key) for key in ("used_percent", "limit_window_seconds", "reset_at")
        )
        if (
            type(percent) is not int or not 0 <= percent <= 100
            or type(seconds) is not int or seconds <= 0
            or type(reset) is not int or not 0 < reset <= 253402300799
        ):
            raise ProbeError("OpenAI returned an invalid quota window.")
        windows.append({"used_percent": percent, "window_seconds": seconds, "reset_at": reset})
    if not windows:
        raise ProbeError("OpenAI did not return any quota windows for this account.")
    return windows


def show_usage(windows: list[dict[str, int]]) -> None:
    for window in windows:
        seconds = window["window_seconds"]
        if seconds % 86400 == 0:
            duration = f"{seconds // 86400}d"
        elif seconds % 3600 == 0:
            duration = f"{seconds // 3600}h"
        elif seconds % 60 == 0:
            duration = f"{seconds // 60}m"
        else:
            duration = f"{seconds}s"
        reset = datetime.fromtimestamp(window["reset_at"], timezone.utc)
        used = window["used_percent"]
        print(f"{duration}: {used}% used ({100 - used}% remaining), resets {reset:%Y-%m-%d %H:%M:%S UTC}")


def authorize_url(redirect: str, verifier: str, state: str) -> str:
    challenge = base64url(hashlib.sha256(verifier.encode("ascii")).digest())
    query = urllib.parse.urlencode({
        "response_type": "code",
        "client_id": CLIENT_ID,
        "redirect_uri": redirect,
        "scope": "openid profile email offline_access",
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "state": state,
        "id_token_add_organizations": "true",
        "codex_cli_simplified_flow": "true",
        "originator": "opendeck_codex_usage",
    })
    return f"{ISSUER}/oauth/authorize?{query}"


def get_code(verifier: str, state: str, open_browser: bool) -> tuple[str, str]:
    try:
        server = CallbackServer((CALLBACK_HOST, CALLBACK_PORT), CallbackHandler)
    except OSError:
        raise ProbeError(f"Cannot listen on {CALLBACK_HOST}:{CALLBACK_PORT}.") from None
    with server:
        server.oauth_state = state
        server.timeout = 0.5
        redirect = f"http://{CALLBACK_HOST}:{server.server_port}/auth/callback"
        url = authorize_url(redirect, verifier, state)
        if open_browser and webbrowser.open(url):
            print("Complete the OpenAI sign-in in your browser.", flush=True)
        else:
            print(f"Open this link in a browser on this machine:\n{url}", flush=True)
        deadline = time.monotonic() + 180
        while not server.result[0] and time.monotonic() < deadline:
            server.handle_request()
        received, code = server.result
    if code is None:
        message = "OpenAI sign-in did not complete." if received else "No OAuth callback within 3 minutes."
        raise ProbeError(message)
    return code, redirect


def account_id(id_token: object) -> str:
    if isinstance(id_token, str):
        try:
            _, payload, _ = id_token.split(".")
            claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
            auth = claims["https://api.openai.com/auth"]
            value = auth["chatgpt_account_id"]
            # This is a routing hint from the HTTPS token exchange, not local ID validation.
            if isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]+", value):
                return value
        except (ValueError, KeyError, TypeError):
            pass
    raise ProbeError("The OpenAI token did not contain a usable ChatGPT account ID.")


def fetch_usage(code: str, redirect: str, verifier: str) -> list[dict[str, int]]:
    form = urllib.parse.urlencode({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "redirect_uri": redirect,
        "code": code,
        "code_verifier": verifier,
    }).encode("ascii")
    token = request_json(urllib.request.Request(
        f"{ISSUER}/oauth/token",
        data=form,
        headers={"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"},
    ), "OAuth token exchange")
    access = token.get("access_token")
    if not isinstance(access, str) or not re.fullmatch(r"[A-Za-z0-9._~+/-]+=*", access):
        raise ProbeError("OpenAI did not return a usable access token.")
    return usage_windows(request_json(urllib.request.Request(
        USAGE_URL,
        headers={
            "Authorization": f"Bearer {access}",
            "ChatGPT-Account-Id": account_id(token.get("id_token")),
            "Accept": "application/json",
            "User-Agent": "opendeck-codex-usage-probe",
        },
    ), "Subscription quota lookup"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-browser", action="store_true", help="Print the local sign-in link instead.")
    args = parser.parse_args()
    verifier = secrets.token_urlsafe(48)
    state = secrets.token_urlsafe(32)
    try:
        code, redirect = get_code(verifier, state, not args.no_browser)
        show_usage(fetch_usage(code, redirect, verifier))
    except ProbeError as exc:
        print(f"Probe failed: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Sign-in canceled.", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    sys.exit(main())
