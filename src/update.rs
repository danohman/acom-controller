//! Check GitHub releases for a newer version of the program.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const LATEST_URL: &str = "https://api.github.com/repos/danohman/acom-controller/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed(String),
}

/// Split "v1.2.3-beta+meta" into numeric components and an optional pre-release tag.
fn parse_version(s: &str) -> (Vec<u64>, Option<String>) {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split('+').next().unwrap_or(s);
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p.to_string())),
        None => (s, None),
    };
    (core.split('.').map_while(|p| p.parse::<u64>().ok()).collect(), pre)
}

/// True when `latest` is a strictly newer version than `current`.
/// A final release is newer than a pre-release of the same number (1.0.0 > 1.0.0-beta).
pub fn is_newer(latest: &str, current: &str) -> bool {
    let ((a, pa), (b, pb)) = (parse_version(latest), parse_version(current));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    match (pa, pb) {
        (None, Some(_)) => true,
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

fn fetch() -> Result<UpdateStatus, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let body = agent
        .get(LATEST_URL)
        .header("User-Agent", concat!("acom-controller/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            // No release published yet.
            ureq::Error::StatusCode(404) => "no releases published yet".to_string(),
            e => e.to_string(),
        })?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("unexpected reply from GitHub")?;
    let url = v["html_url"].as_str().unwrap_or("https://github.com/danohman/acom-controller/releases").to_string();
    if is_newer(tag, env!("CARGO_PKG_VERSION")) {
        Ok(UpdateStatus::Available { version: tag.trim_start_matches(['v', 'V']).to_string(), url })
    } else {
        Ok(UpdateStatus::UpToDate)
    }
}

/// Start a background check; the result arrives on the returned channel.
pub fn spawn_check(ctx: eframe::egui::Context) -> Receiver<UpdateStatus> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let st = fetch().unwrap_or_else(UpdateStatus::Failed);
        let _ = tx.send(st);
        ctx.request_repaint();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        // A final release supersedes its own beta, but not the other way round.
        assert!(is_newer("v1.0.0", "1.0.0-beta"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("v1.0.0-beta", "1.0.0-beta"));
        assert!(is_newer("1.0.0-beta.2", "1.0.0-beta.1"));
        assert!(is_newer("1.0.1-beta", "1.0.0"));
    }
}
