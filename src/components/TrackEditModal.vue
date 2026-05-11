<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'

import CloseIcon from './Icons/CloseIcon.vue'
import EditIcon from './Icons/EditIcon.vue'
import { useTrackEditModal } from '../composables/useTrackEditModal'
import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import { isPhoneViewport } from '../composables/usePlatform'

const { isOpen, currentTrack, closeTrackEdit } = useTrackEditModal()
const { refreshLibrary, trackList, currentTrack: playerCurrentTrack } = usePlayer()

interface OriginalTrack {
    title: string | null
    artist: string | null
    coverArt: string | null
    hasOverrideCover: boolean
}

const titleInput = ref('')
const artistInput = ref('')
// Локальный preview обложки: либо data URL текущей, либо src нового выбранного файла,
// либо null если обложку решили сбросить.
const coverPreview = ref<string | null>(null)
const coverSourcePath = ref<string | null>(null)
const coverReset = ref(false)

// Оригинальные значения трека (как они в тегах файла, без переопределений).
// Используется кнопкой «Сбросить» в шапке модалки и для определения,
// является ли текущая обложка пользовательской.
const original = ref<OriginalTrack | null>(null)
const loadingOriginal = ref(false)

const saving = ref(false)
const errorText = ref('')
const modalCardEl = ref<HTMLElement | null>(null)
let sheetDragStartY = 0
let sheetDragging = false

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

// Превью обложки совпадает с оригиналом из тегов файла → нет смысла
// показывать «крестик сброса». Считаем сравнением data URL (для override
// и оригинала это либо одинаковая base64-строка, либо обе пусты).
const previewIsOriginal = computed(() => {
    if (coverSourcePath.value) return false
    if (!original.value) return false
    return (coverPreview.value || null) === (original.value.coverArt || null)
})

// Есть ли что сбрасывать в принципе — переопределения по любому из полей.
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
        // Загружаем оригинал параллельно — нужен и для кнопки «Сбросить»,
        // и чтобы решить, показывать ли крестик на обложке.
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
    // «Сбросить» в модалке не сохраняет — только обновляет форму.
    // Чтобы при последующем Save обложка тоже вернулась к оригинальной,
    // помечаем coverReset = true (если у трека есть override-обложка).
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
    // Показываем оригинальную обложку из тегов файла; если её нет —
    // только тогда превью становится «без обложки».
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

        // Обновляем in-memory список треков, чтобы UI среагировал моментально
        const idx = trackList.value.findIndex(t => t.file_path === filePath)
        if (idx !== -1) {
            trackList.value[idx] = { ...trackList.value[idx], ...updated }
        }
        if (playerCurrentTrack.value?.file_path === filePath) {
            playerCurrentTrack.value = { ...playerCurrentTrack.value, ...updated }
        }
        // Полная перезагрузка — чтобы плейлисты исполнителей подхватили
        // переименованного артиста и т.п.
        await refreshLibrary()
        closeTrackEdit()
    } catch (e: any) {
        errorText.value = `Не удалось сохранить: ${e}`
    } finally {
        saving.value = false
    }
}

function onBackdropMousedown(e: MouseEvent) {
    if (isPhoneViewport()) return
    if (e.target === e.currentTarget) closeTrackEdit()
}

function onSheetDragStart(e: PointerEvent) {
    if (!isPhoneViewport()) return
    e.preventDefault()
    sheetDragging = true
    sheetDragStartY = e.clientY
    if (modalCardEl.value) modalCardEl.value.style.transition = 'none'
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
    window.addEventListener('pointermove', onSheetDragMove)
    window.addEventListener('pointerup', onSheetDragEnd)
    window.addEventListener('pointercancel', onSheetDragEnd)
}

function onSheetDragMove(e: PointerEvent) {
    if (!sheetDragging || !modalCardEl.value) return
    e.preventDefault()
    const delta = Math.max(0, e.clientY - sheetDragStartY)
    modalCardEl.value.style.transform = `translateY(${delta}px)`
}

