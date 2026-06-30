// Порт ранжирования каталога из фронтенда (src/composables/useSearch.ts) в Rust.
//
// Цель — один-в-один сохранить поведение поиска при переносе на бэкенд:
//   1) точное вхождение подстроки;
//   2) нечёткое совпадение (Левенштейн, порог 0.2 / минимум 4 символа);
//   3) точное вхождение в перевёрнутой раскладке (ru/en);
//   4) нечёткое в перевёрнутой раскладке;
//   5) совпадение по аудиоформату (lossless / lossy / расширение).
//
// Поиск идёт по in-memory индексу (см. library.rs), который строится из БД в том
// же каноническом порядке, что и `get_all_tracks`, поэтому порядок результатов
// внутри каждого тира совпадает с прежним фронтендовым.

/// Lossless-форматы для спец-фильтра по расширению (как в useSearch.ts).
pub const LOSSLESS_FORMATS: &[&str] =
    &["flac", "wav", "alac", "ape", "dsd", "dsf", "aiff", "aif", "wv", "tak"];
/// Lossy-форматы.
pub const LOSSY_FORMATS: &[&str] = &["mp3", "aac", "ogg", "m4a", "wma", "opus"];

fn is_known_format(ext: &str) -> bool {
    LOSSLESS_FORMATS.contains(&ext) || LOSSY_FORMATS.contains(&ext)
}

const FUZZY_MIN_LEN: usize = 4;
const FUZZY_THRESHOLD: f64 = 0.2;

/// Одна запись индекса: путь + уже приведённые к нижнему регистру поля + жанр + расширение.
pub struct SearchEntry {
    pub path: String,
    pub genre: Option<String>,
    /// [title, artist, album, basename] в нижнем регистре.
    fields: [String; 4],
    /// Расширение файла в нижнем регистре (для фильтра по формату).
    ext: String,
}

impl SearchEntry {
    pub fn new(
        path: String,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        genre: Option<String>,
    ) -> Self {
        let norm = path.replace('\\', "/");
        let file = norm.rsplit('/').next().unwrap_or("");
        let basename = strip_ext(file).to_lowercase();
        let ext = extension(&path);
        let fields = [
            title.unwrap_or_default().to_lowercase(),
            artist.unwrap_or_default().to_lowercase(),
            album.unwrap_or_default().to_lowercase(),
            basename,
        ];
        SearchEntry {
            path,
            genre,
            fields,
            ext,
        }
    }
}

/// Имя файла без расширения — аналог `fileName.replace(/\.[^/.]+$/, '')`.
fn strip_ext(file: &str) -> &str {
    if let Some(idx) = file.rfind('.') {
        // Расширение (часть после последней точки) должно быть непустым.
        if idx + 1 < file.len() {
            return &file[..idx];
        }
    }
    file
}

/// Расширение пути в нижнем регистре — аналог getExtension (`/\.([^.\\/]+)$/`).
fn extension(path: &str) -> String {
    if let Some(idx) = path.rfind('.') {
        let ext = &path[idx + 1..];
        if !ext.is_empty() && !ext.contains('/') && !ext.contains('\\') && !ext.contains('.') {
            return ext.to_lowercase();
        }
    }
    String::new()
}

// ── Раскладка клавиатуры (en ↔ ru), как карты EN_TO_RU / RU_TO_EN ──────────────

fn en_to_ru(c: char) -> Option<char> {
    Some(match c {
        'q' => 'й', 'w' => 'ц', 'e' => 'у', 'r' => 'к', 't' => 'е', 'y' => 'н', 'u' => 'г',
        'i' => 'ш', 'o' => 'щ', 'p' => 'з', '[' => 'х', ']' => 'ъ',
        'a' => 'ф', 's' => 'ы', 'd' => 'в', 'f' => 'а', 'g' => 'п', 'h' => 'р', 'j' => 'о',
        'k' => 'л', 'l' => 'д', ';' => 'ж', '\'' => 'э',
        'z' => 'я', 'x' => 'ч', 'c' => 'с', 'v' => 'м', 'b' => 'и', 'n' => 'т', 'm' => 'ь',
        ',' => 'б', '.' => 'ю', '/' => '.',
        '`' => 'ё',
        _ => return None,
    })
}

