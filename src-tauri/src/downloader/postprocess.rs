//! After yt-dlp: make sure the file is playable (convert with ffmpeg when it
//! isn't), then write tags, cover art and lyrics.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use lofty::file::FileType;
use serde::Deserialize;

use super::tools::{self, Resolved};
use super::ytdlp::Control;
use super::{ErrorKind, JobError};

/// What we know about the track when tagging it.
pub struct TagInfo<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub album_id: Option<u64>,
    /// Catalog cover (Deezer/iTunes CDN).
    pub cover_url: Option<&'a str>,
    /// Video thumbnail, used (cropped square) when nothing better exists.
    pub thumbnail_url: Option<&'a str>,
}

/// Content-detected container/codec (never trust the extension).
fn detect(path: &Path) -> Option<FileType> {
    lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.file_type()
}

/// Extension the player decodes for this content, or `None` if it needs converting.
fn playable_ext(path: &Path) -> Option<&'static str> {
    match detect(path)? {
        FileType::Mp4 => {
            // Only audio-only MP4 (m4a); a muxed video file would also land here.
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            (ext == "m4a").then_some("m4a")
        }
        FileType::Mpeg => Some("mp3"),
        FileType::Flac => Some("flac"),
        FileType::Vorbis => Some("ogg"),
        FileType::Wav => Some("wav"),
        FileType::Aac => Some("aac"),
        FileType::Aiff => Some("aiff"),
        // Opus (symphonia has no decoder), WebM/Matroska and anything else.
        _ => None,
    }
}

/// Ensures the downloaded file is something the player can decode, converting
/// to m4a (AAC) with ffmpeg if needed. Returns the final path and extension.
pub fn ensure_playable(
    file: &Path,
    tools: &Resolved,
    control: &Control,
) -> Result<(PathBuf, &'static str), JobError> {
    if let Some(ext) = playable_ext(file) {
        return Ok((file.to_path_buf(), ext));
    }
    let Some(ffmpeg) = &tools.ffmpeg else {
        return Err(JobError::new(
            ErrorKind::Format,
            "Источник отдал формат, который плеер не воспроизводит - установите ffmpeg для конвертации",
        ));
    };
    let out = file.with_file_name("converted.m4a");
    // A muxed MP4 almost always carries AAC, which can be stream-copied.
    // Anything else (WebM/Opus) must be transcoded — copying Opus into an
    // .m4a would produce a file that still can't be decoded.
    const COPY: &[&str] = &["-c:a", "copy"];
    const AAC: &[&str] = &["-c:a", "aac", "-b:a", "256k"];
    let attempts: &[&[&str]] = if detect(file) == Some(FileType::Mp4) { &[COPY, AAC] } else { &[AAC] };
    for codec_args in attempts.iter().copied() {
        control.check()?;
        let _ = std::fs::remove_file(&out);
        let status = tools::command(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(file)
            .args(["-vn", "-map_metadata", "-1"])
            .args(codec_args)
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(status, Ok(s) if s.success()) && playable_ext(&out).is_some() {
            let _ = std::fs::remove_file(file);
            return Ok((out, "m4a"));
        }
    }
    let _ = std::fs::remove_file(&out);
    Err(JobError::new(ErrorKind::Format, "ffmpeg не смог конвертировать аудио"))
}

// ── Network helpers ─────────────────────────────────────────────────────────

fn http() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent("LFMusic (+https://github.com/mel1x/LFMusic)")
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    const MAX: u64 = 10 * 1024 * 1024;
    let resp = http().get(url).send().ok()?.error_for_status().ok()?;
    let mut bytes = Vec::new();
    resp.take(MAX + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() > 1000 && (bytes.len() as u64) <= MAX).then_some(bytes)
}

/// Center-crops a (16:9, letterboxed) video thumbnail to a square JPEG.
fn square_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let cropped = img.crop_imm((w - side) / 2, (h - side) / 2, side, side).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(cropped)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .ok()?;
    Some(out.into_inner())
}

fn resolve_cover(info: &TagInfo) -> Option<Vec<u8>> {
    if let Some(bytes) = info.cover_url.and_then(fetch_image) {
        return Some(bytes);
    }
    if let crate::external::FetchOutcome::Found(img) =
        crate::external::fetch_track_cover(info.artist, info.album.unwrap_or(""), info.title)
    {
        if let Some(bytes) = fetch_image(&img.url) {
            return Some(bytes);
        }
    }
    info.thumbnail_url
        .and_then(fetch_image)
        .and_then(|b| square_jpeg(&b))
}

