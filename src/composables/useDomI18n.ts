import { nextTick, watch } from 'vue'
import { translateText, useI18n } from './useI18n'

const ATTRS = ['placeholder', 'aria-label', 'title'] as const
type Attr = typeof ATTRS[number]

// What a node held before we translated it, and what we wrote. If the node still
// shows `written`, nothing re-rendered it and `source` stays the original text;
// anything else means Vue (or other code) changed it, so the current value is the
// new source. Only nodes we actually translated are tracked.
interface Tracked {
    source: string
    written: string
}

const textState = new WeakMap<Text, Tracked>()
const attrState = new WeakMap<Element, Partial<Record<Attr, Tracked>>>()

let observer: MutationObserver | null = null
let observing = false
let mutationScheduled = false
let mutationQueue: MutationRecord[] = []

function shouldSkip(node: Node): boolean {
    const parent = node.nodeType === Node.ELEMENT_NODE
        ? node as Element
        : node.parentElement
    if (!parent) return false
    return !!parent.closest('script, style, code, pre, textarea')
}

function translateTextNode(node: Text) {
    const current = node.nodeValue ?? ''
    const tracked = textState.get(node)
    const source = tracked && current === tracked.written ? tracked.source : current
    if (!source.trim()) return

    const next = translateText(source)
    // Untranslatable text (track titles, names) is neither tracked nor written.
    if (next === source && !tracked) return
    // closest() is the costly part, so it runs only when there is something to write.
    if (shouldSkip(node)) return

    textState.set(node, { source, written: next })
    if (current !== next) node.nodeValue = next
}

function translateAttrs(el: Element) {
    let state = attrState.get(el)
    for (const attr of ATTRS) {
        const current = el.getAttribute(attr)
        if (current === null) continue

        const tracked = state?.[attr]
        const source = tracked && current === tracked.written ? tracked.source : current
        if (!source.trim()) continue

        const next = translateText(source)
        if (next === source && !tracked) continue
        if (shouldSkip(el)) return

        if (!state) {
            state = {}
            attrState.set(el, state)
        }
        state[attr] = { source, written: next }
        if (current !== next) el.setAttribute(attr, next)
    }
}

function walk(root: Node) {
    if (root.nodeType === Node.TEXT_NODE) {
        translateTextNode(root as Text)
        return
    }

    if (root.nodeType !== Node.ELEMENT_NODE && root.nodeType !== Node.DOCUMENT_NODE) return

    if (root.nodeType === Node.ELEMENT_NODE) translateAttrs(root as Element)

    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT)
    let node = walker.nextNode()
    while (node) {
        if (node.nodeType === Node.TEXT_NODE) translateTextNode(node as Text)
        else translateAttrs(node as Element)
        node = walker.nextNode()
    }
}

/** Our own writes would come back as mutation records; drop them. Callers only
 *  invoke this right after a synchronous pass, so nothing else is pending. */
function dropOwnRecords() {
    observer?.takeRecords()
}

export function refreshDomI18n() {
    if (!document.body) return

    const run = () => {
        walk(document.body)
        dropOwnRecords()
    }

    run()
    nextTick(run)
    requestAnimationFrame(run)
    window.setTimeout(run, 80)
}

function flushMutationQueue() {
    mutationScheduled = false
    // Records delivered after the callback but before this microtask are still
    // queued in the observer: take them too, then drop what our writes produced.
    const records = observer ? mutationQueue.concat(observer.takeRecords()) : mutationQueue
    mutationQueue = []
    if (records.length === 0) return

    for (const record of records) {
        if (record.type === 'characterData') {
            translateTextNode(record.target as Text)
        } else if (record.type === 'attributes') {
            translateAttrs(record.target as Element)
        } else {
            record.addedNodes.forEach(walk)
        }
    }
    dropOwnRecords()
}

function scheduleMutationFlush(records: MutationRecord[]) {
    mutationQueue.push(...records)
    if (mutationScheduled) return
    mutationScheduled = true
    queueMicrotask(flushMutationQueue)
}

function setObserving(on: boolean) {
    if (!observer || on === observing) return
    observing = on
    if (on) {
        observer.observe(document.body, {
            childList: true,
            subtree: true,
            characterData: true,
            attributes: true,
            attributeFilter: [...ATTRS],
        })
    } else {
        observer.disconnect()
        mutationQueue = []
    }
}

export function startDomI18n() {
    if (observer) return

    const { language } = useI18n()

    observer = new MutationObserver(scheduleMutationFlush)

    // Legacy literals are Russian, so in Russian there is nothing to translate:
    // the observer only runs in English. The refresh still runs on every switch,
    // so text translated earlier goes back to its source.
    let previous: string | null = null
    watch(language, (lang) => {
        setObserving(lang === 'en')
        if (lang === 'en' || previous === 'en') refreshDomI18n()
        previous = lang
    }, { immediate: true, flush: 'post' })
}
