import { ref, watch, effectScope } from 'vue'
import { usePlayer, type TrackMetadata } from './usePlayer'
import { getRecentlyPlayed, getTopArtists, TRACK_STATISTICS_UPDATED_EVENT } from './useStatistics'

// What the search page shows for an empty query: most played artists and
// recently played tracks. Kept between visits so the page opens filled in;
// refetched only after listening changed it (a listen was counted or another
// track started), not on every open.

const topArtistStats = ref<{ name: string; listens: number }[]>([])
const recentTracks = ref<TrackMetadata[]>([])
const loaded = ref(false)

let stale = true
let loading: Promise<void> | null = null
let watchersSet = false

function markStale() {
    stale = true
}

function setupWatchers() {
    if (watchersSet) return
    watchersSet = true
    window.addEventListener(TRACK_STATISTICS_UPDATED_EVENT, markStale)
    // Detached scope: first called from a page's onMounted, but must outlive that page.
    effectScope(true).run(() => {
        watch(() => usePlayer().currentTrack.value?.file_path, markStale)
    })
}

async function load() {
    const { cacheTracks, getTrackByPath, loadCoversForTracks } = usePlayer()
    stale = false
    const [artists, recent] = await Promise.all([
        getTopArtists(12).catch(() => null),
        getRecentlyPlayed(6).catch(() => null),
    ])
    // Keep what we had if a request failed.
    if (artists) topArtistStats.value = artists.map(a => ({ name: a.name, listens: a.listen_count }))
    if (recent) {
        cacheTracks(recent)
        recentTracks.value = recent.map(tr => getTrackByPath(tr.file_path) ?? tr)
        loadCoversForTracks(recentTracks.value)
    }
    loaded.value = true
}

/** Refetches only when the data is stale; old data stays on screen meanwhile. */
function refresh(): Promise<void> {
    setupWatchers()
    if (!stale) return Promise.resolve()
    if (!loading) loading = load().finally(() => { loading = null })
    return loading
}

export function useSearchBrowse() {
    return { topArtistStats, recentTracks, loaded, refresh }
}
