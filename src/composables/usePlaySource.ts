import { usePlayer, type PlayTrackOptions } from './usePlayer'
import { usePlaylists } from './usePlaylists'

/** Moves the items named in `topOrder` (pinned "move to top") to the front, keeping the rest in order. */
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

/**
 * Play button of an album / artist / playlist card: toggles pause when that
 * source is already playing, otherwise loads its tracks and starts from the first.
 */
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
        // Live cache objects, so covers loaded into the cache show up on these.
        const merged = tracks.map(t => getTrackByPath(t.file_path) ?? t)
        setPlaylistSource(sourceId, merged)
        beforePlay?.()
        await playTrack(merged[0], options)
    }

    return { isSourceActive, playSource }
}
