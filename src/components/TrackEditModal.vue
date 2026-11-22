<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'

import CloseIcon from './Icons/CloseIcon.vue'
import EditIcon from './Icons/EditIcon.vue'
import EmptyIcon from './Icons/EmptyIcon.vue'
import { useTrackEditModal } from '../composables/useTrackEditModal'
import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import { useSheetModal } from '../composables/useSheetModal'

const { isOpen, currentTrack, closeTrackEdit } = useTrackEditModal()
const { refreshLibrary, getTrackByPath, cacheTracks, currentTrack: playerCurrentTrack } = usePlayer()

interface OriginalTrack {
    title: string | null
    artist: string | null
    coverArt: string | null
    hasOverrideCover: boolean
}

const titleInput = ref('')
const artistInput = ref('')
const coverPreview = ref<string | null>(null)
const coverSourcePath = ref<string | null>(null)
const coverReset = ref(false)

const original = ref<OriginalTrack | null>(null)
const loadingOriginal = ref(false)

const saving = ref(false)
const errorText = ref('')
const modalCardEl = ref<HTMLElement | null>(null)
const { onBackdropMousedown, onSheetDragStart } = useSheetModal(modalCardEl, closeTrackEdit)

const MAX_FIELD_LINES = 2
function autoGrowField(el: HTMLTextAreaElement) {
    el.style.height = 'auto'
    const cs = getComputedStyle(el)
    const lineHeight = parseFloat(cs.lineHeight) || (parseFloat(cs.fontSize) * 1.35)
    const extra = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom)
        + parseFloat(cs.borderTopWidth) + parseFloat(cs.borderBottomWidth)
    const maxH = lineHeight * MAX_FIELD_LINES + extra
    el.style.height = Math.min(el.scrollHeight, maxH) + 'px'
    el.style.overflowY = el.scrollHeight > maxH + 1 ? 'auto' : 'hidden'
}
const vAutogrow = {
    mounted(el: HTMLTextAreaElement) { autoGrowField(el) },
    updated(el: HTMLTextAreaElement) { autoGrowField(el) },
}
function onFieldEnter(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey) return
    e.preventDefault()
}

const fileName = computed(() => {
    const p = currentTrack.value?.file_path || ''
    return p.split(/[\\/]/).pop() || p
})

const hasCoverChange = computed(() => coverSourcePath.value !== null || coverReset.value)
const hasChanges = computed(() => {
    if (!currentTrack.value) return false
    const origTitle = currentTrack.value.title || ''
    const origArtist = currentTrack.value.artist || ''
    return (
        titleInput.value.trim() !== origTitle.trim() ||
        artistInput.value.trim() !== origArtist.trim() ||
        hasCoverChange.value
    )
})

const previewIsOriginal = computed(() => {
    if (coverSourcePath.value) return false
    if (!original.value) return false
    return (coverPreview.value || null) === (original.value.coverArt || null)
})

const canResetAll = computed(() => {
    if (!original.value || !currentTrack.value) return false
    const origTitle = (original.value.title || '').trim()
    const origArtist = (original.value.artist || '').trim()
    const curTitle = titleInput.value.trim()
    const curArtist = artistInput.value.trim()
    return (
        curTitle !== origTitle ||
        curArtist !== origArtist ||
        !previewIsOriginal.value
    )
})

watch(isOpen, async (open) => {
    if (open && currentTrack.value) {
        titleInput.value = currentTrack.value.title || ''
        artistInput.value = currentTrack.value.artist || ''
        coverPreview.value = currentTrack.value.cover_art || null
        coverSourcePath.value = null
        coverReset.value = false
        errorText.value = ''
        original.value = null
        await loadOriginal()
    }
})

async function loadOriginal() {
    if (!currentTrack.value) return
    loadingOriginal.value = true
    try {
        const orig = await invoke<OriginalTrack>('get_original_track', {
            filePath: currentTrack.value.file_path,
        })
        original.value = orig
    } catch (e) {
        console.error('Не удалось загрузить оригинальные данные трека:', e)
        original.value = null
    } finally {
        loadingOriginal.value = false
    }
}

