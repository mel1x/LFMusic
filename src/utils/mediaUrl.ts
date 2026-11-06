import { ref, watchEffect, type Ref } from 'vue';
import { isLinuxDesktop } from '../composables/usePlatform';

// WebKitGTK's media player only loads http(s)/file/blob URLs: bundled videos
// served over tauri:// fail right away with FormatError ("protocol not
// allowed"). There we fetch the file once and hand <video> a blob: URL instead.
// The dev server (http://localhost) works as is.
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

// URL the current platform's <video> can actually play. On Linux it stays
// undefined until the blob is ready, so render the <source> with v-if.
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
