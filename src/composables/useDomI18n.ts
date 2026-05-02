import { nextTick, watch } from 'vue'
import { translateText, useI18n } from './useI18n'

const textSources = new WeakMap<Text, string>()
const ATTRS = ['placeholder', 'aria-label', 'title'] as const

let observer: MutationObserver | null = null
let translating = false
let mutationScheduled = false
let mutationQueue: MutationRecord[] = []

function shouldSkip(node: Node): boolean {
    const parent = node.nodeType === Node.ELEMENT_NODE
        ? node as Element
        : node.parentElement
    if (!parent) return false
    return !!parent.closest('script, style, code, pre, textarea')
}

function translatedFromSource(source: string): string {
    return translateText(source)
}

function translateTextNode(node: Text) {
    if (shouldSkip(node)) return
    const current = node.nodeValue ?? ''
    if (!current.trim()) return

    const previousSource = textSources.get(node)
    const previousTranslation = previousSource ? translatedFromSource(previousSource) : null
    const source = !previousSource || (current !== previousSource && current !== previousTranslation)
        ? current
        : previousSource

    if (source !== previousSource) textSources.set(node, source)

    const next = translatedFromSource(source)
    if (current !== next) node.nodeValue = next
}

function translateAttrs(el: Element) {
    if (shouldSkip(el)) return

    for (const attr of ATTRS) {
        if (!el.hasAttribute(attr)) continue

        const current = el.getAttribute(attr) ?? ''
        if (!current.trim()) continue

        const key = `i18nSource${attr.replace(/(^|-)(\w)/g, (_, __, c: string) => c.toUpperCase())}`
        const previousSource = (el as HTMLElement).dataset[key]
        const previousTranslation = previousSource ? translatedFromSource(previousSource) : null
        const source = !previousSource || (current !== previousSource && current !== previousTranslation)
            ? current
            : previousSource

        ;(el as HTMLElement).dataset[key] = source

        const next = translatedFromSource(source)
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

export function refreshDomI18n() {
    if (!document.body) return

    const run = () => {
        translating = true
        walk(document.body)
        translating = false
    }

    run()
    nextTick(run)
    requestAnimationFrame(run)
    window.setTimeout(run, 80)
}

function flushMutationQueue() {
    mutationScheduled = false
    if (translating || mutationQueue.length === 0) return

    const records = mutationQueue
    mutationQueue = []

    translating = true
    for (const record of records) {
        if (record.type === 'characterData') {
            translateTextNode(record.target as Text)
        } else if (record.type === 'attributes') {
            translateAttrs(record.target as Element)
        } else {
            record.addedNodes.forEach(walk)
        }
    }
    translating = false
}

function scheduleMutationFlush(records: MutationRecord[]) {
    mutationQueue.push(...records)
    if (mutationScheduled) return
    mutationScheduled = true
    queueMicrotask(flushMutationQueue)
}

export function startDomI18n() {
    if (observer) return

    const { language } = useI18n()

    observer = new MutationObserver((records) => {
        if (translating) return
        scheduleMutationFlush(records)
    })

    observer.observe(document.body, {
        childList: true,
        subtree: true,
        characterData: true,
        attributes: true,
        attributeFilter: [...ATTRS],
    })

    watch(language, refreshDomI18n, { immediate: true, flush: 'post' })
}