function resetAllToOriginal() {
    if (!original.value) return
    titleInput.value = original.value.title || ''
    artistInput.value = original.value.artist || ''
    coverSourcePath.value = null
    coverReset.value = original.value.hasOverrideCover
    coverPreview.value = original.value.coverArt || null
}

async function pickCover() {
    try {
        const selected = await openDialog({
            multiple: false,
            directory: false,
            filters: [
                { name: 'Изображения', extensions: ['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp'] },
            ],
        })
        if (!selected || typeof selected !== 'string') return
        coverSourcePath.value = selected
        coverReset.value = false
        try {
            coverPreview.value = await invoke<string>('read_image_as_data_url', { sourcePath: selected })
        } catch (err) {
            console.error('Не удалось прочитать обложку для превью:', err)
            coverPreview.value = currentTrack.value?.cover_art || null
            coverSourcePath.value = null
        }
    } catch (e) {
        console.error('Не удалось выбрать обложку:', e)
    }
}

async function resetCover() {
    if (!currentTrack.value) return
    coverSourcePath.value = null
    coverReset.value = true
    try {
        const original = await invoke<string | null>('get_original_cover_art', {
            filePath: currentTrack.value.file_path,
        })
        coverPreview.value = original ?? null
    } catch {
        coverPreview.value = null
    }
}

async function onSave() {
    if (!currentTrack.value) return
    if (saving.value) return
    saving.value = true
    errorText.value = ''
    try {
        const filePath = currentTrack.value.file_path
        const args: Record<string, unknown> = {
            filePath,
            title: titleInput.value,
            artist: artistInput.value,
        }
        if (coverSourcePath.value) {
            args.cover = { sourcePath: coverSourcePath.value }
        } else if (coverReset.value) {
            args.cover = { reset: true }
        }
        const updated = await invoke<TrackMetadata>('track_set_override', args)

        // Object.assign, not cacheTrack's merge, so a cover reset to null is applied.
        const cached = getTrackByPath(filePath)
        if (cached) Object.assign(cached, updated)
        else cacheTracks([updated])
        if (playerCurrentTrack.value?.file_path === filePath) {
            playerCurrentTrack.value = { ...playerCurrentTrack.value, ...updated }
        }
        await refreshLibrary()
        closeTrackEdit()
    } catch (e: any) {
        errorText.value = `Не удалось сохранить: ${e}`
    } finally {
        saving.value = false
    }
}

function onKeydown(e: KeyboardEvent) {
    if (!isOpen.value) return
    if (e.key === 'Escape') {
        e.preventDefault()
        closeTrackEdit()
    } else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault()
        if (hasChanges.value) onSave()
    }
}

onMounted(() => {
    document.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
    document.removeEventListener('keydown', onKeydown)
})
</script>

