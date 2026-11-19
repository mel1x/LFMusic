// Vite aliases @tauri-apps/api/core here (vite.config.ts): __TAURI_INTERNALS__.invoke is
// read-only, so dev mode wraps it here.
import { invoke as tauriInvoke, type InvokeArgs, type InvokeOptions } from '../../node_modules/@tauri-apps/api/core.js'

export * from '../../node_modules/@tauri-apps/api/core.js'

export type InvokeHook = (cmd: string, args: InvokeArgs | undefined, run: () => Promise<unknown>) => Promise<unknown>

let hook: InvokeHook | null = null

export function setInvokeHook(next: InvokeHook | null) {
    hook = next
}

export function invoke<T>(cmd: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
    const run = () => tauriInvoke<T>(cmd, args, options)
    return (hook ? hook(cmd, args, run) : run()) as Promise<T>
}
