// Утилиты поиска: точное совпадение, нечёткое совпадение (>=80%),
// перевод раскладки клавиатуры (rus/eng), фильтр по аудиоформату.

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

// Аудиоформаты для специального фильтра по расширению
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
    // alac часто хранится в .m4a, считаем m4a лосси (по умолчанию большинство m4a — aac)
    if (ALL_FORMATS.includes(q)) return ext === q;
    return false;
}

// Расстояние Левенштейна
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

// Минимальное расстояние от query до любой подстроки target длины ~= |query|.
// Используем DP с разрешённой "свободой" по началу/концу target (substring matching).
function approxSubstringDistance(query: string, target: string): number {
    const m = query.length;
    const n = target.length;
    if (m === 0) return 0;
    if (n === 0) return m;
    // dp[j] = минимальное расстояние, когда query сопоставляется с подстрокой target,
    // заканчивающейся в позиции j (классический алгоритм approximate string matching).
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
    threshold?: number; // допустимая доля ошибок (по умолчанию 0.2 = 80% совпадение)
}

// Нечёткое совпадение query внутри target.
// Возвращает true если есть подстрока target, отличающаяся от query
// не более чем на threshold * len(query) символов.
export function fuzzyMatch(target: string, query: string, opts: FuzzyOptions = {}): boolean {
    const minLen = opts.minQueryLen ?? 4;
    const threshold = opts.threshold ?? 0.2;
    if (query.length < minLen) return false;
    if (!target) return false;
    const maxEdits = Math.floor(query.length * threshold);
    if (maxEdits <= 0) return false;
    // быстрый путь: проверим по словам — даёт более точное и предсказуемое поведение
    const words = target.split(/\s+/);
    for (const w of words) {
        if (!w) continue;
        // если слово сильно длиннее или короче запроса — пропускаем
        if (Math.abs(w.length - query.length) > maxEdits) continue;
        if (levenshtein(w, query) <= maxEdits) return true;
    }
    // и общий подстрочный поиск (для случая, когда запрос охватывает несколько слов)
    return approxSubstringDistance(query, target) <= maxEdits;
}

// Тип элемента + поля, по которым ищем
export type FieldGetter<T> = (item: T) => string[];

export interface SearchOptions<T> {
    fuzzyMinLen?: number;
    fuzzyThreshold?: number;
    formatPath?: (item: T) => string | null; // путь к файлу — включает фильтр по формату
}

// Универсальный поиск: возвращает список в порядке
// 1) точное совпадение
// 2) нечёткое >=80% (если запрос длиннее порога)
// 3) точное совпадение в перевёрнутой раскладке
// 4) нечёткое в перевёрнутой раскладке
// 5) (опционально) совпадение по аудиоформату
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
    // Для формат-ключевых слов (lossy/lossless/flac/mp3/...) нечёткий поиск
    // даёт ложные срабатывания (например "lossy" → "glossy"), поэтому
    // оставляем только точное вхождение в название + фильтр по расширению.
    const allowFuzzy = !isFormat;

    const tier1: T[] = []; // exact match по запросу
    const tier2: T[] = []; // fuzzy по запросу
    const tier3: T[] = []; // exact match по перевёрнутой раскладке
    const tier4: T[] = []; // fuzzy по перевёрнутой раскладке
    const tier5: T[] = []; // совпадение по формату

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
