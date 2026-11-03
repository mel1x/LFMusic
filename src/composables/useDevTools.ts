import { setInvokeHook } from '../utils/tauriCore'

// Developer mode capture: hooks app `invoke` calls and collects JS errors.
// Installed in main.ts before mount so startup calls are recorded too; if the
// saved settings say dev mode is off, `stopDevCapture` removes every hook.
// Data lives in plain arrays (no Vue reactivity per call); the panel polls it.

export interface DevCall {
    id: number
    cmd: string
    args: string
    start: number
    ms: number | null
    error: string | null
}

export interface DevCommandStat {
    cmd: string
    count: number
    errors: number
    totalMs: number
    maxMs: number
    last: number
}

export interface DevError {
    time: number
    source: 'invoke' | 'error' | 'promise' | 'console'
    message: string
}

const RECENT_LIMIT = 200
const ERROR_LIMIT = 200
const ARGS_LIMIT = 300

export const devStartedAt = Date.now()
export const devCalls: DevCall[] = []
export const devStats = new Map<string, DevCommandStat>()
export const devErrors: DevError[] = []

let nextId = 1
let restore: (() => void) | null = null

function stringify(value: unknown): string {
    if (value === undefined) return ''
    try {
        const s = JSON.stringify(value, (_, v) =>
            v instanceof ArrayBuffer || ArrayBuffer.isView(v) ? `<${(v as ArrayBuffer).byteLength} bytes>` : v)
        return s.length > ARGS_LIMIT ? `${s.slice(0, ARGS_LIMIT)}...` : s
    } catch {
        return String(value)
    }
}

function errorText(e: unknown): string {
    if (e instanceof Error) return e.stack || e.message
    if (typeof e === 'string') return e
    return stringify(e)
}

function pushError(source: DevError['source'], message: string) {
    devErrors.push({ time: Date.now(), source, message })
    if (devErrors.length > ERROR_LIMIT) devErrors.splice(0, devErrors.length - ERROR_LIMIT)
}

function finishCall(call: DevCall, error: unknown) {
    call.ms = performance.now() - call.start
    const stat = devStats.get(call.cmd) ?? { cmd: call.cmd, count: 0, errors: 0, totalMs: 0, maxMs: 0, last: 0 }
    stat.count++
    stat.totalMs += call.ms
    stat.maxMs = Math.max(stat.maxMs, call.ms)
    stat.last = Date.now()
    if (error !== undefined) {
        call.error = errorText(error)
        stat.errors++
        pushError('invoke', `${call.cmd}: ${call.error}`)
    }
    devStats.set(call.cmd, stat)
}

export function installDevCapture() {
    if (restore) return
    const undo: (() => void)[] = []

    setInvokeHook((cmd, args, run) => {
        const call: DevCall = { id: nextId++, cmd, args: stringify(args), start: performance.now(), ms: null, error: null }
        devCalls.push(call)
        if (devCalls.length > RECENT_LIMIT) devCalls.splice(0, devCalls.length - RECENT_LIMIT)
        const result = run()
        result.then(() => finishCall(call, undefined), e => finishCall(call, e ?? 'error'))
        return result
    })
    undo.push(() => setInvokeHook(null))

    const onError = (e: ErrorEvent) => pushError('error', e.error ? errorText(e.error) : e.message)
    const onRejection = (e: PromiseRejectionEvent) => pushError('promise', errorText(e.reason))
    window.addEventListener('error', onError)
    window.addEventListener('unhandledrejection', onRejection)
    undo.push(() => {
        window.removeEventListener('error', onError)
        window.removeEventListener('unhandledrejection', onRejection)
    })

    const originalConsoleError = console.error
    console.error = (...args: unknown[]) => {
        pushError('console', args.map(a => (typeof a === 'string' ? a : errorText(a))).join(' '))
        originalConsoleError.apply(console, args)
    }
    undo.push(() => { console.error = originalConsoleError })

    restore = () => undo.forEach(fn => fn())
}

export function stopDevCapture() {
    restore?.()
    restore = null
    devCalls.length = 0
    devStats.clear()
    devErrors.length = 0
}

export function clearDevCalls() {
    devCalls.length = 0
    devStats.clear()
}

export function clearDevErrors() {
    devErrors.length = 0
}
