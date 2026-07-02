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
const folderBusy = ref(false);
const folderError = ref<string | null>(null);
const bodyEl = ref<HTMLElement | null>(null);

// Placeholder until the real step-by-step recordings are dropped in - one
// looped clip per step, shown above the description. `null` renders a plain
// placeholder frame instead of a broken <video>.
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

// Steps have different text length (and only step 1 has the folder button),
// so the body's natural height differs per step. Animate between them
// (measure-before/after, like a mini FLIP) instead of letting the card snap
// to a new height. Suppressed on the reset-to-step-0 that happens when the
// modal opens, since there's no visible "before" state to animate from.
let skipNextAnimation = false;

async function animateBodyHeight() {
    const el = bodyEl.value;
    if (!el) return;
    const startHeight = el.getBoundingClientRect().height;
    el.style.height = `${startHeight}px`;
    await nextTick();
    const endHeight = el.scrollHeight;
    requestAnimationFrame(() => {
        el.style.height = `${endHeight}px`;
    });
}

function onBodyTransitionEnd(e: TransitionEvent) {
    if (e.propertyName === 'height' && bodyEl.value) {
        bodyEl.value.style.height = '';
    }
}

watch(currentStep, () => {
    if (skipNextAnimation) {
        skipNextAnimation = false;
        return;
    }
    void animateBodyHeight();
});

watch(() => props.open, async (isOpen) => {
    if (isOpen) {
        skipNextAnimation = true;
        currentStep.value = 0;
        folderError.value = null;
        await nextTick();
        skipNextAnimation = false;
    }
});

function prevStep() {
    if (!isFirstStep.value) currentStep.value -= 1;
}
function nextStep() {
    if (!isLastStep.value) currentStep.value += 1;
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

onMounted(() => document.addEventListener('keydown', onKeydown));
onUnmounted(() => document.removeEventListener('keydown', onKeydown));
</script>

<template>
    <Teleport to="body">
        <Transition name="modal">
            <div v-if="open" class="modalBackdrop" @mousedown="onBackdropMousedown">
                <div class="wizardCard" @mousedown.stop>
                    <button class="wizardClose" type="button" @click="close" aria-label="Close">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
                            <path d="M6 6l12 12M18 6L6 18" />
                        </svg>
                    </button>

                    <div class="wizardVideo">
                        <video
                            v-if="stepVideos[currentStep]"
                            :key="currentStep"
                            class="wizardVideoEl"
                            :src="stepVideos[currentStep]!"
                            loop
                            muted
                            autoplay
                            playsinline
                        />
                        <div v-else class="wizardVideoPlaceholder">
                            <span class="wizardStepBadge">{{ currentStep + 1 }}/{{ STEP_COUNT }}</span>
                        </div>
                    </div>

                    <div class="wizardBody" ref="bodyEl" @transitionend="onBodyTransitionEnd">
                        <h3 class="wizardStepTitle">{{ steps[currentStep].title }}</h3>
                        <p class="wizardStepDesc">{{ steps[currentStep].desc }}</p>
                        <p v-if="folderError" class="wizardError">{{ folderError }}</p>

                        <button
                            v-if="isFolderStep"
                            class="wizardFolderBtn"
                            type="button"
                            :disabled="folderBusy"
                            @click="openExtensionFolder"
                        >{{ t('downloads.cookies.openFolder') }}</button>
                    </div>

                    <div class="wizardNav">
                        <div class="wizardDots">
                            <span
                                v-for="i in STEP_COUNT"
                                :key="i"
                                class="wizardDot"
                                :class="{ active: i - 1 === currentStep }"
                            ></span>
                        </div>
                        <div class="wizardNavBtns">
                            <button
                                v-if="!isFirstStep"
                                class="wizardNavBtn"
                                type="button"
                                @click="prevStep"
                            >{{ t('downloads.cookies.wizard.back') }}</button>
                            <button
                                v-if="!isLastStep"
                                class="wizardNavBtn primary"
                                type="button"
                                @click="nextStep"
                            >{{ t('downloads.cookies.wizard.next') }}</button>
                            <button
                                v-else
                                class="wizardNavBtn primary"
                                type="button"
                                @click="close"
                            >{{ t('downloads.cookies.wizard.done') }}</button>
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
    width: 440px;
    max-width: calc(100vw - 42px);
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
    overflow: hidden;
}
.wizardClose {
    position: absolute;
    top: 12px;
    right: 12px;
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 50%;
    background: hsla(0, 0%, 0%, 0.35);
    color: hsla(0, 0%, 100%, 0.85);
    cursor: pointer;
    z-index: 1;
    transition: background-color .2s ease;
}
.wizardClose:hover {
    background: hsla(0, 0%, 0%, 0.55);
}

.wizardVideo {
    aspect-ratio: 16 / 10;
    width: 100%;
    background: hsla(0, 0%, 0%, 0.3);
}
.wizardVideoEl {
    width: 100%;
    height: 100%;
    object-fit: cover;
}
.wizardVideoPlaceholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: linear-gradient(155deg, hsla(var(--accent-h), var(--accent-s), 22%, 0.5), hsla(0, 0%, 0%, 0.25));
}
.wizardStepBadge {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 15px;
    font-weight: 800;
    letter-spacing: 0.02em;
    color: hsla(0, 0%, 100%, 0.55);
}

.wizardBody {
    padding: 20px 24px 4px;
    overflow: hidden;
    transition: height .22s ease;
}
.wizardStepTitle {
    margin: 0;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 18px;
    color: hsla(0, 0%, 100%, 0.95);
}
.wizardStepDesc {
    margin: 8px 0 0;
    font-size: 14px;
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
.wizardFolderBtn {
    margin-top: 14px;
    padding: 9px 18px;
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
    opacity: 0.5;
}
.wizardFolderBtn:not(:disabled):hover {
    background-color: #fff;
    transform: translateY(-1px);
}

.wizardNav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 18px 24px 22px;
}
.wizardDots {
    display: flex;
    align-items: center;
    gap: 6px;
}
.wizardDot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.2);
    transition: background-color .2s ease, transform .2s ease;
}
.wizardDot.active {
    background-color: hsla(0, 0%, 100%, 0.85);
    transform: scale(1.2);
}
.wizardNavBtns {
    display: flex;
    gap: 8px;
}
.wizardNavBtn {
    padding: 8px 16px;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-family: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background-color .2s ease, color .2s ease;
}
.wizardNavBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: #fff;
}
.wizardNavBtn.primary {
    background-color: hsla(0, 0%, 100%, 0.95);
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
}
.wizardNavBtn.primary:hover {
    background-color: #fff;
}

.modal-enter-active, .modal-leave-active {
    transition: opacity .18s ease;
}
.modal-enter-active .wizardCard, .modal-leave-active .wizardCard {
    transition: transform .18s ease, opacity .18s ease;
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .wizardCard, .modal-leave-to .wizardCard {
    transform: scale(0.96) translateY(4px);
    opacity: 0;
}
</style>
