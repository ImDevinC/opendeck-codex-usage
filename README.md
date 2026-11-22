# Codex Usage for OpenDeck

Shows ChatGPT subscription Codex quota on an OpenDeck key. It does not show
OpenAI API spend. No Codex, OpenCode, Python, or helper app is needed at runtime.

## Build and install

Build with stable Rust and `zip`. On Linux, install the D-Bus development
headers (`libdbus-1-dev` on Debian/Ubuntu). At runtime, the Linux desktop must
have an available Secret Service credential store, such as GNOME Keyring.

```sh
sh package.sh
```

In OpenDeck, use **Install from file** and select `com.imdevinc.codexusage.zip`.
Or run `sh package.sh stage` and copy the `.sdPlugin` directory into the
OpenDeck `plugins` directory.

## Use

1. Add **Codex → Usage** to your deck.
2. Select **Sign in with ChatGPT** in the property inspector. Complete sign-in
   in your browser on the same machine.
3. Select a quota window and choose whether to show the percentage. The
   available windows depend on your account. Use separate keys to show
   different windows.

The property inspector uses the webview's default dark-mode text color.

The ring shows percentage used. It is green below 50%, yellow below 85%, and
red at 85% or more. The countdown updates every 30 seconds. OpenDeck requests
quota at most once a minute across visible keys. Press a key to refresh now.
A gray ring marked **STALE** shows the last available data after a failed
refresh or when the cached reset time has passed.

The plugin binds to `127.0.0.1:1455` during sign-in, uses PKCE, and checks OAuth
state. It uses the public client ID from [Codex's legacy login flow](https://github.com/openai/codex/blob/main/codex-rs/login/src/server.rs)
and calls the internal [ChatGPT quota endpoint](https://github.com/steipete/CodexBar/blob/main/docs/codex-oauth.md).
OpenAI does not support this as a public integration.
Sign-in or quota access can stop working without notice.

The plugin keeps the access token in memory. It stores the refresh token and
account ID in the OS credential store, not in OpenDeck settings. **Disconnect**
removes the local entry. It does not revoke a server-side session. The plugin
does not read browser cookies or other applications' credentials.

## Development

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

For a one-shot diagnostic, run `python3 probe.py` with Python 3.10 or newer.
It does not save tokens. Compare the result with the
[Codex usage dashboard](https://chatgpt.com/codex/settings/usage).
