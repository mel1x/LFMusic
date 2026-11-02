import { computed, ref } from 'vue'
import { useSettings } from './useSettings'

// Holiday events: during a holiday window the app gets a themed background
// pattern (HolidayPattern) and ambient/click effects (HolidayEffects).
// Dates are local [month, day], inclusive; a window may wrap over the new year.

export type HolidayId =
    | 'new_year'
    | 'valentine'
    | 'womens_day'
    | 'cosmonautics'
    | 'music_day'
    | 'halloween'

interface HolidayDef {
    id: HolidayId
    from: [number, number]
    to: [number, number]
}

export const HOLIDAYS: HolidayDef[] = [
    { id: 'new_year',     from: [12, 20], to: [1, 10] },
    { id: 'valentine',    from: [2, 13],  to: [2, 15] },
    { id: 'womens_day',   from: [3, 7],   to: [3, 9] },
    { id: 'cosmonautics', from: [4, 11],  to: [4, 13] },
    // Fete de la Musique / World Music Day.
    { id: 'music_day',    from: [6, 20],  to: [6, 22] },
    { id: 'halloween',    from: [10, 29], to: [11, 1] },
]

// Preview any theme without waiting for the date:
// localStorage.setItem('lfmusic_holiday', 'halloween') + reload ('none' disables).
const OVERRIDE_KEY = 'lfmusic_holiday'

function readOverride(): HolidayId | 'none' | null {
    try {
        const v = localStorage.getItem(OVERRIDE_KEY)
        if (v === 'none') return 'none'
        return HOLIDAYS.some(h => h.id === v) ? v as HolidayId : null
    } catch {
        return null
    }
}

export function holidayForDate(date: Date): HolidayId | null {
    const md = (date.getMonth() + 1) * 100 + date.getDate()
    for (const h of HOLIDAYS) {
        const from = h.from[0] * 100 + h.from[1]
        const to = h.to[0] * 100 + h.to[1]
        const inside = from <= to ? md >= from && md <= to : md >= from || md <= to
        if (inside) return h.id
    }
    return null
}

/** Holiday info modal, opened from the titlebar badge. */
export const holidayModalOpen = ref(false)

const now = ref(new Date())
const override = readOverride()
let started = false

function tick() {
    const next = new Date()
    if (next.toDateString() !== now.value.toDateString()) now.value = next
}

export const activeHoliday = computed<HolidayId | null>(() => {
    const { settings } = useSettings()
    if (!settings.value.holiday_events) return null
    if (override === 'none') return null
    return override ?? holidayForDate(now.value)
})

export function useHoliday() {
    if (!started) {
        started = true
        // The app can stay open across midnight; also recheck when the window wakes up.
        setInterval(tick, 10 * 60 * 1000)
        document.addEventListener('visibilitychange', () => {
            if (!document.hidden) tick()
        })
    }
    return { activeHoliday }
}