fn ru_to_en(c: char) -> Option<char> {
    Some(match c {
        'й' => 'q', 'ц' => 'w', 'у' => 'e', 'к' => 'r', 'е' => 't', 'н' => 'y', 'г' => 'u',
        'ш' => 'i', 'щ' => 'o', 'з' => 'p', 'х' => '[', 'ъ' => ']',
        'ф' => 'a', 'ы' => 's', 'в' => 'd', 'а' => 'f', 'п' => 'g', 'р' => 'h', 'о' => 'j',
        'л' => 'k', 'д' => 'l', 'ж' => ';', 'э' => '\'',
        'я' => 'z', 'ч' => 'x', 'с' => 'c', 'м' => 'v', 'и' => 'b', 'т' => 'n', 'ь' => 'm',
        'б' => ',', 'ю' => '.', '.' => '/',
        'ё' => '`',
        _ => return None,
    })
}

/// Перевод между раскладками — аналог switchLayout.
pub fn switch_layout(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let lower = ch.to_lowercase().next().unwrap_or(ch);
        if let Some(m) = en_to_ru(lower) {
            out.push(m);
        } else if let Some(m) = ru_to_en(lower) {
            out.push(m);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Запрос является ключевым словом формата (lossless/lossy/расширение).
pub fn is_format_query(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q == "lossless" || q == "lossy" || is_known_format(&q)
}

/// Совпадение расширения с форматным запросом — аналог matchesFormat.
fn matches_format(ext: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q == "lossless" {
        return LOSSLESS_FORMATS.contains(&ext);
    }
    if q == "lossy" {
        return LOSSY_FORMATS.contains(&ext);
    }
    if is_known_format(&q) {
        return ext == q;
    }
    false
}

// ── Расстояние Левенштейна / нечёткое совпадение (по символам Unicode) ─────────

fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a == b {
        return 0;
    }
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let n = b.len();
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        let ca = a[i - 1];
        for j in 1..=n {
            let cost = if ca == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Минимальное расстояние от query до любой подстроки target (approximate
/// substring matching) — аналог approxSubstringDistance.
fn approx_substring_distance(query: &[char], target: &[char]) -> usize {
    let m = query.len();
    let n = target.len();
    if m == 0 {
        return 0;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = vec![0; n + 1];
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        let cq = query[i - 1];
        for j in 1..=n {
            let cost = if cq == target[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        prev.copy_from_slice(&curr);
    }
    *prev.iter().min().unwrap_or(&m)
}

/// Нечёткое совпадение query внутри target — аналог fuzzyMatch.
fn fuzzy_match(target: &str, query_chars: &[char]) -> bool {
    let qn = query_chars.len();
    if qn < FUZZY_MIN_LEN {
        return false;
    }
    if target.is_empty() {
        return false;
    }
    let max_edits = (qn as f64 * FUZZY_THRESHOLD).floor() as usize;
    if max_edits == 0 {
        return false;
    }
    // Быстрый путь: по словам.
    for w in target.split_whitespace() {
        let wchars: Vec<char> = w.chars().collect();
        let wl = wchars.len();
        let diff = if wl > qn { wl - qn } else { qn - wl };
        if diff > max_edits {
            continue;
        }
        if levenshtein(&wchars, query_chars) <= max_edits {
            return true;
        }
    }
    // Общий подстрочный поиск.
    let tchars: Vec<char> = target.chars().collect();
    approx_substring_distance(query_chars, &tchars) <= max_edits
}

fn genre_matches(entry: &SearchEntry, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(g) => entry.genre.as_deref() == Some(g),
    }
}

/// Ранжированный поиск по индексу. Возвращает ссылки на записи в порядке тиров,
/// сохраняя внутри тира исходный (канонический) порядок индекса.
/// `genre_filter` — необязательный точный фильтр по жанру (как на странице каталога).
pub fn rank<'a>(
    entries: &'a [SearchEntry],
    query: &str,
    genre_filter: Option<&str>,
) -> Vec<&'a SearchEntry> {
    let q_lower = query.to_lowercase();
    let q = q_lower.trim();

    if q.is_empty() {
        return entries
            .iter()
            .filter(|e| genre_matches(e, genre_filter))
            .collect();
    }

    let switched = switch_layout(q);
    let use_switched = switched != q;
    let is_format = is_format_query(q);
    let allow_fuzzy = !is_format;

    let q_chars: Vec<char> = q.chars().collect();
    let switched_chars: Vec<char> = switched.chars().collect();
    let q_len = q_chars.len();
    let switched_len = switched_chars.len();

    let mut tiers: [Vec<&SearchEntry>; 5] = Default::default();

    for e in entries {
        if !genre_matches(e, genre_filter) {
            continue;
        }
        let fields = &e.fields;

        if fields.iter().any(|f| f.contains(q)) {
            tiers[0].push(e);
            continue;
        }
        if allow_fuzzy
            && q_len >= FUZZY_MIN_LEN
            && fields.iter().any(|f| fuzzy_match(f, &q_chars))
        {
            tiers[1].push(e);
            continue;
        }
        if use_switched && fields.iter().any(|f| f.contains(&switched)) {
            tiers[2].push(e);
            continue;
        }
        if allow_fuzzy
            && use_switched
            && switched_len >= FUZZY_MIN_LEN
            && fields.iter().any(|f| fuzzy_match(f, &switched_chars))
        {
            tiers[3].push(e);
            continue;
        }
        if is_format && matches_format(&e.ext, q) {
            tiers[4].push(e);
            continue;
        }
    }

    let mut out: Vec<&SearchEntry> = Vec::new();
    for tier in tiers {
        out.extend(tier);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, title: &str, artist: &str, album: &str, genre: Option<&str>) -> SearchEntry {
        SearchEntry::new(
            path.to_string(),
            Some(title.to_string()),
            Some(artist.to_string()),
            Some(album.to_string()),
            genre.map(|s| s.to_string()),
        )
    }

    fn paths<'a>(res: Vec<&'a SearchEntry>) -> Vec<&'a str> {
        res.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn exact_substring_tier() {
        let entries = vec![
            entry("/m/1.mp3", "Bohemian Rhapsody", "Queen", "A Night", None),
            entry("/m/2.mp3", "Another One", "Queen", "The Game", None),
        ];
        let res = paths(rank(&entries, "rhap", None));
        assert_eq!(res, vec!["/m/1.mp3"]);
    }

    #[test]
    fn fuzzy_typo_matches() {
        // "boheman" — опечатка в 1 правку (пропущена 'i'); порог 0.2 при 7 символах
        // допускает 1 правку → нечёткое совпадение. (Транспозиция = 2 правки и
        // НЕ матчится — это совпадает с поведением фронтенда.)
        let entries = vec![entry("/m/1.mp3", "Bohemian Rhapsody", "Queen", "A", None)];
        let res = paths(rank(&entries, "boheman", None));
        assert_eq!(res, vec!["/m/1.mp3"]);
    }

    #[test]
    fn layout_switch_match() {
        // Набрано в англ. раскладке вместо русской: "Ц運" не нужен — берём простой случай.
        // "queen" в русской раскладке = "йгуут"; обратный перевод даёт "queen".
        let entries = vec![entry("/m/1.mp3", "Song", "Queen", "A", None)];
        let res = paths(rank(&entries, "йгуут", None));
        assert_eq!(res, vec!["/m/1.mp3"]);
    }

    #[test]
    fn format_filter() {
        let entries = vec![
            entry("/m/a.flac", "X", "Y", "Z", None),
            entry("/m/b.mp3", "X", "Y", "Z", None),
        ];
        assert_eq!(paths(rank(&entries, "flac", None)), vec!["/m/a.flac"]);
        assert_eq!(paths(rank(&entries, "lossless", None)), vec!["/m/a.flac"]);
        assert_eq!(paths(rank(&entries, "lossy", None)), vec!["/m/b.mp3"]);
    }

    #[test]
    fn genre_filter_restricts() {
        let entries = vec![
            entry("/m/1.mp3", "Track One", "A", "Al", Some("Rock")),
            entry("/m/2.mp3", "Track Two", "A", "Al", Some("Jazz")),
        ];
        let res = paths(rank(&entries, "track", Some("Jazz")));
        assert_eq!(res, vec!["/m/2.mp3"]);
    }

    #[test]
    fn tier_ordering_exact_before_fuzzy() {
        let entries = vec![
            entry("/m/1.mp3", "Imagine", "John", "Al", None),   // fuzzy для "imagin" тоже подходит, но exact-вхождение есть
            entry("/m/2.mp3", "Imagne Dragons", "X", "Al", None), // содержит "imag" → exact-вхождение для "imagine"? нет
        ];
        // Запрос с точным вхождением должен идти раньше нечёткого.
        let res = paths(rank(&entries, "imagine", None));
        assert_eq!(res.first(), Some(&"/m/1.mp3"));
    }

    // ── Доп. покрытие ────────────────────────────────────────────────────────

    #[test]
    fn empty_query_returns_all_entries_in_index_order() {
        let entries = vec![
            entry("/m/1.mp3", "B", "X", "Al", None),
            entry("/m/2.mp3", "A", "X", "Al", None),
        ];
        // Пустой запрос не сортирует по алфавиту — отдаёт канонический порядок индекса.
        assert_eq!(paths(rank(&entries, "", None)), vec!["/m/1.mp3", "/m/2.mp3"]);
        assert_eq!(paths(rank(&entries, "   ", None)), vec!["/m/1.mp3", "/m/2.mp3"]);
    }

    #[test]
    fn empty_query_still_respects_genre_filter() {
        let entries = vec![
            entry("/m/1.mp3", "A", "X", "Al", Some("Rock")),
            entry("/m/2.mp3", "B", "X", "Al", Some("Jazz")),
        ];
        assert_eq!(paths(rank(&entries, "", Some("Jazz"))), vec!["/m/2.mp3"]);
    }

    #[test]
    fn search_is_case_insensitive() {
        let entries = vec![entry("/m/1.mp3", "Bohemian Rhapsody", "Queen", "A", None)];
        assert_eq!(paths(rank(&entries, "RHAPSODY", None)), vec!["/m/1.mp3"]);
        assert_eq!(paths(rank(&entries, "queen", None)), vec!["/m/1.mp3"]);
    }

    #[test]
    fn fuzzy_requires_minimum_length() {
        // 3-символьный запрос короче FUZZY_MIN_LEN (4) — нечёткое совпадение не
        // должно срабатывать даже при опечатке в один символ.
        let entries = vec![entry("/m/1.mp3", "Cat", "X", "Al", None)]; // "Cwt" -> "Cat", 1 правка, но длина 3
        assert!(rank(&entries, "Cwt", None).is_empty());
    }

    #[test]
    fn fuzzy_rejects_too_many_edits() {
        // 4 символа, порог 0.2 -> floor(4*0.2)=0 правок допустимо => полностью
        // отличающаяся строка той же длины не должна совпадать.
        let entries = vec![entry("/m/1.mp3", "Wxyz", "X", "Al", None)];
        assert!(rank(&entries, "Abcd", None).is_empty());
    }

    #[test]
    fn basename_is_searchable_when_tags_are_missing() {
        let mut entries = vec![SearchEntry::new(
            "/music/Artist - Great Song.mp3".to_string(),
            None,
            None,
            None,
            None,
        )];
        let res = rank(&entries, "great song", None);
        assert_eq!(res.len(), 1);
        // Без тегов остаются только запись и поиск по basename.
        entries.clear();
        assert!(entries.is_empty());
    }

    #[test]
    fn windows_backslash_paths_are_normalized_for_basename() {
        let e = SearchEntry::new(
            "C:\\Music\\Artist\\Track Name.flac".to_string(),
            None,
            None,
            None,
            None,
        );
        let entries = vec![e];
        assert_eq!(paths(rank(&entries, "track name", None)), vec!["C:\\Music\\Artist\\Track Name.flac"]);
    }

    #[test]
    fn is_format_query_is_case_and_space_insensitive() {
        assert!(is_format_query("FLAC"));
        assert!(is_format_query("  lossless  "));
        assert!(is_format_query("Mp3"));
        assert!(!is_format_query("flacx"));
        assert!(!is_format_query(""));
    }

    #[test]
    fn format_query_skips_fuzzy_text_matching() {
        // is_format_query("flac") == true => allow_fuzzy == false, поэтому трек,
        // чьё название лишь НЕЧЁТКО похоже на "flac", не должен попасть в выдачу
        // через текстовый тир — только через точное вхождение или формат файла.
        let entries = vec![
            entry("/m/1.mp3", "Flac", "X", "Al", None), // точное вхождение "flac" в title
            entry("/m/2.flac", "Unrelated", "Y", "Bl", None), // реальный .flac файл
            entry("/m/3.mp3", "Flak", "Z", "Cl", None), // нечёткое совпадение с "flac" (1 правка) — НЕ должно попасть
        ];
        let res = paths(rank(&entries, "flac", None));
        assert_eq!(res, vec!["/m/1.mp3", "/m/2.flac"]);
    }

    #[test]
    fn switch_layout_is_involutive_for_mapped_chars() {
        // Перевод раскладки туда-обратно должен возвращать исходную строку для
        // символов, присутствующих в обеих картах.
        let original = "hello world";
        let switched = switch_layout(original);
        let back = switch_layout(&switched);
        assert_eq!(back, original);
    }

    #[test]
    fn switch_layout_passes_through_unmapped_chars() {
        // Цифры и большинство пунктуации не размечены ни в одной карте.
        assert_eq!(switch_layout("12345"), "12345");
    }

    #[test]
    fn strip_ext_handles_dotfiles_and_no_extension() {
        assert_eq!(strip_ext("track"), "track");
        assert_eq!(strip_ext("track.mp3"), "track");
        assert_eq!(strip_ext("a.b.c.mp3"), "a.b.c");
        // Точка в самом конце не считается валидным разделителем расширения.
        assert_eq!(strip_ext("track."), "track.");
    }

    #[test]
    fn extension_ignores_dots_in_directory_components() {
        // Точка встречается в имени папки, а не в имени файла — расширения нет.
        assert_eq!(extension("C:/My.Folder/track"), "");
        assert_eq!(extension("/music/Artist.Name/track.flac"), "flac");
    }

    #[test]
    fn genre_filter_excludes_entries_without_genre() {
        let entries = vec![
            entry("/m/1.mp3", "Track", "A", "Al", None),
            entry("/m/2.mp3", "Track", "A", "Al", Some("Rock")),
        ];
        let res = paths(rank(&entries, "track", Some("Rock")));
        assert_eq!(res, vec!["/m/2.mp3"]);
    }

    #[test]
    fn levenshtein_distance_basic_cases() {
        assert_eq!(levenshtein(&[], &[]), 0);
        let a: Vec<char> = "kitten".chars().collect();
        let b: Vec<char> = "sitting".chars().collect();
        assert_eq!(levenshtein(&a, &b), 3);
    }
}
