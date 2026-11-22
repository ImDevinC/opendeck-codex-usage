mod auth;
mod render;
mod usage;

use openaction::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
	window: Option<u64>,
	show_percent: bool,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			window: None,
			show_percent: true,
		}
	}
}

static KEYS: LazyLock<Mutex<HashMap<String, Settings>>> = LazyLock::new(Default::default);
static SENDER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();

enum Command {
	Show,
	Refresh,
	Connect,
	Connected(auth::Result<auth::Session>),
	Disconnect,
}

async fn queue(command: Command) {
	if let Some(sender) = SENDER.get() {
		let _ = sender.send(command).await;
	}
}

struct UsageAction;

#[async_trait]
impl Action for UsageAction {
	const UUID: ActionUuid = "com.imdevinc.codexusage.usage";
	type Settings = Settings;

	async fn will_appear(&self, instance: &Instance, settings: &Settings) -> OpenActionResult<()> {
		self.did_receive_settings(instance, settings).await
	}

	async fn will_disappear(&self, instance: &Instance, _: &Settings) -> OpenActionResult<()> {
		KEYS.lock().unwrap().remove(&instance.instance_id);
		Ok(())
	}

	async fn did_receive_settings(
		&self,
		instance: &Instance,
		settings: &Settings,
	) -> OpenActionResult<()> {
		KEYS.lock()
			.unwrap()
			.insert(instance.instance_id.clone(), settings.clone());
		queue(Command::Show).await;
		Ok(())
	}

	async fn key_up(&self, _: &Instance, _: &Settings) -> OpenActionResult<()> {
		queue(Command::Refresh).await;
		Ok(())
	}

	async fn send_to_plugin(
		&self,
		_: &Instance,
		_: &Settings,
		payload: &Value,
	) -> OpenActionResult<()> {
		match payload["command"].as_str() {
			Some("status") => queue(Command::Show).await,
			Some("connect") => queue(Command::Connect).await,
			Some("disconnect") => queue(Command::Disconnect).await,
			_ => {}
		}
		Ok(())
	}
}

#[derive(Default)]
struct State {
	session: Option<auth::Session>,
	connected: bool,
	connecting: bool,
	reconnect: bool,
	windows: Vec<usage::Window>,
	error: Option<&'static str>,
	last_attempt: Option<Instant>,
}

impl State {
	async fn restore(&mut self, client: &reqwest::Client) -> auth::Result<()> {
		if let Some(secret) = auth::load().await? {
			self.connected = true;
			self.session = Some(auth::refresh(client, &secret).await?);
		} else {
			self.connected = false;
		}
		Ok(())
	}

	async fn lookup(&mut self, client: &reqwest::Client) -> auth::Result<()> {
		if let Some(session) = &self.session {
			if session.expires <= Instant::now() + Duration::from_secs(30) {
				self.session = Some(auth::refresh(client, &session.secret).await?);
			}
		} else {
			self.restore(client).await?;
		}
		let Some(session) = &self.session else {
			return Ok(());
		};
		match usage::fetch(client, session).await {
			Ok(windows) => self.windows = windows,
			Err(usage::Error::Other(error)) => return Err(error),
			Err(usage::Error::Unauthorized) => {
				let renewed = auth::refresh(client, &session.secret).await?;
				self.session = Some(renewed);
				match usage::fetch(client, self.session.as_ref().unwrap()).await {
					Ok(windows) => self.windows = windows,
					Err(usage::Error::Unauthorized) => return Err("Sign-in rejected. Reconnect."),
					Err(usage::Error::Other(error)) => return Err(error),
				}
			}
		}
		Ok(())
	}

	async fn refresh(&mut self, client: &reqwest::Client, force: bool) {
		if self.connecting || self.reconnect {
			return;
		}
		if !force
			&& self
				.last_attempt
				.is_some_and(|t| t.elapsed() < Duration::from_secs(60))
		{
			return;
		}
		self.last_attempt = Some(Instant::now());
		self.error = self.lookup(client).await.err();
		self.reconnect = matches!(
			self.error,
			Some("Sign-in rejected. Reconnect." | "ChatGPT account changed. Reconnect.")
		);
	}

	async fn publish(&self) {
		let instances = visible_instances(UsageAction::UUID).await;
		let now = std::time::UNIX_EPOCH
			.elapsed()
			.unwrap_or_default()
			.as_secs() as i64;
		let status = if self.connecting {
			"Complete sign-in in your browser."
		} else if let Some(error) = self.error {
			error
		} else if !self.connected {
			"Sign in to see subscription usage."
		} else if self.windows.is_empty() {
			"No quota windows reported."
		} else {
			"Connected to ChatGPT."
		};
		let windows: Vec<_> = self
			.windows
			.iter()
			.map(|w| w.limit_window_seconds)
			.collect();
		let payload = json!({ "status": status, "windows": windows, "connected": self.connected, "connecting": self.connecting });
		for instance in instances {
			let settings = KEYS.lock().unwrap().get(&instance.instance_id).cloned();
			let Some(settings) = settings else {
				continue;
			};
			let svg = if self.connecting {
				render::message("SIGNING IN", "")
			} else if self.reconnect {
				render::message("RECONNECT", "")
			} else if let Some(window) = usage::selected(&self.windows, settings.window) {
				render::ring(
					window,
					now,
					settings.show_percent,
					self.error.is_some() || window.reset_at <= now,
				)
			} else if self.error.is_some() {
				render::message("LOOKUP ERROR", "Check settings")
			} else if !self.connected {
				render::message("SIGN IN", "Open settings")
			} else {
				render::message("NO WINDOW", "Check settings")
			};
			let _ = instance.set_image(Some(render::data_uri(&svg)), None).await;
			let _ = instance.send_to_property_inspector(&payload).await;
		}
	}
}

async fn work(mut receiver: mpsc::Receiver<Command>) {
	let client = reqwest::Client::builder()
		.timeout(Duration::from_secs(12))
		.redirect(reqwest::redirect::Policy::none())
		.user_agent("opendeck-codex-usage")
		.build()
		.expect("HTTP client");
	let mut state = State::default();
	let mut tick = tokio::time::interval(Duration::from_secs(30));
	tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
	loop {
		tokio::select! {
			_ = tick.tick() => {
				let visible = !KEYS.lock().unwrap().is_empty();
				if visible { state.refresh(&client, false).await; }
			}
			event = receiver.recv() => match event {
				Some(Command::Show) => state.refresh(&client, false).await,
				Some(Command::Refresh) => state.refresh(&client, true).await,
				Some(Command::Connect) if !state.connecting => {
					state.connecting = true;
					let client = client.clone();
					tokio::spawn(async move { queue(Command::Connected(auth::connect(&client).await)).await; });
				}
				Some(Command::Connected(result)) => {
					state.connecting = false;
					match result {
						Ok(session) => {
							state = State { session: Some(session), connected: true, ..State::default() };
							state.refresh(&client, true).await;
						}
						Err(error) => state.error = Some(error),
					}
				}
				Some(Command::Disconnect) if !state.connecting => match auth::clear().await {
					Ok(()) => state = State { last_attempt: Some(Instant::now()), ..State::default() },
					Err(error) => state.error = Some(error),
				}
				Some(_) => {}
				None => break,
			}
		}
		state.publish().await;
	}
}

#[tokio::main]
async fn main() -> OpenActionResult<()> {
	let (sender, receiver) = mpsc::channel(32);
	let _ = SENDER.set(sender);
	register_action(UsageAction).await;
	let worker = tokio::spawn(work(receiver));
	let result = run(std::env::args().collect()).await;
	worker.abort();
	result
}
