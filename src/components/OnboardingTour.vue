<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from '../composables/useI18n';
import { usePlayer } from '../composables/usePlayer';
import { isAndroidRuntime, isPhoneViewport } from '../composables/usePlatform';

// First launch tour: four short explainer videos recorded from the real UI
// (see onboarding-videos/record.mjs). Shown once, then remembered locally.
const STORAGE_KEY = 'lfmusic.onboardingDone.v2';

const videos = import.meta.glob('../assets/onboarding/*.{webm,mp4}', {
    eager: true,
    query: '?url',
    import: 'default',
}) as Record<string, string>;

const STEPS = [
    { id: 'library', color: '#ff4f8b' },
    { id: 'play', color: '#ff8a1f' },
    { id: 'collection', color: '#b388eb' },
    { id: 'mix', color: '#2bb3c0' },
] as const;

const router = useRouter();
const { t, language } = useI18n();
const { catalogPaths } = usePlayer();

function readDone(): boolean {
    try {
        return localStorage.getItem(STORAGE_KEY) === '1';
    } catch {
        return false;
    }
}

const visible = ref(!readDone() && !isAndroidRuntime() && !isPhoneViewport());
const index = ref(0);
const progress = ref(0);
const videoEl = ref<HTMLVideoElement | null>(null);

const step = computed(() => STEPS[index.value]);
const isLast = computed(() => index.value === STEPS.length - 1);

function src(id: string, ext: 'webm' | 'mp4'): string | undefined {
    return videos[`../assets/onboarding/${id}_${language.value}.${ext}`]
        ?? videos[`../assets/onboarding/${id}_ru.${ext}`];
}

function goTo(i: number) {
    index.value = Math.max(0, Math.min(STEPS.length - 1, i));
}

function next() {
    if (isLast.value) finish();
    else goTo(index.value + 1);
}

function finish() {
    visible.value = false;
    try {
        localStorage.setItem(STORAGE_KEY, '1');
    } catch {
        // Not critical: the tour just shows again next launch
    }
    if (catalogPaths.value.length === 0) router.push('/settings/library');
}

function onTimeUpdate() {
    const v = videoEl.value;
    if (v && v.duration) progress.value = v.currentTime / v.duration;
}

// The last video loops, the others hand over to the next step
function onEnded() {
    if (isLast.value) {
        videoEl.value?.play().catch(() => {});
    } else {
        goTo(index.value + 1);
    }
}

watch(index, () => {
    progress.value = 0;
});

function onKeydown(e: KeyboardEvent) {
    if (!visible.value) return;
    if (e.key === 'Escape') finish();
    else if (e.key === 'ArrowRight' || e.key === 'Enter') next();
    else if (e.key === 'ArrowLeft') goTo(index.value - 1);
    else return;
    e.preventDefault();
    e.stopPropagation();
}

onMounted(() => document.addEventListener('keydown', onKeydown, true));
onUnmounted(() => document.removeEventListener('keydown', onKeydown, true));
</script>

<template>
    <Teleport to="body">
        <Transition name="onboarding">
            <div v-if="visible" class="onboarding" :style="{ '--ob-color': step.color }">
                <div class="obGlow obGlowA"></div>
                <div class="obGlow obGlowB"></div>

                <div class="obCard">
                    <div class="obSide">
                        <div class="obKicker">{{ t('onboarding.kicker') }}</div>

                        <Transition name="ob-text" mode="out-in">
                            <div class="obText" :key="step.id">
                                <div class="obStep">{{ t('onboarding.step', { n: index + 1, total: STEPS.length }) }}</div>
                                <h1 class="obTitle">{{ t(`onboarding.${step.id}Title`) }}</h1>
                                <p class="obDesc">{{ t(`onboarding.${step.id}Text`) }}</p>
                            </div>
                        </Transition>

                        <div class="obSteps">
                            <button
                                v-for="(s, i) in STEPS"
                                :key="s.id"
                                class="obStepBtn"
                                :class="{ active: i === index, done: i < index }"
                                @click="goTo(i)"
                            >
                                <span class="obStepBar">
                                    <span
                                        class="obStepFill"
                                        :style="{ transform: `scaleX(${i < index ? 1 : i === index ? progress : 0})` }"
                                    ></span>
                                </span>
                                <span class="obStepLabel">{{ t(`onboarding.${s.id}Label`) }}</span>
                            </button>
                        </div>

                        <div class="obActions">
                            <button class="obSkip" @click="finish">{{ t('onboarding.skip') }}</button>
                            <div class="obNav">
                                <button v-if="index > 0" class="obBack" @click="goTo(index - 1)">{{ t('onboarding.back') }}</button>
                                <button class="obNext" @click="next">
                                    {{ isLast ? (catalogPaths.length === 0 ? t('onboarding.addMusic') : t('onboarding.start')) : t('onboarding.next') }}
                                </button>
                            </div>
                        </div>
                    </div>

                    <div class="obStage">
                        <Transition name="ob-video">
                            <video
                                :key="`${step.id}-${language}`"
                                ref="videoEl"
                                class="obVideo"
                                autoplay
                                muted
                                playsinline
                                disablepictureinpicture
                                @timeupdate="onTimeUpdate"
                                @ended="onEnded"
                            >
                                <source v-if="src(step.id, 'webm')" :src="src(step.id, 'webm')" type="video/webm" />
                                <source v-if="src(step.id, 'mp4')" :src="src(step.id, 'mp4')" type="video/mp4" />
                            </video>
                        </Transition>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.onboarding {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    z-index: 1000000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px;
    box-sizing: border-box;
    overflow: hidden;
    background: hsla(0, 0%, 3%, 0.86);
    backdrop-filter: blur(18px);
    user-select: none;
}

