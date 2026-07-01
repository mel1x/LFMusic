<script setup lang="ts">
import { ref, computed, watch } from 'vue'

// Shimmer placeholder until the image fully loads, then swap in with a fade;
// falls back to `fallback` on error.
const props = defineProps<{ src: string | null; fallback: string; alt?: string }>()
defineOptions({ inheritAttrs: false })

const loaded = ref(false)
const failed = ref(false)

const effectiveSrc = computed(() => (failed.value || !props.src) ? props.fallback : props.src)

// New src resets to shimmer until it loads again.
watch(() => props.src, () => { loaded.value = false; failed.value = false })

function onLoad() { loaded.value = true }
function onError() { failed.value = true; loaded.value = true }
</script>

<template>
    <div class="coverImage">
        <div v-if="!loaded" class="coverShimmer"></div>
        <img
            :src="effectiveSrc"
            :alt="alt || ''"
            draggable="false"
            class="coverImg"
            :class="{ ready: loaded }"
            v-bind="$attrs"
            @load="onLoad"
            @error="onError"
        />
    </div>
</template>

<style scoped>
.coverImage {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
}
.coverShimmer {
    position: absolute;
    inset: 0;
    background: linear-gradient(
        90deg,
        hsla(0, 0%, 100%, 0.04) 25%,
        hsla(0, 0%, 100%, 0.10) 37%,
        hsla(0, 0%, 100%, 0.04) 63%
    );
    background-size: 400% 100%;
    animation: coverShimmer 1.4s ease infinite;
}
@keyframes coverShimmer {
    0% { background-position: 100% 0; }
    100% { background-position: 0 0; }
}
.coverImg {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    user-select: none;
    opacity: 0;
    transition: opacity 0.25s ease;
}
.coverImg.ready {
    opacity: 1;
}
</style>