function onSheetDragEnd(e: PointerEvent) {
    if (!sheetDragging) return
    sheetDragging = false
    window.removeEventListener('pointermove', onSheetDragMove)
    window.removeEventListener('pointerup', onSheetDragEnd)
    window.removeEventListener('pointercancel', onSheetDragEnd)
    const delta = Math.max(0, e.clientY - sheetDragStartY)
    if (delta > 76) {
        closeTrackEdit()
        return
    }
    if (modalCardEl.value) {
        modalCardEl.value.style.transition = ''
        modalCardEl.value.style.transform = ''
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
    window.removeEventListener('pointermove', onSheetDragMove)
    window.removeEventListener('pointerup', onSheetDragEnd)
    window.removeEventListener('pointercancel', onSheetDragEnd)
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
                    <div class="accentGlow"></div>

                    <button class="closeBtn" @click="closeTrackEdit" aria-label="Закрыть">
                        <CloseIcon />
                    </button>

                    <div class="modalHeader">
                        <div class="overline">Редактирование трека</div>
                        <div class="fileName" v-tooltip="currentTrack.file_path">{{ fileName }}</div>
                    </div>

                    <div class="editArea">
                        <div class="coverColumn">
                            <div class="coverWrap">
                                <div class="coverHalo"></div>
                                <div class="cover" :class="{ empty: !coverPreview }">
                                    <img v-if="coverPreview" :src="coverPreview" alt="" draggable="false" />
                                    <div v-else class="coverPlaceholder">Без обложки</div>
                                </div>
                                <div class="coverActions">
                                    <button class="coverActionBtn" @click="pickCover" v-tooltip="'Выбрать изображение'">
                                        <EditIcon width="16" height="16" />
                                    </button>
                                    <button
                                        v-if="coverPreview && !previewIsOriginal"
                                        class="coverActionBtn coverReset"
                                        @click="resetCover"
                                        v-tooltip="'Сбросить обложку'"
                                    >
                                        <CloseIcon width="16" height="16" />
                                    </button>
                                </div>
                            </div>
                            <button class="coverPickBtn" @click="pickCover">
                                <EditIcon width="14" height="14" />
                                <span>{{ coverSourcePath ? 'Заменить файл' : 'Выбрать обложку' }}</span>
                            </button>
                        </div>

                        <div class="fieldsColumn">
                            <label class="fieldGroup">
                                <span class="fieldLabel">Название</span>
                                <input
                                    v-model="titleInput"
                                    type="text"
                                    class="fieldInput"
                                    :placeholder="fileName"
                                    spellcheck="false"
                                    autocomplete="off"
                                />
                            </label>
                            <label class="fieldGroup">
                                <span class="fieldLabel">Исполнитель</span>
                                <input
                                    v-model="artistInput"
                                    type="text"
                                    class="fieldInput"
                                    placeholder="Неизвестный исполнитель"
                                    spellcheck="false"
                                    autocomplete="off"
                                />
                            </label>
                            <div class="hint">
                                Изменения сохраняются и переживают пересканирование библиотеки.
                                Сбросятся, только если папка с этим треком будет удалена из приложения.
                            </div>
                        </div>
                    </div>

                    <div v-if="errorText" class="errorBox">{{ errorText }}</div>

                    <div class="footer">
                        <button
                            class="btn ghost resetBtn"
                            :disabled="!canResetAll || saving || loadingOriginal"
                            @click="resetAllToOriginal"
                            v-tooltip="'Подставить оригинальные значения из тегов файла'"
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
                            {{ saving ? 'Сохранение…' : 'Сохранить' }}
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
    width: calc(100vw / var(--ui-scale, 1));
    height: calc(100vh / var(--ui-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.35);
    backdrop-filter: blur(6px);
}
.modalCard {
    position: relative;
    width: 540px;
    max-width: calc(100vw - 40px);
    padding: 22px 22px 18px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow:
        0 24px 60px rgba(0, 0, 0, 0.25),
        0 0 0 1px hsla(0, 0%, 100%, 0.04) inset;
    display: flex;
    flex-direction: column;
    gap: 18px;
    overflow: hidden;
}
.sheetHandle {
    display: none;
}
.accentGlow {
    pointer-events: none;
    position: absolute;
    top: -60%;
    left: -20%;
    right: -20%;
    height: 220px;
    background: radial-gradient(
        ellipse at center,
        hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.28) 0%,
        hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0) 70%
    );
    filter: blur(8px);
    z-index: 0;
}
.closeBtn {
    position: absolute;
    top: 16px;
    right: 16px;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: none;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: .24s ease;
    z-index: 2;
}
.closeBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.14);
    color: #fff;
}

.modalHeader {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-right: 36px;
}
.overline {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.7), 75%, 0.85);
    user-select: none;
}
.fileName {
    font-family: "JetBrains Mono", "Fira Code", Consolas, monospace;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.6);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}

.editArea {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: row;
    gap: 22px;
    align-items: stretch;
}
.coverColumn {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
}
.coverWrap {
    position: relative;
    width: 156px;
    height: 156px;
}
.coverHalo {
    position: absolute;
    inset: -10px;
    border-radius: 22px;
    background: hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.45);
    filter: blur(20px);
    opacity: 0.55;
    z-index: 0;
}
.cover {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: 16px;
    overflow: hidden;
    background-color: hsla(0, 0%, 100%, 0.04);
    box-shadow:
        0 8px 24px rgba(0, 0, 0, 0.35),
        0 0 0 1px hsla(0, 0%, 100%, 0.06) inset;
    z-index: 1;
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
    color: hsla(0, 0%, 100%, 0.4);
    font-size: 12px;
    font-weight: 600;
    background: linear-gradient(135deg, hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.35), hsla(0, 0%, 100%, 0.03));
    user-select: none;
}
.coverActions {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    opacity: 0;
    transition: opacity .22s ease;
    background-color: hsla(0, 0%, 0%, 0.35);
    border-radius: 16px;
}
.coverWrap:hover .coverActions {
    opacity: 1;
}
.coverActionBtn {
    cursor: pointer;
    padding: 9px;
    border-radius: 999px;
    color: #fff;
    background-color: hsla(0, 0%, 0%, 0.55);
    backdrop-filter: blur(8px);
    border: none;
    transition: background-color .2s ease, transform .15s ease, color .2s ease;
}
.coverActionBtn:hover {
    background-color: hsla(0, 0%, 0%, 0.78);
    transform: scale(1.05);
}
.coverActionBtn.coverReset:hover {
    color: hsl(0, 70%, 75%);
}
.coverPickBtn {
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.85);
    font-size: 12px;
    font-weight: 600;
    transition: background-color .2s ease, color .2s ease;
}
.coverPickBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.18);
    color: #fff;
}

