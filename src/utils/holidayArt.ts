import type { HolidayId } from '../composables/useHoliday'

// Artwork for holiday events: 24x24 glyphs shared by the background pattern
// (SVG tile used as a CSS mask) and the canvas particles (Path2D).

type Pt = [number, number]

function rotate([x, y]: Pt, deg: number): Pt {
    const a = deg * Math.PI / 180
    const dx = x - 12, dy = y - 12
    const r = (v: number) => Math.round(v * 100) / 100
    return [r(12 + dx * Math.cos(a) - dy * Math.sin(a)), r(12 + dx * Math.sin(a) + dy * Math.cos(a))]
}

function ellipse(cx: number, cy: number, rx: number, ry: number): string {
    return `M${cx - rx} ${cy}a${rx} ${ry} 0 1 0 ${rx * 2} 0a${rx} ${ry} 0 1 0 ${-rx * 2} 0z`
}

function snowflake(): string {
    let d = ''
    for (let k = 0; k < 6; k++) {
        const a = k * 60
        const tip = rotate([12, 2.5], a)
        const stem = rotate([12, 6.2], a)
        const b1 = rotate([9.4, 4], a)
        const b2 = rotate([14.6, 4], a)
        d += `M12 12L${tip[0]} ${tip[1]}M${b1[0]} ${b1[1]}L${stem[0]} ${stem[1]}L${b2[0]} ${b2[1]}`
    }
    return d
}

function daisy(): string {
    let d = ellipse(12, 12, 2.4, 2.4)
    for (let k = 0; k < 6; k++) {
        const a = k * 60
        const p = [[10, 9], [10, 4.5], [12, 3.2], [14, 4.5], [14, 9], [12, 12]].map(pt => rotate(pt as Pt, a))
        d += `M12 12C${p[0]} ${p[1]} ${p[2]}C${p[3]} ${p[4]} ${p[5]}z`.replace(/,/g, ' ')
    }
    return d
}

export const PATHS = {
    heart: 'M12 21C12 21 2.5 15.2 2.5 8.8 2.5 5.6 4.9 3.5 7.6 3.5c1.9 0 3.4 1 4.4 2.5 1-1.5 2.5-2.5 4.4-2.5 2.7 0 5.1 2.1 5.1 5.3C21.5 15.2 12 21 12 21z',
    star: 'M12 2l2.9 6.6 7.1.7-5.4 4.7 1.6 7L12 17.3 5.8 21l1.6-7L2 9.3l7.1-.7z',
    sparkle: 'M12 2C12.8 8 16 11.2 22 12 16 12.8 12.8 16 12 22 11.2 16 8 12.8 2 12 8 11.2 11.2 8 12 2z',
    flake: snowflake(),
    note: ellipse(8.5, 18, 3.5, 2.6) + 'M10.4 18V3h1.8v15zM12.2 3c.4 3 5.2 3.6 4.6 8.4-.6-2.6-2.6-3.8-4.6-4.2z',
    notes: ellipse(6.6, 18.5, 3.3, 2.5) + ellipse(17, 16.5, 3.3, 2.5) + 'M8.6 18.5V6.2L20.3 3.6v12.9h-1.6V6.6l-8.5 1.9v10z',
    bat: 'M12 9.6 11 7.9l-.5 1.8C8.4 8.5 4.9 8.1 1.5 9.6c2 1 3 2.6 3 4.4 1.5-1 3.5-.8 4.7.3.8-.8 1.8-1.1 2.8-1.1s2 .3 2.8 1.1c1.2-1.1 3.2-1.3 4.7-.3 0-1.8 1-3.4 3-4.4-3.4-1.5-6.9-1.1-9 .1L13 7.9z',
    tulip: 'M12 13.5c-3.2 0-5.2-2.6-5.2-6.2 0-1.6.4-3.1 1-4.3l2.2 2.3L12 2.2l2 3.1 2.2-2.3c.6 1.2 1 2.7 1 4.3 0 3.6-2 6.2-5.2 6.2zM11.3 13h1.4v9h-1.4zM11.8 20.5c-.8-3-3.3-4.8-6.3-4.8 1 3.1 3.4 4.8 6.3 4.8z',
    daisy: daisy(),
    pumpkin: ellipse(8.5, 14.2, 5, 5.8) + ellipse(15.5, 14.2, 5, 5.8) + ellipse(12, 14.2, 4, 6.3) + 'M11.2 8.4c0-2.2.8-3.8 2.8-4.6l.8 1.2c-1.4.7-1.9 1.9-1.9 3.4z',
    moon: 'M15.5 2.5A9.5 9.5 0 1 0 21.5 19 10 10 0 0 1 15.5 2.5z',
    rocket: 'M12 1.8c3.1 2.6 4.6 6.2 4.6 10.2l-1.4 4.2H8.8L7.4 12c0-4 1.5-7.6 4.6-10.2z' + ellipse(12, 9, 1.7, 1.7) + 'M7.5 12.8l-3 3.6v3l4-2.4zM16.5 12.8l3 3.6v3l-4-2.4zM10.2 17.4h3.6L12 22.2z',
    planet: ellipse(12, 12, 5.4, 5.4),
    ornament: ellipse(12, 14.5, 6.5, 6.5) + 'M10 6.2h4v2.4h-4z',
    tree: 'M12 2l4.5 6h-2.3l4.3 5.5h-2.6l4.6 5.5h-7.3v3h-2.4v-3H3.5l4.6-5.5H5.5L9.8 8H7.5z',
    dot: ellipse(12, 12, 1.8, 1.8),
} as const

