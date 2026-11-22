use crate::usage::{self, Window};
use base64::{engine::general_purpose::STANDARD, Engine};

fn countdown(seconds: i64) -> String {
	match seconds {
		..=59 => "now".into(),
		60..=3599 => format!("{} min.", seconds / 60),
		3600..=86399 => format!("{} hr", seconds / 3600),
		86400..=172799 => "1 day".into(),
		_ => format!("{} days", seconds / 86400),
	}
}

fn color(percent: f64) -> &'static str {
	if percent < 50.0 {
		"#2ecc71"
	} else if percent < 85.0 {
		"#f1c40f"
	} else {
		"#e74c3c"
	}
}

pub fn ring(window: &Window, now: i64, show_percent: bool, stale: bool) -> String {
	let percent = window.used_percent;
	let color = if stale { "#888888" } else { color(percent) };
	let duration = usage::label(window.limit_window_seconds).to_uppercase();
	let label = if stale {
		format!("{duration} STALE")
	} else {
		duration
	};
	let reset = countdown(window.reset_at.saturating_sub(now));
	let percent_text = if show_percent {
		format!(r#"<text y="102" fill="{color}" font-size="14">{percent:.0}%</text>"#)
	} else {
		String::new()
	};
	let arc = if percent > 0.0 {
		format!(
			r#"<circle cx="72" cy="72" r="56" pathLength="100" stroke="{color}" stroke-dasharray="{percent} 100" stroke-linecap="round" transform="rotate(-90 72 72)"/>"#
		)
	} else {
		String::new()
	};
	format!(
		r##"<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 144 144">
<g fill="none" stroke-width="12"><circle cx="72" cy="72" r="56" stroke="#3a3a3a"/>{arc}</g>
<g text-anchor="middle" font-family="sans-serif" font-weight="bold" transform="translate(72 0)">
<text y="56" font-size="11" fill="#9a9a9a">{label}</text>
<text y="80" font-size="24" fill="#ffffff">{reset}</text>{percent_text}</g></svg>"##
	)
}

pub fn message(title: &str, detail: &str) -> String {
	format!(
		r##"<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 144 144"><g text-anchor="middle" font-family="sans-serif">
<text x="72" y="73" fill="#f1c40f" font-size="15" font-weight="bold">{title}</text>
<text x="72" y="94" fill="#aaaaaa" font-size="12">{detail}</text></g></svg>"##
	)
}

pub fn data_uri(svg: &str) -> String {
	format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn colors_and_countdown_match_original() {
		assert_eq!(color(49.9), "#2ecc71");
		assert_eq!(color(50.0), "#f1c40f");
		assert_eq!(color(85.0), "#e74c3c");
		assert_eq!(countdown(59), "now");
		assert_eq!(countdown(86400), "1 day");
	}

	#[test]
	fn hiding_percent_keeps_countdown_and_marks_stale() {
		let window = Window {
			used_percent: 59.0,
			limit_window_seconds: 604800,
			reset_at: 3600,
		};
		let svg = ring(&window, 0, false, true);
		assert!(svg.contains("7D STALE") && svg.contains("1 hr"));
		assert!(!svg.contains("59%"));
	}
}