fn image_mime(bytes: &[u8]) -> lofty::picture::MimeType {
    use lofty::picture::MimeType;
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => MimeType::Png,
        [b'G', b'I', b'F', ..] => MimeType::Gif,
        [b'B', b'M', ..] => MimeType::Bmp,
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => {
            MimeType::Unknown("image/webp".into())
        }
        _ => MimeType::Jpeg,
    }
}

// ── Lyrics (LRCLIB, keyless) ────────────────────────────────────────────────

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lrclib {
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

pub struct Lyrics {
    pub synced: Option<String>,
    pub plain: Option<String>,
}

impl Lrclib {
    fn usable(self) -> Option<Lyrics> {
        if self.instrumental {
            return None;
        }
        let keep = |s: Option<String>| s.filter(|v| !v.trim().is_empty());
        let (synced, plain) = (keep(self.synced_lyrics), keep(self.plain_lyrics));
        (synced.is_some() || plain.is_some()).then_some(Lyrics { synced, plain })
    }
}

fn fetch_lyrics(artist: &str, title: &str, album: Option<&str>, duration: Option<u64>) -> Option<Lyrics> {
    if let Some(d) = duration {
        let exact = http()
            .get("https://lrclib.net/api/get")
            .query(&[
                ("artist_name", artist),
                ("track_name", title),
                ("album_name", album.unwrap_or("")),
                ("duration", &d.to_string()),
            ])
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<Lrclib>().ok())
            .and_then(Lrclib::usable);
        if exact.is_some() {
            return exact;
        }
    }
    let results: Vec<Lrclib> = http()
        .get("https://lrclib.net/api/search")
        .query(&[("artist_name", artist), ("track_name", title)])
        .send()
        .ok()
        .and_then(|r| r.error_for_status().ok())
        .and_then(|r| r.json().ok())
        .unwrap_or_default();
    let mut plain_only = None;
    for found in results.into_iter().filter_map(Lrclib::usable) {
        if found.synced.is_some() {
            return Some(found);
        }
        plain_only.get_or_insert(found);
    }
    plain_only
}

// ── Tagging ─────────────────────────────────────────────────────────────────

/// Writes title/artist/album/genre/cover/lyrics. Best-effort: a file that
/// can't be tagged is still a valid download. Returns lyrics for the `.lrc`
/// sidecar, which is written once the file has its final name.
pub fn write_tags(path: &Path, info: &TagInfo) -> Option<Lyrics> {
    use lofty::config::WriteOptions;
    use lofty::picture::{Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::{ItemKey, Tag};

    let mut tagged = crate::audio_player::AudioPlayer::read_tagged_file(path);
    let duration = tagged
        .as_ref()
        .map(|t| t.properties().duration().as_secs())
        .filter(|s| *s > 0);

    // Independent hosts — fetch concurrently.
    let (lyrics, cover, genre) = std::thread::scope(|s| {
        let lyrics = s.spawn(|| fetch_lyrics(info.artist, info.title, info.album, duration));
        let cover = s.spawn(|| resolve_cover(info));
        let genre = s.spawn(|| {
            info.genre
                .map(str::to_string)
                .or_else(|| info.album_id.and_then(crate::external::deezer_album_genre))
        });
        (
            lyrics.join().ok().flatten(),
            cover.join().ok().flatten(),
            genre.join().ok().flatten(),
        )
    });

    let Some(file) = tagged.as_mut() else {
        return lyrics;
    };
    let tag_type = file.primary_tag_type();
    if file.primary_tag().is_none() {
        file.insert_tag(Tag::new(tag_type));
    }
    let Some(tag) = file.primary_tag_mut() else {
        return lyrics;
    };
    tag.set_title(info.title.to_string());
    tag.set_artist(info.artist.to_string());
    if let Some(album) = info.album.filter(|a| !a.trim().is_empty()) {
        tag.set_album(album.to_string());
    }
    if let Some(g) = genre {
        tag.set_genre(g);
    }
    if let Some(bytes) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        let mime = image_mime(&bytes);
        tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(mime), None, bytes));
    }
    if let Some(text) = lyrics.as_ref().and_then(|l| l.plain.as_deref().or(l.synced.as_deref())) {
        tag.insert_text(ItemKey::Lyrics, text.to_string());
    }
    if let Err(e) = tag.save_to_path(path, WriteOptions::default()) {
        eprintln!("[downloader] tag write failed for {}: {}", path.display(), e);
    }
    lyrics
}

/// `.lrc` next to the track, synced lyrics preferred.
pub fn write_lrc(audio: &Path, lyrics: Option<&Lyrics>) {
    if let Some(body) = lyrics.and_then(|l| l.synced.as_deref().or(l.plain.as_deref())) {
        let _ = std::fs::write(audio.with_extension("lrc"), body);
    }
}
