import { ref } from 'vue'
import type { TrackMetadata } from './usePlayer'

const isOpen = ref(false)
const currentTrack = ref<TrackMetadata | null>(null)

export function useTrackEditModal() {
    function openTrackEdit(track: TrackMetadata) {
        currentTrack.value = track
        isOpen.value = true
    }

    function closeTrackEdit() {
        isOpen.value = false
    }

    return {
        isOpen,
        currentTrack,
        openTrackEdit,
        closeTrackEdit,
    }
}
