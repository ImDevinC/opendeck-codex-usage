# Codex Usage for OpenDeck

Standalone Rust/OpenAction plugin. Do not add a dependency on Codex, OpenCode,
browser cookies, or a helper application.

- `src/main.rs`: serialized refresh worker and OpenAction events. One session
  and quota cache serve all visible keys. Keep credentials out of OpenAction
  settings, logs, and property-inspector messages.
- `src/auth.rs`: legacy OAuth with a loopback callback and PKCE. The keyring
  contains only the refresh token and account ID. Persist rotated refresh
  tokens before using the new access token. Do not replace keyring failures
  with plaintext storage.
- `src/usage.rs`: OpenAI's internal Codex quota endpoint. Missing windows are
  not zero usage.
- `src/render.rs`: 144×144 SVG key images. Window choice and percentage
  visibility are per key.
- `assets/`: manifest, property inspector, and icon. The bundle ID is
  `com.imdevinc.codexusage`. Keep the manifest and Cargo versions in sync.
- `probe.py`: optional diagnostic, not part of the plugin bundle.

Run `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`,
and `sh package.sh` after relevant changes. Update the README for user-facing
changes.