/* Two soft light pools in the step color, placed asymmetrically */
.obGlow {
    position: absolute;
    border-radius: 50%;
    pointer-events: none;
    filter: blur(90px);
    transition: background-color .9s ease;
    background-color: var(--ob-color);
    animation: obBreathe 8s ease-in-out infinite;
}
.obGlowA {
    width: 46vw;
    height: 46vw;
    left: -12vw;
    top: -18vw;
    opacity: .2;
}
.obGlowB {
    width: 34vw;
    height: 34vw;
    right: -8vw;
    bottom: -16vw;
    opacity: .14;
    animation-delay: -4s;
}
@keyframes obBreathe {
    0%, 100% { transform: scale(1); }
    50% { transform: scale(1.12); }
}

.obCard {
    position: relative;
    display: flex;
    align-items: stretch;
    gap: 44px;
    width: min(1320px, 100%);
    max-height: 100%;
}

.obSide {
    flex: 0 0 390px;
    display: flex;
    flex-direction: column;
    padding: 6px 0;
    min-width: 0;
}

.obKicker {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: .12em;
    text-transform: uppercase;
    color: var(--ob-color);
    transition: color .9s ease;
}

.obText {
    margin-top: 26px;
}
.obStep {
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, .45);
    font-variant-numeric: tabular-nums;
}
.obTitle {
    margin: 8px 0 0;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 34px;
    line-height: 1.08;
    font-weight: 800;
    color: #fff;
}
.obDesc {
    margin: 14px 0 0;
    font-size: 15px;
    line-height: 1.5;
    font-weight: 500;
    color: hsla(0, 0%, 100%, .62);
}

.obSteps {
    margin-top: auto;
    padding-top: 28px;
    display: flex;
    flex-direction: column;
    gap: 4px;
}
.obStepBtn {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 7px;
    padding: 9px 10px;
    margin: 0 -10px;
    border-radius: 12px;
    background: transparent;
    cursor: pointer;
    transition: background-color .2s ease;
}
.obStepBtn:hover {
    background: hsla(0, 0%, 100%, .05);
}
.obStepBar {
    position: relative;
    width: 100%;
    height: 3px;
    border-radius: 3px;
    overflow: hidden;
    background: hsla(0, 0%, 100%, .1);
}
.obStepFill {
    position: absolute;
    inset: 0;
    transform-origin: 0 50%;
    background: hsla(0, 0%, 100%, .75);
    transition: transform .25s linear;
}
.obStepBtn.active .obStepFill {
    background: var(--ob-color);
}
.obStepLabel {
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, .4);
    transition: color .2s ease;
}
.obStepBtn.active .obStepLabel,
.obStepBtn:hover .obStepLabel {
    color: hsla(0, 0%, 100%, .92);
}

.obActions {
    margin-top: 22px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
}
.obNav {
    display: flex;
    gap: 8px;
}
.obSkip,
.obBack {
    padding: 9px 14px;
    margin-left: -14px;
    border-radius: 20px;
    background: transparent;
    color: hsla(0, 0%, 100%, .5);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: color .2s ease, background-color .2s ease;
}
.obBack {
    margin-left: 0;
    background: hsla(0, 0%, 100%, .08);
    color: hsla(0, 0%, 100%, .85);
}
.obSkip:hover,
.obBack:hover {
    color: #fff;
    background: hsla(0, 0%, 100%, .1);
}
.obNext {
    padding: 9px 20px;
    border-radius: 20px;
    background: #fff;
    color: #111;
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    cursor: pointer;
    transition: transform .15s ease, box-shadow .2s ease;
}
.obNext:hover {
    box-shadow: 0 6px 22px hsla(0, 0%, 100%, .18);
}
.obNext:active {
    transform: scale(.97);
}

.obStage {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    align-self: center;
    aspect-ratio: 16 / 9;
    max-height: calc(100vh / var(--overlay-scale, 1) - 80px);
    border-radius: 18px;
    overflow: hidden;
    background: #0d0d0d;
    box-shadow: 0 0 0 1px hsla(0, 0%, 100%, .08), 0 30px 80px rgba(0, 0, 0, .5);
}
.obVideo {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}

.onboarding-enter-active,
.onboarding-leave-active {
    transition: opacity .35s ease;
}
.onboarding-enter-active .obCard,
.onboarding-leave-active .obCard {
    transition: transform .45s cubic-bezier(.2, .8, .2, 1);
}
.onboarding-enter-from,
.onboarding-leave-to {
    opacity: 0;
}
.onboarding-enter-from .obCard {
    transform: translateY(14px) scale(.985);
}
.onboarding-leave-to .obCard {
    transform: scale(.985);
}

.ob-text-enter-active,
.ob-text-leave-active {
    transition: opacity .22s ease, transform .22s ease;
}
.ob-text-enter-from {
    opacity: 0;
    transform: translateY(8px);
}
.ob-text-leave-to {
    opacity: 0;
    transform: translateY(-6px);
}

.ob-video-enter-active,
.ob-video-leave-active {
    transition: opacity .4s ease;
}
.ob-video-enter-from,
.ob-video-leave-to {
    opacity: 0;
}

@media (max-width: 900px) {
    .obCard {
        flex-direction: column-reverse;
        gap: 20px;
    }
    .obSide {
        flex: 0 0 auto;
    }
    .obStage {
        align-self: stretch;
    }
}
</style>