.fieldsColumn {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    justify-content: flex-start;
}
.fieldGroup {
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.fieldLabel {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.45);
    padding-left: 2px;
    user-select: none;
}
.fieldInput {
    width: 100%;
    padding: 10px 14px;
    border-radius: 12px;
    background-color: hsla(0, 0%, 100%, 0.04);
    color: hsla(0, 0%, 100%, 0.92);
    font-size: 14px;
    font-weight: 500;
    border: 1px solid hsla(0, 0%, 100%, 0.04);
    transition: background-color .18s ease, border-color .18s ease;
}
.fieldInput::placeholder {
    color: hsla(0, 0%, 100%, 0.3);
    font-weight: 500;
}
.fieldInput:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}
.fieldInput:focus {
    background-color: hsla(0, 0%, 100%, 0.07);
    border-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.45);
}
.hint {
    font-size: 11px;
    line-height: 1.45;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
}

.errorBox {
    position: relative;
    z-index: 1;
    padding: 10px 14px;
    border-radius: 12px;
    background-color: hsla(0, 70%, 50%, 0.12);
    color: hsl(0, 70%, 80%);
    font-size: 12px;
    font-weight: 500;
}

.footer {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding-top: 4px;
}
.footerSpacer {
    flex: 1;
}
.btn.resetBtn {
    color: hsla(0, 0%, 100%, 0.55);
}
.btn.resetBtn:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.btn {
    cursor: pointer;
    padding: 9px 18px;
    border-radius: 999px;
    font-family: inherit;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.4px;
    text-transform: uppercase;
    transition: .22s ease;
    user-select: none;
    border: none;
}
.btn:disabled {
    cursor: default;
    opacity: 0.5;
}
.btn.ghost {
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.7);
}
.btn.ghost:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: #fff;
}
.btn.primary {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.22);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 88%, 1);
    box-shadow: 0 0 0 1px hsla(var(--accent-h), var(--accent-s), 60%, 0.25) inset;
}
.btn.primary:hover:not(:disabled) {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.34);
    color: #fff;
}

.modal-enter-active, .modal-leave-active {
    transition: opacity .22s ease;
}
.modal-enter-active .modalCard, .modal-leave-active .modalCard {
    transition: transform .26s cubic-bezier(0.22, 1.2, 0.36, 1), opacity .22s ease;
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .modalCard, .modal-leave-to .modalCard {
    transform: scale(0.94) translateY(8px);
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
        padding: 0 16px calc(18px + var(--safe-bottom));
        gap: 16px;
        border-radius: 28px 28px 0 0 !important;
        background-color: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%);
        overflow-y: auto !important;
        overscroll-behavior: contain;
        touch-action: pan-y;
        transition: transform .2s ease;
        will-change: transform;
    }

    .sheetHandle {
        display: flex;
        flex: 0 0 auto;
        height: 34px;
        margin: 0 -16px;
        align-items: center;
        justify-content: center;
        cursor: grab;
        touch-action: none;
    }

    .sheetHandle::before {
        content: "";
        width: 42px;
        height: 4px;
        border-radius: 999px;
        background-color: hsla(0, 0%, 100%, 0.28);
    }

    .modalHeader {
        padding-right: 0;
    }

    .closeBtn {
        display: none;
    }

    .coverColumn {
        align-items: stretch;
    }

    .coverWrap {
        align-self: center;
        width: min(52vw, 170px);
        height: min(52vw, 170px);
    }

    .coverActions {
        opacity: 1;
    }

    .coverActionBtn {
        width: 44px;
        height: 44px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
    }

    .coverPickBtn {
        min-height: 44px;
        justify-content: center;
        border-radius: 14px;
    }

    .fieldInput {
        min-height: 44px;
        font-size: 16px;
    }

    .hint {
        font-size: 12px;
    }

    .footerSpacer {
        display: none;
    }

    .modal-enter-active .modalCard,
    .modal-leave-active .modalCard {
        transition: transform .2s ease, opacity .2s ease;
    }

    .modal-enter-from .modalCard,
    .modal-leave-to .modalCard {
        transform: translateY(20px);
        opacity: 0;
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