// Outline-only parts (drawn with stroke in the pattern).
const STROKES: Partial<Record<Glyph, string>> = {
    flake: PATHS.flake,
    planet: 'M12 12m-10.5 0a10.5 3.4 0 1 0 21 0a10.5 3.4 0 1 0 -21 0',
    ornament: 'M12 6.2c-1.4-1.6.2-3.6 1.4-2.2',
}

type Glyph = keyof typeof PATHS

function glyphMarkup(g: Glyph, color = '#fff'): string {
    // evenodd cuts the rocket window out of its body.
    const rule = g === 'rocket' ? ' fill-rule="evenodd"' : ''
    const fill = g !== 'flake' ? `<path d="${PATHS[g]}" fill="${color}"${rule}/>` : ''
    const strokeD = STROKES[g]
    const stroke = strokeD
        ? `<path d="${strokeD}" fill="none" stroke="${color}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"${g === 'planet' ? ' transform="rotate(-22 12 12)"' : ''}/>`
        : ''
    return fill + stroke
}

export const TILE_SIZE = 140

// Pattern tile is drawn this much larger than its SVG.
export const TILE_SCALE = 1.7

// Hand-placed slots of one tile: [cx, cy, size, rotation]. Two big, three mid,
// one small glyph and dots keep the dense pattern from reading as a grid and
// leave something in the thin gaps between app surfaces.
const SLOTS: [number, number, number, number][] = [
    [22, 24, 18, -14], [76, 16, 10, 18], [118, 36, 14, 10], [50, 64, 5, 0],
    [100, 82, 17, -8], [22, 104, 11, 12], [64, 120, 14, -22], [126, 122, 6, 0],
    [88, 46, 4, 0], [40, 84, 4, 0], [134, 70, 4, 0], [100, 132, 4, 0],
]

export interface HolidayArt {
    /** Glyphs of the first slots; the rest are dots. */
    glyphs: Glyph[]
    /** Glyph of the titlebar badge. */
    badge: Glyph
    /** Theme accent while the static (non-dynamic) color theme is on. */
    accent: string
}

export const HOLIDAY_ART: Record<HolidayId, HolidayArt> = {
    new_year: {
        glyphs: ['flake', 'sparkle', 'ornament', 'dot', 'tree', 'star', 'flake', 'dot'],
        badge: 'tree',
        accent: '#1c3854',
    },
    valentine: {
        glyphs: ['heart', 'sparkle', 'heart', 'dot', 'heart', 'sparkle', 'heart', 'dot'],
        badge: 'heart',
        accent: '#541c2a',
    },
    womens_day: {
        glyphs: ['tulip', 'sparkle', 'daisy', 'dot', 'tulip', 'daisy', 'daisy', 'dot'],
        badge: 'tulip',
        accent: '#562541',
    },
    cosmonautics: {
        glyphs: ['planet', 'star', 'rocket', 'dot', 'moon', 'sparkle', 'star', 'dot'],
        badge: 'rocket',
        accent: '#1f2351',
    },
    music_day: {
        glyphs: ['notes', 'note', 'sparkle', 'dot', 'note', 'sparkle', 'notes', 'dot'],
        badge: 'notes',
        accent: '#392259',
    },
    halloween: {
        glyphs: ['bat', 'star', 'pumpkin', 'dot', 'moon', 'bat', 'pumpkin', 'dot'],
        badge: 'pumpkin',
        accent: '#522e14',
    },
}

const tileCache = new Map<HolidayId, string>()

/** `url(...)` of the pattern tile for `mask-image`. */
export function holidayTile(id: HolidayId): string {
    const cached = tileCache.get(id)
    if (cached) return cached
    const { glyphs } = HOLIDAY_ART[id]
    const items = SLOTS.map(([x, y, size, rot], i) => {
        const g = glyphs[i] ?? 'dot'
        return `<g transform="translate(${x} ${y}) rotate(${rot}) scale(${size / 24}) translate(-12 -12)">${glyphMarkup(g)}</g>`
    }).join('')
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${TILE_SIZE}" height="${TILE_SIZE}" viewBox="0 0 ${TILE_SIZE} ${TILE_SIZE}">${items}</svg>`
    const url = `url("data:image/svg+xml,${encodeURIComponent(svg)}")`
    tileCache.set(id, url)
    return url
}

/** Inline SVG of the holiday's badge glyph, drawn in `currentColor`. */
export function holidayBadgeSvg(id: HolidayId, size = 17): string {
    return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true">${glyphMarkup(HOLIDAY_ART[id].badge, 'currentColor')}</svg>`
}