<template>
    <Teleport to="body">
        <Transition name="modal">
            <div
                v-if="isOpen && currentTrack"
                class="modalBackdrop"
                @mousedown="onBackdropMousedown"
            >
                <div ref="modalCardEl" class="modalCard" @mousedown.stop>
                    <div class="sheetHandle" @pointerdown="onSheetDragStart"></div>

                    <div class="fileName" v-tooltip="currentTrack.file_path">{{ fileName }}</div>

                    <div class="editArea">
                        <div class="coverWrap">
                            <button class="cover" @click="pickCover" aria-label="Выбрать обложку">
                                <img v-if="coverPreview" :src="coverPreview" alt="" draggable="false" />
                                <span v-else class="coverPlaceholder"><EmptyIcon /></span>
                                <span class="coverOverlay"><EditIcon width="22" height="22" /></span>
                            </button>
                            <button
                                v-if="coverPreview && !previewIsOriginal"
                                class="coverReset"
                                @click="resetCover"
                                aria-label="Сбросить обложку"
                                v-tooltip="'Сбросить обложку'"
                            >
                                <CloseIcon width="14" height="14" />
                            </button>
                        </div>

                        <div class="fieldsColumn">
                            <label class="fieldGroup">
                                <span class="fieldLabel">Название</span>
                                <textarea
                                    v-model="titleInput"
                                    v-autogrow
                                    rows="1"
                                    class="fieldInput"
                                    :placeholder="fileName"
                                    spellcheck="false"
                                    autocomplete="off"
                                    @keydown.enter="onFieldEnter"
                                ></textarea>
                            </label>
                            <label class="fieldGroup">
                                <span class="fieldLabel">Исполнитель</span>
                                <textarea
                                    v-model="artistInput"
                                    v-autogrow
                                    rows="1"
                                    class="fieldInput"
                                    placeholder="Неизвестный исполнитель"
                                    spellcheck="false"
                                    autocomplete="off"
                                    @keydown.enter="onFieldEnter"
                                ></textarea>
                            </label>
                        </div>
                    </div>

                    <div v-if="errorText" class="errorBox">{{ errorText }}</div>

                    <div class="footer">
                        <button
                            class="btn quiet"
                            :disabled="!canResetAll || saving || loadingOriginal"
                            @click="resetAllToOriginal"
                            v-tooltip="'Вернуть значения из тегов файла'"
                        >
                            Сбросить
                        </button>
                        <div class="footerSpacer"></div>
                        <button class="btn ghost" @click="closeTrackEdit" :disabled="saving">Отмена</button>
                        <button
                            class="btn primary"
                            :disabled="!hasChanges || saving"
                            @click="onSave"
                        >
                            {{ saving ? 'Сохранение...' : 'Сохранить' }}
                        </button>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.modalBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.4);
    backdrop-filter: blur(6px);
}
.modalCard {
    --tile: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    position: relative;
    width: 520px;
    max-width: calc(100vw - 40px);
    padding: 24px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.32);
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow: hidden;
}
.sheetHandle {
    display: none;
}

.fileName {
    align-self: flex-start;
    max-width: 100%;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.45);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}

.editArea {
    display: flex;
    flex-direction: row;
    gap: 20px;
    align-items: flex-start;
}
.coverWrap {
    position: relative;
    flex-shrink: 0;
    width: 148px;
    height: 148px;
}
.cover {
    position: relative;
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
    border: none;
    border-radius: 16px;
    overflow: hidden;
    background-color: var(--tile);
    box-shadow: 0 12px 28px rgba(0, 0, 0, 0.35);
    cursor: pointer;
}
.cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    user-select: none;
}
.coverPlaceholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.35);
}
.coverPlaceholder svg {
    width: 34px;
    height: 34px;
}
.coverOverlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: hsla(0, 0%, 0%, 0.45);
    opacity: 0;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.cover:hover .coverOverlay,
.cover:focus-visible .coverOverlay {
    opacity: 1;
}
.coverReset {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: hsla(0, 0%, 0%, 0.6);
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.coverReset:hover {
    background-color: hsla(0, 0%, 0%, 0.8);
}

.fieldsColumn {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
}
.fieldGroup {
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.fieldLabel {
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    padding-left: 2px;
    user-select: none;
}
.fieldInput {
    display: block;
    box-sizing: border-box;
    width: 100%;
    padding: 10px 14px;
    border: none;
    border-radius: 12px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.92);
    font-family: inherit;
    font-size: 14px;
    font-weight: 500;
    line-height: 1.35;
    resize: none;
    overflow-y: hidden;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    outline: none;
    caret-color: hsl(var(--accent-h), max(var(--accent-s), 55%), 72%);
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.fieldInput::placeholder {
    color: hsla(0, 0%, 100%, 0.3);
}
.fieldInput:hover {
    background-color: hsla(0, 0%, 100%, 0.07);
}
.fieldInput:focus {
    background-color: var(--tile);
}

.errorBox {
    padding: 10px 14px;
    border-radius: 12px;
    background-color: hsla(0, 70%, 50%, 0.12);
    color: hsl(0, 70%, 80%);
    font-size: 12px;
    font-weight: 500;
}

.footer {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
}
.footerSpacer {
    flex: 1;
}
.btn {
    cursor: pointer;
    height: 36px;
    padding: 0 18px;
    border: none;
    border-radius: 999px;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth), opacity var(--duration-hover) var(--ease-smooth);
}
.btn:disabled {
    cursor: default;
    opacity: 0.4;
}
.btn.quiet {
    padding: 0 12px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.55);
}
.btn.quiet:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.btn.ghost {
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.8);
}
.btn.ghost:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.11);
    color: #fff;
}
.btn.primary {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 60%), 62%);
    color: hsl(var(--accent-h), 40%, 10%);
}
.btn.primary:hover:not(:disabled) {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 60%), 68%);
}

