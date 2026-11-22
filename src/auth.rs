use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT: &str = "http://127.0.0.1:1455/auth/callback";
const SERVICE: &str = "com.imdevinc.codexusage";

pub type Result<T> = std::result::Result<T, &'static str>;

#[derive(Serialize, Deserialize)]
pub struct Secret {
	pub refresh: String,
	pub account: String,
}

pub struct Session {
	pub secret: Secret,
	pub access: String,
	pub expires: Instant,
}

#[derive(Deserialize)]
struct Tokens {
	access_token: String,
	refresh_token: Option<String>,
	id_token: Option<String>,
	expires_in: Option<u64>,
}

fn entry() -> Result<keyring::Entry> {
	keyring::Entry::new(SERVICE, "chatgpt").map_err(|_| "Credential store unavailable")
}

pub async fn load() -> Result<Option<Secret>> {
	tokio::task::spawn_blocking(|| match entry()?.get_password() {
		Ok(value) => serde_json::from_str(&value)
			.map(Some)
			.map_err(|_| "Invalid saved sign-in"),
		Err(keyring::Error::NoEntry) => Ok(None),
		Err(_) => Err("Cannot read credential store"),
	})
	.await
	.map_err(|_| "Credential store unavailable")?
}

pub async fn clear() -> Result<()> {
	tokio::task::spawn_blocking(|| match entry()?.delete_credential() {
		Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
		Err(_) => Err("Cannot clear credential store"),
	})
	.await
	.map_err(|_| "Credential store unavailable")?
}

async fn save(secret: &Secret) -> Result<()> {
	let value = serde_json::to_string(secret).map_err(|_| "Cannot encode sign-in")?;
	tokio::task::spawn_blocking(move || {
		entry()?
			.set_password(&value)
			.map_err(|_| "Cannot save to credential store")
	})
	.await
	.map_err(|_| "Credential store unavailable")?
}

fn account_id(jwt: &str) -> Result<String> {
	let payload = jwt.split('.').nth(1).ok_or("Invalid identity token")?;
	let bytes = URL_SAFE_NO_PAD
		.decode(payload)
		.map_err(|_| "Invalid identity token")?;
	let claims: serde_json::Value =
		serde_json::from_slice(&bytes).map_err(|_| "Invalid identity token")?;
	// Use this claim only as a routing hint. OpenAI authenticates the bearer token.
	claims["https://api.openai.com/auth"]["chatgpt_account_id"]
		.as_str()
		.filter(|value| {
			!value.is_empty()
				&& value
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
		})
		.map(str::to_owned)
		.ok_or("No ChatGPT account ID")
}

async fn exchange(request: reqwest::RequestBuilder) -> Result<Tokens> {
	let response = request
		.send()
		.await
		.map_err(|_| "Cannot reach OpenAI sign-in")?;
	match response.status() {
		StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED => Err("Sign-in rejected. Reconnect."),
		status if !status.is_success() => Err("OpenAI sign-in unavailable"),
		_ => response
			.json()
			.await
			.map_err(|_| "Invalid sign-in response"),
	}
}

async fn session(tokens: Tokens, previous: Option<&Secret>) -> Result<Session> {
	let account = match tokens.id_token.as_deref() {
		Some(jwt) => account_id(jwt)?,
		None => previous.ok_or("No identity token")?.account.clone(),
	};
	if previous.is_some_and(|secret| secret.account != account) {
		return Err("ChatGPT account changed. Reconnect.");
	}
	let refresh = tokens
		.refresh_token
		.or_else(|| previous.map(|secret| secret.refresh.clone()))
		.filter(|token| !token.is_empty())
		.ok_or("No refresh token")?;
	if tokens.access_token.is_empty() {
		return Err("No access token");
	}
	let secret = Secret { refresh, account };
	if previous.is_none_or(|old| old.refresh != secret.refresh) {
		save(&secret).await?;
	}
	let seconds = tokens.expires_in.unwrap_or(3600).clamp(1, 86400);
	Ok(Session {
		secret,
		access: tokens.access_token,
		expires: Instant::now() + Duration::from_secs(seconds),
	})
}

pub async fn refresh(client: &Client, secret: &Secret) -> Result<Session> {
	let tokens = exchange(
		client
			.post(format!("{ISSUER}/oauth/token"))
			.json(&serde_json::json!({
				"grant_type": "refresh_token",
				"client_id": CLIENT_ID,
				"refresh_token": secret.refresh,
				"scope": "openid profile email",
			})),
	)
	.await?;
	session(tokens, Some(secret)).await
}

