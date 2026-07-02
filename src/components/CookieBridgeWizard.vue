<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useDownloads } from '../composables/useDownloads';
import { useI18n } from '../composables/useI18n';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const { prepareBrowserExtension } = useDownloads();

const STEP_COUNT = 4;
const currentStep = ref(0);
// 1 = forward, -1 = back — picks the slide direction of step transitions.
const direction = ref<1 | -1>(1);
const folderBusy = ref(false);
const folderError = ref<string | null>(null);
const bodyEl = ref<HTMLElement | null>(null);
const stepPaneEl = ref<HTMLElement | null>(null);

// Placeholder until the real step-by-step recordings are dropped in - one
// looped webm clip per step, shown above the description. `null` renders a
// quiet placeholder frame instead of a broken <video>.
const stepVideos: (string | null)[] = [null, null, null, null];

const steps = computed(() => [
    { title: t('downloads.cookies.wizard.step1Title'), desc: t('downloads.cookies.wizard.step1Desc') },
    { title: t('downloads.cookies.wizard.step2Title'), desc: t('downloads.cookies.wizard.step2Desc') },
    { title: t('downloads.cookies.wizard.step3Title'), desc: t('downloads.cookies.wizard.step3Desc') },
    { title: t('downloads.cookies.wizard.step4Title'), desc: t('downloads.cookies.wizard.step4Desc') },
]);

const isFirstStep = computed(() => currentStep.value === 0);
const isLastStep = computed(() => currentStep.value === STEP_COUNT - 1);
const isFolderStep = computed(() => currentStep.value === 0);
const stepTransition = computed(() => (direction.value > 0 ? 'stepFwd' : 'stepBack'));

// Steps have different text length (and only step 1 has the folder button),
// so the body's natural height differs per step. The body always carries an
// explicit pixel height (CSS-transitioned) kept in sync with the active pane
// by a ResizeObserver — the leaving pane is absolutely positioned during the
// step transition, so the entering pane alone defines the target height, and
// late content changes (an error appearing) animate too instead of snapping.
let paneObserver: ResizeObserver | null = null;

function syncBodyHeight() {
    const el = bodyEl.value;
    const pane = stepPaneEl.value;
    if (!el || !pane) return;
    el.style.height = `${pane.offsetHeight}px`;
}

watch(stepPaneEl, (pane, old) => {
    if (!paneObserver) return;
    if (old) paneObserver.unobserve(old);
    if (pane) {
        paneObserver.observe(pane);
        syncBodyHeight();
    }
}, { flush: 'post' });

watch(() => props.open, async (isOpen) => {
    if (!isOpen) return;
    direction.value = 1;
    currentStep.value = 0;
    folderError.value = null;
    // First height set happens from `auto`, which the browser can't
    // interpolate — it applies instantly, so opening never animates height.
    await nextTick();
    syncBodyHeight();
});

function goToStep(i: number) {
    if (i === currentStep.value || i < 0 || i >= STEP_COUNT) return;
    direction.value = i > currentStep.value ? 1 : -1;
    currentStep.value = i;
}
function prevStep() {
    goToStep(currentStep.value - 1);
}
function nextStep() {
    goToStep(currentStep.value + 1);
}
function close() {
    emit('close');
}

async function openExtensionFolder() {
    if (folderBusy.value) return;
    folderBusy.value = true;
    folderError.value = null;
    try {
        const path = await prepareBrowserExtension();
        await invoke('open_folder_in_explorer', { path });
    } catch (e) {
        folderError.value = String(e);
    } finally {
        folderBusy.value = false;
    }
}

function onBackdropMousedown(e: MouseEvent) {
    if (e.target === e.currentTarget) close();
}

function onKeydown(e: KeyboardEvent) {
    if (!props.open) return;
    if (e.key === 'Escape') {
        e.preventDefault();
        close();
    } else if (e.key === 'ArrowRight') {
        nextStep();
    } else if (e.key === 'ArrowLeft') {
        prevStep();
    }
}

onMounted(() => {
    document.addEventListener('keydown', onKeydown);
    paneObserver = new ResizeObserver(() => syncBodyHeight());
    if (stepPaneEl.value) {
        paneObserver.observe(stepPaneEl.value);
        syncBodyHeight();
    }
});
onUnmounted(() => {
    document.removeEventListener('keydown', onKeydown);
    paneObserver?.disconnect();
    paneObserver = null;
});
</script>

