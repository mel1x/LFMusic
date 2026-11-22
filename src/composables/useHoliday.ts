import { computed, ref } from 'vue'
import { useSettings } from './useSettings'

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
    { id: 'music_day',    from: [6, 20],  to: [6, 22] },
    { id: 'halloween',    from: [10, 29], to: [11, 1] },
]

export type HolidayOverride = HolidayId | 'none' | null

const OVERRIDE_KEY = 'lfmusic_holiday'

function readOverride(): HolidayOverride {
    try {
        const v = localStorage.getItem(OVERRIDE_KEY)
        if (v === 'none') return 'none'
        return HOLIDAYS.some(h => h.id === v) ? v as HolidayId : null
    } catch {
        return null
    }
}

/** Holiday info modal, opened from the titlebar badge. */
export const holidayModalOpen = ref(false)
export const holidayOverride = ref<HolidayOverride>(readOverride())

export function setHolidayOverride(next: HolidayOverride) {
    holidayOverride.value = next
    try {
        if (next === null) localStorage.removeItem(OVERRIDE_KEY)
        else localStorage.setItem(OVERRIDE_KEY, next)
    } catch {
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

const now = ref(new Date())
let started = false

function tick() {
    const next = new Date()
    if (next.toDateString() !== now.value.toDateString()) now.value = next
}

export const activeHoliday = computed<HolidayId | null>(() => {
    if (holidayOverride.value === 'none') return null
    if (holidayOverride.value) return holidayOverride.value
    const { settings } = useSettings()
    if (!settings.value.holiday_events) return null
    return holidayForDate(now.value)
})

export function useHoliday() {
    if (!started) {
        started = true
        setInterval(tick, 10 * 60 * 1000)
        document.addEventListener('visibilitychange', () => {
            if (!document.hidden) tick()
        })
    }
    return { activeHoliday }
}
