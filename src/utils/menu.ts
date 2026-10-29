/** Context menu position centered under the clicked button (`e.currentTarget`). */
export function menuAnchorBelow(e: MouseEvent): { x: number; y: number } {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
    return { x: rect.left + rect.width / 2, y: rect.bottom + 6 }
}