fn authorize(verifier: &str, state: &str) -> Url {
	let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier));
	let mut url = Url::parse(&format!("{ISSUER}/oauth/authorize")).expect("constant URL");
	url.query_pairs_mut().extend_pairs([
		("response_type", "code"),
		("client_id", CLIENT_ID),
		("redirect_uri", REDIRECT),
		("scope", "openid profile email offline_access"),
		("code_challenge", &challenge),
		("code_challenge_method", "S256"),
		("state", state),
		("id_token_add_organizations", "true"),
		("codex_cli_simplified_flow", "true"),
		("originator", "opendeck_codex_usage"),
	]);
	url
}

pub async fn connect(client: &Client) -> Result<Session> {
	let listener = TcpListener::bind(("127.0.0.1", 1455))
		.await
		.map_err(|_| "Cannot open local port 1455")?;
	let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
	let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
	openaction::open_url(authorize(&verifier, &state).into())
		.await
		.map_err(|_| "Cannot open browser")?;
	let code = timeout(Duration::from_secs(180), receive_code(&listener, &state))
		.await
		.map_err(|_| "Sign-in timed out")??;
	drop(listener);
	let tokens = exchange(client.post(format!("{ISSUER}/oauth/token")).form(&[
		("grant_type", "authorization_code"),
		("client_id", CLIENT_ID),
		("redirect_uri", REDIRECT),
		("code", &code),
		("code_verifier", &verifier),
	]))
	.await?;
	session(tokens, None).await
}

fn callback(target: &str, state: &str) -> Option<Result<String>> {
	let query = target.strip_prefix("/auth/callback?")?;
	let mut actual = None;
	let mut code = None;
	let mut denied = false;
	let url = Url::parse(&format!("{REDIRECT}?{query}")).ok()?;
	for (name, value) in url.query_pairs() {
		match name.as_ref() {
			"state" if actual.is_none() => actual = Some(value),
			"state" => return None,
			"code" if code.is_none() => code = Some(value.into_owned()),
			"code" | "error" => denied = true,
			_ => {}
		}
	}
	if actual.as_deref() != Some(state) {
		return None;
	}
	Some(
		code.filter(|value| !denied && !value.is_empty())
			.ok_or("OpenAI sign-in did not complete"),
	)
}

async fn read_callback(stream: &mut TcpStream, state: &str) -> Option<Result<String>> {
	let mut bytes = [0u8; 8192];
	let mut used = 0;
	while used < bytes.len() {
		let count = stream.read(&mut bytes[used..]).await.ok()?;
		if count == 0 {
			return None;
		}
		used += count;
		let mut headers = [httparse::EMPTY_HEADER; 32];
		let mut request = httparse::Request::new(&mut headers);
		if request.parse(&bytes[..used]).ok()?.is_partial() {
			continue;
		}
		let mut hosts = request
			.headers
			.iter()
			.filter(|header| header.name.eq_ignore_ascii_case("host"));
		if request.method != Some("GET")
			|| hosts.next()?.value != b"127.0.0.1:1455"
			|| hosts.next().is_some()
		{
			return None;
		}
		return callback(request.path?, state);
	}
	None
}

async fn receive_code(listener: &TcpListener, state: &str) -> Result<String> {
	loop {
		let (mut stream, _) = listener
			.accept()
			.await
			.map_err(|_| "Local sign-in failed")?;
		let result = timeout(Duration::from_secs(2), read_callback(&mut stream, state))
			.await
			.ok()
			.flatten();
		let (status, body) = match &result {
			Some(Ok(_)) => ("200 OK", "Sign-in received. Return to OpenDeck."),
			_ => (
				"400 Bad Request",
				"Invalid or canceled sign-in request. Return to OpenDeck.",
			),
		};
		let reply = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
		let _ = timeout(Duration::from_secs(2), stream.write_all(reply.as_bytes())).await;
		if let Some(result) = result {
			return result;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn authorize_uses_tested_redirect_and_pkce() {
		let url = authorize("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "state");
		let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
		assert_eq!(params["redirect_uri"], REDIRECT);
		assert_eq!(
			params["code_challenge"],
			"E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
		);
	}

	#[test]
	fn callback_requires_one_matching_state_and_one_code() {
		assert_eq!(
			callback("/auth/callback?state=x&code=y", "x"),
			Some(Ok("y".into()))
		);
		assert_eq!(callback("/auth/callback?state=x&code=y", "z"), None);
		assert_eq!(callback("/auth/callback?state=x&state=x&code=y", "x"), None);
		assert!(matches!(
			callback("/auth/callback?state=x&code=y&code=z", "x"),
			Some(Err(_))
		));
	}
}
