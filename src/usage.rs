use crate::auth::Session;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

const URL: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Window {
	pub used_percent: f64,
	pub limit_window_seconds: u64,
	pub reset_at: i64,
}

#[derive(Deserialize)]
struct Usage {
	rate_limit: Option<Limits>,
}

#[derive(Deserialize)]
struct Limits {
	primary_window: Option<Window>,
	secondary_window: Option<Window>,
}

impl Usage {
	fn windows(self) -> Vec<Window> {
		self.rate_limit
			.into_iter()
			.flat_map(|limit| [limit.primary_window, limit.secondary_window])
			.flatten()
			.filter(|w| {
				(0.0..=100.0).contains(&w.used_percent)
					&& w.limit_window_seconds > 0
					&& w.reset_at > 0
			})
			.collect()
	}
}

pub enum Error {
	Unauthorized,
	Other(&'static str),
}

pub async fn fetch(client: &Client, session: &Session) -> Result<Vec<Window>, Error> {
	let response = client
		.get(URL)
		.bearer_auth(&session.access)
		.header("ChatGPT-Account-Id", &session.secret.account)
		.header("Accept", "application/json")
		.send()
		.await
		.map_err(|_| Error::Other("Cannot reach OpenAI"))?;
	match response.status() {
		StatusCode::UNAUTHORIZED => Err(Error::Unauthorized),
		StatusCode::FORBIDDEN => Err(Error::Other("Quota access denied")),
		StatusCode::TOO_MANY_REQUESTS => Err(Error::Other("OpenAI rate limited the lookup")),
		status if !status.is_success() => Err(Error::Other("OpenAI quota unavailable")),
		_ => response
			.json::<Usage>()
			.await
			.map(Usage::windows)
			.map_err(|_| Error::Other("Invalid quota response")),
	}
}

pub fn label(seconds: u64) -> String {
	for (unit, suffix) in [(86400, "d"), (3600, "h"), (60, "m")] {
		if seconds.is_multiple_of(unit) {
			return format!("{}{}", seconds / unit, suffix);
		}
	}
	format!("{seconds}s")
}

pub fn selected(windows: &[Window], duration: Option<u64>) -> Option<&Window> {
	match duration {
		Some(duration) => windows
			.iter()
			.find(|window| window.limit_window_seconds == duration),
		None => windows.first(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn single_weekly_window_matches_probe() {
		let windows = serde_json::from_str::<Usage>(r#"{"rate_limit":{"primary_window":null,"secondary_window":{"used_percent":59,"limit_window_seconds":604800,"reset_at":1795798379}}}"#)
			.unwrap().windows();
		assert_eq!(selected(&windows, None).unwrap().used_percent, 59.0);
		assert_eq!(label(windows[0].limit_window_seconds), "7d");
		assert!(selected(&windows, Some(18000)).is_none());
	}

	#[test]
	fn absent_quota_is_not_zero() {
		assert!(serde_json::from_str::<Usage>(r#"{"rate_limit":null}"#)
			.unwrap()
			.windows()
			.is_empty());
	}
}
