import { usePlayer, type PlayTrackOptions } from './usePlayer'
import { usePlaylists } from './usePlaylists'

/**
 * Moves the items named in `topOrder` (pinned "move to top") to the front, keeping the rest in
 * order.
 */
export function sortByTopOrder<T extends { name: string }>(items: T[], topOrder: string[]): T[] {
    const byName = new Map(items.map(a => [a.name, a]))
    const topSet = new Set(topOrder)
    const top: T[] = []
    for (const name of topOrder) {
        const a = byName.get(name)
        if (a) top.push(a)
    }
    return [...top, ...items.filter(a => !topSet.has(a.name))]
}

export function usePlaySource() {
    const { activePlaylistSource, togglePlayPause, getTrackByPath, setPlaylistSource, playTrack } = usePlayer()
    const { getPlaylistTracks } = usePlaylists()

    function isSourceActive(sourceId: string): boolean {
        return activePlaylistSource.value === sourceId
    }

    async function playSource(
        sourceId: string,
        playlistId: number,
        options: PlayTrackOptions = {},
        beforePlay?: () => void,
    ) {
        if (isSourceActive(sourceId)) {
            togglePlayPause()
            return
        }
        const tracks = await getPlaylistTracks(playlistId)
        if (tracks.length === 0) return
        const merged = tracks.map(t => getTrackByPath(t.file_path) ?? t)
        setPlaylistSource(sourceId, merged)
        beforePlay?.()
        await playTrack(merged[0], options)
    }

    return { isSourceActive, playSource }
}
