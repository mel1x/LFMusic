//! `lfmusic-snippet` protocol: a short track fragment for recap cards, decoded into an in-memory
//! WAV.


use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use symphonia::core::formats::SeekMode;

use crate::audio_player::OpenedAudio;
use tauri::http::{header, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

const SCHEME: &str = "lfmusic-snippet";
const SNIPPET_SECS: f64 = 15.0;
/// Hi-res sources are decimated to stay near this rate (keeps the WAV small).
const MAX_SAMPLE_RATE: u32 = 48_000;

/// URL of a track's fragment, same origin rules as `thumbnails::cover_protocol_url`.
pub fn snippet_url(file_path: &str) -> String {
    let key = URL_SAFE_NO_PAD.encode(file_path.as_bytes());
    #[cfg(any(windows, target_os = "android"))]
    {
        format!("http://{}.localhost/{}", SCHEME, key)
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        format!("{}://localhost/{}", SCHEME, key)
    }
}

/// Fragment start: around the first chorus for a typical song, never past the end.
fn snippet_start(duration_secs: Option<f64>) -> f64 {
    match duration_secs {
        Some(d) if d > SNIPPET_SECS * 2.0 => (d * 0.33).min(d - SNIPPET_SECS - 2.0).max(0.0),
        _ => 0.0,
    }
}

/// Decodes the fragment to stereo i16. `known_duration` covers containers without a length (MP3
/// without Xing).
fn decode_snippet(file_path: &str, known_duration: Option<f64>) -> Result<(Vec<i16>, u32), String> {
    let mut audio = OpenedAudio::open(file_path)?;
    let rate = audio.sample_rate;
    let duration = audio
        .num_frames
        .map(|n| n as f64 / rate as f64)
        .or(known_duration.filter(|d| *d > 0.0));

    let start = snippet_start(duration);
    if start > 0.0 {
        let _ = audio.seek(start, SeekMode::Coarse);
    }

    let stride = rate.div_ceil(MAX_SAMPLE_RATE).max(1) as usize;
    let out_rate = rate / stride as u32;
    let max_frames = (out_rate as f64 * SNIPPET_SECS) as usize;
    let mut out: Vec<i16> = Vec::with_capacity(max_frames * 2);
    let mut pos = 0usize;

    let channels = audio.channels as usize;
    let mut packet_samples = Vec::new();

    while out.len() < max_frames * 2 && audio.decode_next(&mut packet_samples) {
        for frame in packet_samples.chunks_exact(channels) {
            if pos.is_multiple_of(stride) && out.len() < max_frames * 2 {
                let left = frame[0];
                let right = if channels > 1 { frame[1] } else { left };
                out.push((left.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                out.push((right.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
            }
            pos += 1;
        }
    }

    if out.is_empty() {
        return Err("empty snippet".into());
    }
    Ok((out, out_rate))
}

fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

fn respond_status(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(Vec::new())
        .unwrap()
}

fn build_response<R: Runtime>(app: &tauri::AppHandle<R>, uri: &tauri::http::Uri) -> Response<Vec<u8>> {
    let Some(file_path) = URL_SAFE_NO_PAD
        .decode(uri.path().trim_start_matches('/'))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return respond_status(StatusCode::NOT_FOUND);
    };
    let library_duration = app
        .state::<crate::LibraryState>()
        .0
        .lock()
        .ok()
        .and_then(|lib| lib.library_track_duration(&file_path));
    let Some(known_duration) = library_duration else {
        return respond_status(StatusCode::FORBIDDEN);
    };
    match decode_snippet(&file_path, known_duration) {
        Ok((samples, rate)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "audio/wav")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(wav_bytes(&samples, rate))
            .unwrap(),
        Err(_) => respond_status(StatusCode::NOT_FOUND),
    }
}

pub fn protocol_handler<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let uri = request.uri().clone();
    let _ = std::thread::Builder::new()
        .name("snippet-decoder".into())
        .spawn(move || responder.respond(build_response(&app, &uri)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_starts_a_third_in_and_stays_inside_the_track() {
        assert_eq!(snippet_start(None), 0.0);
        assert_eq!(snippet_start(Some(20.0)), 0.0);
        assert_eq!(snippet_start(Some(300.0)), 99.0);
        assert!(snippet_start(Some(40.0)) + SNIPPET_SECS <= 40.0);
    }

    #[test]
    fn wav_header_describes_stereo_16_bit_pcm() {
        let wav = wav_bytes(&[0, 1, 2, 3], 44_100);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 44_100);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
        assert_eq!(wav.len(), 44 + 8);
    }
}
