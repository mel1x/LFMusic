// Vite aliases `@tauri-apps/api/core` to this file (see vite.config.ts) so dev
// mode can record backend calls: Tauri defines `__TAURI_INTERNALS__.invoke` as
// read-only, so it can't be wrapped at runtime. Everything else is re-exported as is.
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
