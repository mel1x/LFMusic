import { ref, watchEffect, type Ref } from 'vue';
import { isLinuxDesktop } from '../composables/usePlatform';

// WebKitGTK plays only http(s)/file/blob URLs, not tauri://: there the file is fetched once and
// passed as a blob URL.
const needsBlob =
    typeof location !== 'undefined' && isLinuxDesktop() && !/^https?:$/.test(location.protocol);

const blobUrls = new Map<string, Promise<string | undefined>>();

function toBlobUrl(url: string): Promise<string | undefined> {
    let pending = blobUrls.get(url);
    if (!pending) {
        pending = fetch(url)
            .then((r) => (r.ok ? r.blob() : Promise.reject(new Error(`HTTP ${r.status}`))))
            .then((blob) => URL.createObjectURL(blob))
            .catch(() => {
                blobUrls.delete(url);
                return undefined;
            });
        blobUrls.set(url, pending);
    }
    return pending;
}

export function usePlayableUrl(source: () => string | undefined): Ref<string | undefined> {
    const playable = ref<string | undefined>(needsBlob ? undefined : source());
    watchEffect(() => {
        const url = source();
        if (!needsBlob || !url) {
            playable.value = url;
            return;
        }
        playable.value = undefined;
        toBlobUrl(url).then((blobUrl) => {
            if (source() === url) playable.value = blobUrl;
        });
    });
    return playable;
}
