// Search utilities: exact match, fuzzy match (>=80%), RU/EN keyboard-layout
// translation, audio-format filter.

const EN_TO_RU: Record<string, string> = {
    'q': 'й', 'w': 'ц', 'e': 'у', 'r': 'к', 't': 'е', 'y': 'н', 'u': 'г',
    'i': 'ш', 'o': 'щ', 'p': 'з', '[': 'х', ']': 'ъ',
    'a': 'ф', 's': 'ы', 'd': 'в', 'f': 'а', 'g': 'п', 'h': 'р', 'j': 'о',
    'k': 'л', 'l': 'д', ';': 'ж', "'": 'э',
    'z': 'я', 'x': 'ч', 'c': 'с', 'v': 'м', 'b': 'и', 'n': 'т', 'm': 'ь',
    ',': 'б', '.': 'ю', '/': '.',
    '`': 'ё',
};

const RU_TO_EN: Record<string, string> = Object.fromEntries(
    Object.entries(EN_TO_RU).map(([k, v]) => [v, k])
);

export function switchLayout(input: string): string {
    let out = '';
    for (const ch of input) {
        const lower = ch.toLowerCase();
        if (EN_TO_RU[lower]) out += EN_TO_RU[lower];
        else if (RU_TO_EN[lower]) out += RU_TO_EN[lower];
        else out += ch;
    }
    return out;
}

// Audio formats for the extension-based filter
export const LOSSLESS_FORMATS = ['flac', 'wav', 'alac', 'ape', 'dsd', 'dsf', 'aiff', 'aif', 'wv', 'tak'];
export const LOSSY_FORMATS = ['mp3', 'aac', 'ogg', 'm4a', 'wma', 'opus'];
export const ALL_FORMATS = [...LOSSLESS_FORMATS, ...LOSSY_FORMATS];

export function getExtension(filePath: string): string {
    const m = filePath.match(/\.([^.\\/]+)$/);
    return m ? m[1].toLowerCase() : '';
}

export function isFormatQuery(query: string): boolean {
    const q = query.toLowerCase().trim();
    if (q === 'lossless' || q === 'lossy') return true;
    return ALL_FORMATS.includes(q);
}

export function matchesFormat(filePath: string, query: string): boolean {
    const q = query.toLowerCase().trim();
    const ext = getExtension(filePath);
    if (q === 'lossless') return LOSSLESS_FORMATS.includes(ext);
    if (q === 'lossy') return LOSSY_FORMATS.includes(ext);
    // ALAC is often stored as .m4a, but most .m4a is AAC, so we treat it as lossy.
    if (ALL_FORMATS.includes(q)) return ext === q;
    return false;
}

// Levenshtein distance
function levenshtein(a: string, b: string): number {
    if (a === b) return 0;
    if (!a.length) return b.length;
    if (!b.length) return a.length;
    const m = a.length, n = b.length;
    let prev = new Array(n + 1);
    let curr = new Array(n + 1);
    for (let j = 0; j <= n; j++) prev[j] = j;
    for (let i = 1; i <= m; i++) {
        curr[0] = i;
        const ca = a.charCodeAt(i - 1);
        for (let j = 1; j <= n; j++) {
            const cost = ca === b.charCodeAt(j - 1) ? 0 : 1;
            curr[j] = Math.min(
                curr[j - 1] + 1,
                prev[j] + 1,
                prev[j - 1] + cost
            );
        }
        [prev, curr] = [curr, prev];
    }
    return prev[n];
}

// Minimum distance from query to any substring of target with length ~= |query|.
// DP allows free start/end offsets in target (substring matching).
function approxSubstringDistance(query: string, target: string): number {
    const m = query.length;
    const n = target.length;
    if (m === 0) return 0;
    if (n === 0) return m;
    // dp[j] = min distance matching query against a substring of target ending at j
    // (classic approximate string matching algorithm).
    let prev = new Array(n + 1).fill(0);
    const curr = new Array(n + 1).fill(0);
    for (let i = 1; i <= m; i++) {
        curr[0] = i;
        const cq = query.charCodeAt(i - 1);
        for (let j = 1; j <= n; j++) {
            const cost = cq === target.charCodeAt(j - 1) ? 0 : 1;
            curr[j] = Math.min(
                curr[j - 1] + 1,
                prev[j] + 1,
                prev[j - 1] + cost
            );
        }
        prev = curr.slice();
    }
    let min = m;
    for (let j = 0; j <= n; j++) if (prev[j] < min) min = prev[j];
    return min;
}