<template>
    <Teleport to="body">
        <Transition name="modal">
            <div v-if="open" class="modalBackdrop" @mousedown="onBackdropMousedown">
                <div class="wizardCard" role="dialog" aria-modal="true" @mousedown.stop>
                    <button class="wizardClose" type="button" @click="close" aria-label="Close">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
                            <path d="M6 6l12 12M18 6L6 18" />
                        </svg>
                    </button>

                    <div class="wizardVisual">
                        <Transition :name="stepTransition">
                            <div class="visualPane" :key="currentStep">
                                <video
                                    v-if="stepVideos[currentStep]"
                                    class="wizardVideoEl"
                                    :src="stepVideos[currentStep]!"
                                    loop
                                    muted
                                    autoplay
                                    playsinline
                                />
                                <div v-else class="visualPlaceholder"></div>
                            </div>
                        </Transition>
                        <span class="wizardStepBadge">{{ currentStep + 1 }} / {{ STEP_COUNT }}</span>
                    </div>

                    <div class="wizardBody" ref="bodyEl">
                        <Transition :name="stepTransition">
                            <div class="stepPane" :key="currentStep" ref="stepPaneEl">
                                <h3 class="wizardStepTitle">{{ steps[currentStep].title }}</h3>
                                <p class="wizardStepDesc">{{ steps[currentStep].desc }}</p>
                                <Transition name="noteSlide">
                                    <p v-if="isFolderStep && folderError" class="wizardError">{{ folderError }}</p>
                                </Transition>
                                <button
                                    v-if="isFolderStep"
                                    class="wizardFolderBtn"
                                    :class="{ loading: folderBusy }"
                                    type="button"
                                    :disabled="folderBusy"
                                    @click="openExtensionFolder"
                                >
                                    <span class="btnLabel">{{ t('downloads.cookies.openFolder') }}</span>
                                    <span class="miniSpinner btnSpinner" aria-hidden="true"></span>
                                </button>
                            </div>
                        </Transition>
                    </div>

                    <div class="wizardNav">
                        <div class="wizardDots">
                            <button
                                v-for="i in STEP_COUNT"
                                :key="i"
                                type="button"
                                class="wizardDot"
                                :class="{ active: i - 1 === currentStep, done: i - 1 < currentStep }"
                                :aria-label="`${i} / ${STEP_COUNT}`"
                                @click="goToStep(i - 1)"
                            ></button>
                        </div>
                        <div class="wizardNavBtns">
                            <button
                                class="wizardNavBtn"
                                :class="{ hiddenBtn: isFirstStep }"
                                :disabled="isFirstStep"
                                type="button"
                                @click="prevStep"
                            >{{ t('downloads.cookies.wizard.back') }}</button>
                            <button
                                class="wizardNavBtn primary"
                                type="button"
                                @click="isLastStep ? close() : nextStep()"
                            >{{ isLastStep ? t('downloads.cookies.wizard.done') : t('downloads.cookies.wizard.next') }}</button>
                        </div>
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
    background-color: hsla(0, 0%, 0%, 0.55);
    backdrop-filter: blur(4px);
}
.wizardCard {
    position: relative;
    width: 540px;
    max-width: calc(100vw - 42px);
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
    overflow: hidden;
}
.wizardClose {
    position: absolute;
    top: 14px;
    right: 14px;
    width: 30px;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 50%;
    background: hsla(0, 0%, 0%, 0.35);
    color: hsla(0, 0%, 100%, 0.85);
    cursor: pointer;
    z-index: 2;
    transition: background-color .2s ease, transform .2s ease;
}
.wizardClose:hover {
    background: hsla(0, 0%, 0%, 0.55);
}
.wizardClose:active {
    transform: scale(0.92);
}

/* Visual area: a looped webm clip per step (crossfades on step change), or a
   quiet placeholder frame until the recordings are dropped in. */
.wizardVisual {
    position: relative;
    aspect-ratio: 16 / 10;
    width: 100%;
    background: hsla(0, 0%, 0%, 0.3);
    overflow: hidden;
}
.visualPane {
    position: absolute;
    inset: 0;
}
.wizardVideoEl {
    width: 100%;
    height: 100%;
    object-fit: cover;
}
.visualPlaceholder {
    width: 100%;
    height: 100%;
    background: linear-gradient(155deg, hsla(var(--accent-h), var(--accent-s), 22%, 0.5), hsla(0, 0%, 0%, 0.25));
}
.wizardStepBadge {
    position: absolute;
    top: 14px;
    left: 16px;
    padding: 4px 11px;
    border-radius: 999px;
    background: hsla(0, 0%, 0%, 0.35);
    font-size: 12px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
    color: hsla(0, 0%, 100%, 0.75);
    user-select: none;
}

