//! Discord Rich Presence (desktop): shows cover, track title, source, and a
//! listening progress bar (Discord animates it itself from timestamps).
//!
//! Frontend sends state (see useDiscordRpc.ts); backend just maintains the
//! IPC connection to the local Discord client. No-op on mobile (see stubs below).

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PresencePayload {
    /// Line 1: track title (with a pause marker if paused).
    pub details: String,
    /// Line 2: artist.
    pub state: String,
    /// Track cover https URL, or None to use the `logo` app asset.
    pub cover_url: Option<String>,
    /// Line 3: source (album/playlist/artist/catalog/mix).
    pub cover_text: Option<String>,
    pub duration_secs: f64,
    pub position_secs: f64,
    pub playing: bool,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod imp {
    use std::sync::Mutex;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use discord_rich_presence::activity::{Activity, ActivityType, Assets, Timestamps};
    use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

    use super::PresencePayload;

    const DISCORD_APP_ID: &str = "1514702255843053699";

    /// Minimum gap between reconnect attempts when Discord isn't running.
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);

    struct RpcState {
        client: Option<DiscordIpcClient>,
        last_failed_attempt: Option<Instant>,
    }

    static STATE: Mutex<RpcState> = Mutex::new(RpcState {
        client: None,
        last_failed_attempt: None,
    });

    fn ensure_connected(state: &mut RpcState) -> bool {
        if state.client.is_some() {
            return true;
        }
        if let Some(at) = state.last_failed_attempt {
            if at.elapsed() < RECONNECT_INTERVAL {
                return false;
            }
        }

        let connected = DiscordIpcClient::new(DISCORD_APP_ID)
            .ok()
            .and_then(|mut client| client.connect().ok().map(|_| client));
        match connected {
            Some(client) => {
                state.client = Some(client);
                state.last_failed_attempt = None;
                true
            }
            None => {
                // Discord not running or bad ID — retry no sooner than RECONNECT_INTERVAL.
                state.last_failed_attempt = Some(Instant::now());
                false
            }
        }
    }

    /// Discord requires 2-128 chars in details/state.
    fn clamp_line(s: &str) -> String {
        let mut out: String = s.trim().chars().take(128).collect();
        while out.chars().count() < 2 {
            out.push(' ');
        }
        out
    }

    /// Returns false if Discord is unavailable, signaling the frontend to retry later.
    pub fn update_presence(payload: PresencePayload) -> bool {
        let Ok(mut state) = STATE.lock() else {
            return true;
        };
        if !ensure_connected(&mut state) {
            return false;
        }

        let details = clamp_line(&payload.details);
        let state_line = clamp_line(&payload.state);
        let cover_text = payload.cover_text.as_deref().map(clamp_line);

        let mut assets = Assets::new().large_image(payload.cover_url.as_deref().unwrap_or("logo"));
        if let Some(text) = cover_text.as_deref() {
            assets = assets.large_text(text);
        }

        let mut activity = Activity::new()
            .activity_type(ActivityType::Listening)
            .details(&details)
            .state(&state_line)
            .assets(assets);

        // Discord animates the progress bar itself from start/end timestamps,
        // no need to push updates every second.
        if payload.playing {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let start = now - payload.position_secs.max(0.0) as i64;
            let mut timestamps = Timestamps::new().start(start);
            if payload.duration_secs > 0.0 {
                timestamps = timestamps.end(start + payload.duration_secs as i64);
            }
            activity = activity.timestamps(timestamps);
        }

        let client = state.client.as_mut().expect("client после ensure_connected");
        if client.set_activity(activity).is_err() {
            // Connection dropped (Discord closed) — reset and retry later.
            let _ = client.close();
            state.client = None;
            state.last_failed_attempt = Some(Instant::now());
            return false;
        }
        true
    }

    pub fn clear_presence() {
        let Ok(mut state) = STATE.lock() else { return };
        let Some(client) = state.client.as_mut() else {
            return;
        };
        if client.clear_activity().is_err() {
            let _ = client.close();
            state.client = None;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn clamp_line_passes_through_normal_text_unchanged() {
            assert_eq!(clamp_line("Bohemian Rhapsody"), "Bohemian Rhapsody");
        }

        #[test]
        fn clamp_line_trims_surrounding_whitespace() {
            assert_eq!(clamp_line("  Queen  "), "Queen");
        }

        #[test]
        fn clamp_line_truncates_to_128_chars() {
            let long = "x".repeat(300);
            let clamped = clamp_line(&long);
            assert_eq!(clamped.chars().count(), 128);
        }

        #[test]
        fn clamp_line_pads_short_strings_to_minimum_two_chars() {
            assert_eq!(clamp_line(""), "  ");
            assert_eq!(clamp_line("x"), "x ");
            assert_eq!(clamp_line("  ").chars().count(), 2);
        }

        #[test]
        fn clamp_line_counts_unicode_chars_not_bytes() {
            // Cyrillic is multi-byte; limit must count chars, not bytes, or
            // truncation could split a UTF-8 sequence and panic on indexing.
            let long = "я".repeat(200);
            let clamped = clamp_line(&long);
            assert_eq!(clamped.chars().count(), 128);
        }
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
mod imp {
    use super::PresencePayload;

    // true = "delivered" so the frontend doesn't schedule pointless retries.
    pub fn update_presence(_payload: PresencePayload) -> bool {
        true
    }
    pub fn clear_presence() {}
}

pub use imp::{clear_presence, update_presence};