.modal-enter-active, .modal-leave-active {
    transition: opacity var(--duration-modal) var(--ease-smooth);
}
.modal-enter-active .modalCard, .modal-leave-active .modalCard {
    transition: transform var(--duration-modal) var(--ease-smooth), opacity var(--duration-modal) var(--ease-smooth);
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .modalCard, .modal-leave-to .modalCard {
    transform: translateY(8px) scale(.98);
    opacity: 0;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .modalBackdrop {
        align-items: flex-end !important;
        justify-content: center !important;
        padding: calc(var(--safe-top) + 12px) 0 0 !important;
        background-color: hsla(0, 0%, 0%, 0.36);
    }

    .modalCard {
        width: 100vw !important;
        max-width: none !important;
        height: calc(100dvh - var(--safe-top) - 12px) !important;
        max-height: none !important;
        margin-left: 0 !important;
        margin-right: 0 !important;
        padding: 0 16px calc(16px + var(--safe-bottom));
        gap: 20px;
        border-radius: 28px 28px 0 0 !important;
        background-color: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%);
        overflow-y: auto !important;
        overscroll-behavior: contain;
        touch-action: pan-y;
        transition: transform var(--duration-hover) var(--ease-smooth);
        will-change: transform;
    }

    .sheetHandle {
        display: flex;
        flex: 0 0 auto;
        height: 32px;
        margin: 0 -16px -12px;
        align-items: center;
        justify-content: center;
        cursor: grab;
        touch-action: none;
    }

    .sheetHandle::before {
        content: "";
        width: 40px;
        height: 4px;
        border-radius: 999px;
        background-color: hsla(0, 0%, 100%, 0.28);
    }

    .fileName {
        align-self: center;
    }

    .editArea {
        flex-direction: column;
        align-items: stretch;
        gap: 24px;
    }

    .coverWrap {
        align-self: center;
        width: min(56vw, 200px);
        height: min(56vw, 200px);
    }

    .coverOverlay {
        display: none;
    }

    .coverReset {
        width: 36px;
        height: 36px;
    }

    .fieldInput {
        min-height: 44px;
        font-size: 16px;
    }

    .footer {
        margin-top: auto;
    }

    .btn {
        height: 44px;
    }

    .btn.ghost,
    .btn.primary {
        flex: 1;
    }

    .footerSpacer {
        display: none;
    }

    .modal-enter-active .modalCard {
        transition: transform var(--duration-modal) var(--ease-smooth);
    }

    .modal-leave-active,
    .modal-leave-active .modalCard {
        transition: transform .5s var(--ease-sheet), opacity .5s var(--ease-sheet);
    }

    .modal-enter-from .modalCard,
    .modal-leave-to .modalCard {
        transform: translateY(100%);
        opacity: 1;
    }

    :global(body.lfmusic-phone) .modalBackdrop {
        padding: calc(var(--safe-top) + 12px) 0 0 !important;
    }

    :global(body.lfmusic-phone) .modalCard {
        width: 100vw !important;
        max-width: none !important;
        margin-left: 0 !important;
        margin-right: 0 !important;
        border-radius: 28px 28px 0 0 !important;
    }
}
</style>
