//! Picks which search hit actually is the requested catalog track.
//!
//! A blind "first search result" grabs live versions, covers, sped-up edits and
//! hour-long loops surprisingly often. Every candidate is scored instead on
//! title coverage, artist presence, duration closeness (the catalog knows the
//! exact length) and unwanted-version keywords; anything scoring below
//! [`MIN_ACCEPTABLE_SCORE`] is rejected rather than downloaded.

/// One search hit, normalized across sources.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub url: String,
    pub title: String,
    /// Channel / uploader name.
    pub uploader: String,
    pub duration_secs: Option<f64>,
}

/// What we are looking for (from the catalog).
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub duration_secs: Option<u32>,
}

/// A correct upload typically scores 70–90; a different version of the song
/// (remix, live, sped up…) lands below this even with a matching duration.
pub const MIN_ACCEPTABLE_SCORE: i32 = 40;

/// Version markers that mean "not the studio recording" — penalized unless
/// the requested title itself contains them (asking for a remix is fine).
const UNWANTED: &[&str] = &[
    "live", "cover", "karaoke", "instrumental", "remix", "sped up", "speed up", "slowed",
    "reverb", "nightcore", "8d", "1 hour", "10 hours", "loop", "reaction", "tutorial",
    "lesson", "acapella", "a cappella", "bass boosted", "piano version", "minus", "минус",
    "кавер", "караоке", "live version", "concert", "концерт", "remastered live",
];

/// Lowercased words of letters/digits only.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Space-joined normalized form, padded so `contains(" word ")` matches whole words.
fn padded(s: &str) -> String {
    format!(" {} ", words(s).join(" "))
}

/// The requested title without "(feat. …)" / "[…]" decorations, which uploads
/// often spell differently or drop.
fn core_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut depth = 0i32;
    for ch in title.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    let lower = out.to_lowercase();
    let cut = [" feat.", " feat ", " ft.", " ft "]
        .iter()
        .filter_map(|m| lower.find(m))
        .min();
    match cut {
        Some(i) => out[..i].to_string(),
        None => out,
    }
}

/// First credited artist ("A, B & C feat. D" → "A").
fn primary_artist(artist: &str) -> String {
    let lower = artist.to_lowercase();
    let cut = [",", " & ", " feat", " ft.", " x ", ";", " и "]
        .iter()
        .filter_map(|m| lower.find(m))
        .min();
    match cut {
        Some(i) => artist[..i].trim().to_string(),
        None => artist.trim().to_string(),
    }
}

pub fn score(target: &Target, c: &Candidate) -> i32 {
    let cand_title = padded(&c.title);
    let uploader = padded(&c.uploader);
    let haystack = format!("{}{}", cand_title, uploader);
    let mut s = 0i32;

    // Title coverage: share of requested title words present in the hit.
    let title_words = words(&core_title(target.title));
    if !title_words.is_empty() {
        let hit = title_words
            .iter()
            .filter(|w| cand_title.contains(&format!(" {} ", w)))
            .count();
        let coverage = hit as f64 / title_words.len() as f64;
        s += (coverage * 35.0).round() as i32;
        if coverage < 0.5 {
            s -= 40;
        }
    }

    // Artist anywhere in title or channel name.
    let artist_words = words(&primary_artist(target.artist));
    if !artist_words.is_empty() {
        let artist_phrase = format!(" {} ", artist_words.join(" "));
        if haystack.contains(&artist_phrase) {
            s += 20;
        } else {
            s -= 10;
        }
        // "Artist - Topic" channels are YouTube Music's auto-generated official audio.
        if uploader.contains(&artist_phrase) && uploader.contains(" topic ") {
            s += 15;
        }
    }

    // Duration: the strongest signal when the catalog knows the length.
    if let (Some(want), Some(got)) = (target.duration_secs, c.duration_secs) {
        let diff = (want as f64 - got).abs();
        s += match diff {
            d if d <= 2.0 => 30,
            d if d <= 5.0 => 22,
            d if d <= 10.0 => 10,
            d if d <= 20.0 => 0,
            d if d <= 40.0 => -20,
            _ => -45,
        };
    }

    // Different version of the song (unless that's what was asked for).
    let wanted_title = padded(target.title);
    let mut penalty = 0;
    for marker in UNWANTED {
        let m = format!(" {} ", marker);
        if cand_title.contains(&m) && !wanted_title.contains(&m) {
            penalty += 45;
        }
    }
    s -= penalty.min(90);

    if cand_title.contains(" audio ") {
        s += 4;
    }
    s
}

/// Candidates sorted best-first, only those above the acceptance threshold.
pub fn rank<'a>(target: &Target, candidates: &'a [Candidate]) -> Vec<&'a Candidate> {
    let mut scored: Vec<(i32, usize, &Candidate)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (score(target, c), i, c))
        .filter(|(s, _, _)| *s >= MIN_ACCEPTABLE_SCORE)
        .collect();
    // Ties keep the source's own relevance order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(title: &str, uploader: &str, dur: f64) -> Candidate {
        Candidate {
            url: format!("https://x/{}", title),
            title: title.into(),
            uploader: uploader.into(),
            duration_secs: Some(dur),
        }
    }

    #[test]
    fn prefers_official_audio_with_matching_duration() {
        let t = Target { title: "Blinding Lights", artist: "The Weeknd", duration_secs: Some(200) };
        let list = vec![
            cand("The Weeknd - Blinding Lights (Live at the Grammys)", "The Weeknd", 260.0),
            cand("Blinding Lights (Sped Up)", "Some Edits", 160.0),
            cand("Blinding Lights", "The Weeknd - Topic", 201.0),
            cand("The Weeknd - Blinding Lights (Official Video)", "TheWeekndVEVO", 263.0),
        ];
        let ranked = rank(&t, &list);
        assert_eq!(ranked[0].uploader, "The Weeknd - Topic");
        assert!(ranked.iter().all(|c| !c.title.contains("Sped Up")));
    }

    #[test]
    fn requested_remix_is_not_penalized() {
        let t = Target { title: "Song (Remix)", artist: "Artist", duration_secs: Some(180) };
        let remix = cand("Artist - Song (Remix)", "Artist", 181.0);
        assert!(score(&t, &remix) > 40);
    }

    #[test]
    fn rejects_other_versions_even_with_close_duration() {
        let t = Target { title: "Blinding Lights", artist: "The Weeknd", duration_secs: Some(200) };
        let remix = cand("Blinding Lights - The Weeknd (SPARKZ Remix)", "SPARKZ", 204.9);
        assert!(rank(&t, &[remix]).is_empty());
    }

    #[test]
    fn rejects_unrelated_hits() {
        let t = Target { title: "Intro", artist: "The xx", duration_secs: Some(128) };
        let list = vec![cand("Best Intro Music 1 Hour", "Loops", 3600.0)];
        assert!(rank(&t, &list).is_empty());
    }

    #[test]
    fn feat_suffix_and_cyrillic() {
        let t = Target { title: "Город (feat. Кто-то)", artist: "Сплин, Другой", duration_secs: Some(240) };
        let c = cand("Сплин — Город", "Сплин - Topic", 241.0);
        assert!(score(&t, &c) > 60);
    }

    #[test]
    fn works_without_known_duration() {
        let t = Target { title: "Numb", artist: "Linkin Park", duration_secs: None };
        let c = Candidate { url: "u".into(), title: "Numb".into(), uploader: "Linkin Park - Topic".into(), duration_secs: None };
        assert!(score(&t, &c) >= 50);
    }
}