.wizardBody {
    position: relative;
    overflow: hidden;
    transition: height .3s cubic-bezier(0.22, 1, 0.36, 1);
}
.stepPane {
    padding: 22px 28px 8px;
}
.wizardStepTitle {
    margin: 0;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 19px;
    color: hsla(0, 0%, 100%, 0.95);
}
.wizardStepDesc {
    margin: 9px 0 0;
    font-size: 14.5px;
    line-height: 1.5;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.6);
}
.wizardError {
    margin: 10px 0 0;
    font-size: 12.5px;
    font-weight: 600;
    line-height: 1.4;
    color: hsla(38, 85%, 72%, 0.95);
}
.noteSlide-enter-active {
    transition: opacity .22s ease, transform .22s cubic-bezier(0.22, 1, 0.36, 1);
}
.noteSlide-leave-active {
    transition: opacity .14s ease;
}
.noteSlide-enter-from {
    opacity: 0;
    transform: translateY(-4px);
}
.noteSlide-leave-to {
    opacity: 0;
}
.wizardFolderBtn {
    position: relative;
    margin-top: 16px;
    padding: 10px 20px;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.95);
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    font-family: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background-color .2s ease, transform .2s ease, opacity .2s ease;
}
.wizardFolderBtn:disabled {
    cursor: default;
    opacity: 0.6;
}
.wizardFolderBtn:not(:disabled):hover {
    background-color: #fff;
    transform: translateY(-1px);
}
.wizardFolderBtn:not(:disabled):active {
    transform: translateY(0) scale(0.97);
}
/* Spinner over the label (label fades, width stays) — same loading pattern
   as the settings-page action buttons. Centered via negative margins, not
   translate — the spin animation owns the transform property. */
.wizardFolderBtn .btnLabel {
    opacity: 1;
    transition: opacity .18s ease;
}
.wizardFolderBtn.loading .btnLabel {
    opacity: 0;
}
.wizardFolderBtn .btnSpinner {
    position: absolute;
    top: 50%;
    left: 50%;
    margin: -6.5px 0 0 -6.5px;
    opacity: 0;
    pointer-events: none;
    transition: opacity .18s ease;
}
.wizardFolderBtn.loading .btnSpinner {
    opacity: 1;
}
.miniSpinner {
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 2px solid hsla(var(--accent-h), var(--accent-s), 30%, 0.25);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 25%, 0.9);
    animation: spin 0.8s linear infinite;
}
@keyframes spin {
    to { transform: rotate(360deg); }
}

/* Directional step transitions: forward slides left, back slides right.
   The leaving pane is pulled out of flow so the body height tracks only
   the entering pane. */
.stepFwd-enter-active,
.stepBack-enter-active {
    transition: opacity .3s ease, transform .3s cubic-bezier(0.22, 1, 0.36, 1);
}
.stepFwd-leave-active,
.stepBack-leave-active {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    transition: opacity .18s ease, transform .18s ease;
}
.stepFwd-enter-from { opacity: 0; transform: translateX(26px); }
.stepFwd-leave-to { opacity: 0; transform: translateX(-20px); }
.stepBack-enter-from { opacity: 0; transform: translateX(-26px); }
.stepBack-leave-to { opacity: 0; transform: translateX(20px); }

.wizardNav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 20px 28px 24px;
}
.wizardDots {
    display: flex;
    align-items: center;
    gap: 6px;
}
/* Clickable step dots; the active one stretches into a small pill. */
.wizardDot {
    width: 6px;
    height: 6px;
    padding: 0;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.2);
    cursor: pointer;
    transition: width .26s cubic-bezier(0.22, 1, 0.36, 1), background-color .2s ease;
}
.wizardDot:hover {
    background-color: hsla(0, 0%, 100%, 0.45);
}
.wizardDot.done {
    background-color: hsla(0, 0%, 100%, 0.38);
}
.wizardDot.active {
    width: 20px;
    background-color: hsla(0, 0%, 100%, 0.85);
}
.wizardNavBtns {
    display: flex;
    gap: 8px;
}
.wizardNavBtn {
    padding: 9px 18px;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-family: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background-color .2s ease, color .2s ease, opacity .2s ease, transform .2s ease;
}
.wizardNavBtn:not(:disabled):hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: #fff;
}
.wizardNavBtn:not(:disabled):active {
    transform: scale(0.96);
}
.wizardNavBtn.primary {
    background-color: hsla(0, 0%, 100%, 0.95);
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
}
/* Same specificity weight as the generic hover rule above, declared later —
   otherwise the generic rule paints white text on the white primary bg. */
.wizardNavBtn.primary:not(:disabled):hover {
    background-color: #fff;
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
}
/* "Back" on the first step: invisible but keeps its slot, so "Next" never
   jumps sideways when it appears. */
.wizardNavBtn.hiddenBtn {
    opacity: 0;
    pointer-events: none;
}

.modal-enter-active {
    transition: opacity .22s ease;
}
.modal-leave-active {
    transition: opacity .18s ease;
}
.modal-enter-active .wizardCard {
    transition: transform .28s cubic-bezier(0.22, 1, 0.36, 1), opacity .22s ease;
}
.modal-leave-active .wizardCard {
    transition: transform .18s ease, opacity .18s ease;
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .wizardCard, .modal-leave-to .wizardCard {
    transform: scale(0.95) translateY(10px);
    opacity: 0;
}
</style>
