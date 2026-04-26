import { ref } from 'vue'
import type { TrackMetadata } from './usePlayer'

const isOpen = ref(false)
const currentTrack = ref<TrackMetadata | null>(null)

export function useTrackInfoModal() {
    function openTrackInfo(track: TrackMetadata) {
        currentTrack.value = track
        isOpen.value = true
    }

    function closeTrackInfo() {
        isOpen.value = false
    }

    return {
        isOpen,
        currentTrack,
        openTrackInfo,
        closeTrackInfo,
    }
}