export interface FuzzyOptions {
    minQueryLen?: number;
    threshold?: number; // allowed error ratio (default 0.2 = 80% match)
}

// Fuzzy-matches query inside target: true if some substring of target
// differs from query by no more than threshold * len(query) chars.
export function fuzzyMatch(target: string, query: string, opts: FuzzyOptions = {}): boolean {
    const minLen = opts.minQueryLen ?? 4;
    const threshold = opts.threshold ?? 0.2;
    if (query.length < minLen) return false;
    if (!target) return false;
    const maxEdits = Math.floor(query.length * threshold);
    if (maxEdits <= 0) return false;
    // Fast path: check per word first, more precise and predictable.
    const words = target.split(/\s+/);
    for (const w of words) {
        if (!w) continue;
        if (Math.abs(w.length - query.length) > maxEdits) continue;
        if (levenshtein(w, query) <= maxEdits) return true;
    }
    // General substring search, for queries spanning multiple words.
    return approxSubstringDistance(query, target) <= maxEdits;
}

export type FieldGetter<T> = (item: T) => string[];

export interface SearchOptions<T> {
    fuzzyMinLen?: number;
    fuzzyThreshold?: number;
    formatPath?: (item: T) => string | null; // file path, enables the format filter
}

// Ranks results in tiers:
// 1) exact match, 2) fuzzy >=80%, 3) exact match with layout swapped,
// 4) fuzzy with layout swapped, 5) (optional) audio-format match.
export function searchRanked<T>(
    items: T[],
    query: string,
    getFields: FieldGetter<T>,
    opts: SearchOptions<T> = {},
): T[] {
    const q = query.toLowerCase().trim();
    if (!q) return items;

    const fuzzyMinLen = opts.fuzzyMinLen ?? 4;
    const fuzzyThreshold = opts.fuzzyThreshold ?? 0.2;
    const formatPath = opts.formatPath;
    const switched = switchLayout(q);
    const useSwitched = switched !== q;
    const isFormat = !!formatPath && isFormatQuery(q);
    // Fuzzy match on format keywords (lossy/lossless/flac/mp3/...) causes false
    // positives (e.g. "lossy" -> "glossy"), so we only allow exact match plus
    // the extension filter for those.
    const allowFuzzy = !isFormat;

    const tier1: T[] = []; // exact match on query
    const tier2: T[] = []; // fuzzy on query
    const tier3: T[] = []; // exact match with layout swapped
    const tier4: T[] = []; // fuzzy with layout swapped
    const tier5: T[] = []; // format match

    for (const item of items) {
        const fields = getFields(item).map(s => (s || '').toLowerCase());

        if (fields.some(f => f.includes(q))) {
            tier1.push(item);
            continue;
        }

        if (allowFuzzy && q.length >= fuzzyMinLen &&
            fields.some(f => fuzzyMatch(f, q, { minQueryLen: fuzzyMinLen, threshold: fuzzyThreshold }))) {
            tier2.push(item);
            continue;
        }

        if (useSwitched && fields.some(f => f.includes(switched))) {
            tier3.push(item);
            continue;
        }

        if (allowFuzzy && useSwitched && switched.length >= fuzzyMinLen &&
            fields.some(f => fuzzyMatch(f, switched, { minQueryLen: fuzzyMinLen, threshold: fuzzyThreshold }))) {
            tier4.push(item);
            continue;
        }

        if (isFormat && formatPath) {
            const p = formatPath(item);
            if (p && matchesFormat(p, q)) {
                tier5.push(item);
                continue;
            }
        }
    }

    return [...tier1, ...tier2, ...tier3, ...tier4, ...tier5];
}
