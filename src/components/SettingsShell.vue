<script setup lang="ts">
import { ref } from 'vue';
import { useRouter } from 'vue-router';
import MobileScrollHeader from './MobileScrollHeader.vue';

defineProps<{ title: string; description?: string }>();

const router = useRouter();
function goBack() {
    router.push('/settings');
}

const pageRootEl = ref<HTMLElement | null>(null);
const scrollY = ref(0);

function onPageScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop;
}

function scrollToTop() {
    pageRootEl.value?.scrollTo({ top: 0, behavior: 'smooth' });
}
</script>

<template>
    <div class="pageRoot mobilePage" ref="pageRootEl" @scroll="onPageScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="title" :scrolled="scrollY > 16" @top="scrollToTop" />
        </div>
        <button type="button" class="backBtn" @click="goBack">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                <path d="m15 18-6-6 6-6" />
            </svg>
            <span>Все настройки</span>
        </button>
        <div class="pageHead">
            <h1 class="pageTitle">{{ title }}</h1>
            <p v-if="description" class="pageDescription">{{ description }}</p>
        </div>
        <div class="pageBody">
            <slot />
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 16px 28px 24px;
    display: flex;
    flex-direction: column;
    width: 100%;
    min-height: 100%;
    background-color: var(--light-bg);

    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;

    transition: .24s ease;
}
.backBtn {
    user-select: none;
    align-self: flex-start;
    display: inline-flex;
    flex-direction: row;
    align-items: center;
    gap: 5px;
    padding: 7px 14px 7px 9px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.55);
    font-weight: 600;
    font-size: 14px;
    cursor: pointer;
    transition: background-color .2s ease, color .2s ease, transform .2s ease;
}
.backBtn svg {
    flex-shrink: 0;
    transition: transform .2s ease;
}
.backBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #ffffff;
}
.backBtn:hover svg {
    transform: translateX(-2px);
}
.pageHead {
    margin-top: 14px;
    margin-bottom: 36px;
}
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 34px;
    line-height: 1.1;
    color: #ffffff;
}
.pageDescription {
    user-select: none;
    margin-top: 8px;
    font-weight: 600;
    font-size: 15px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.pageBody {
    display: flex;
    flex-direction: column;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageRoot {
        padding: 14px 14px 24px;
    }
    .backBtn {
        min-height: 40px;
    }
    .pageHead {
        margin-bottom: 26px;
    }
    .pageTitle {
        font-size: 28px;
    }
    .pageDescription {
        font-size: 14px;
        line-height: 1.35;
    }
}
</style>
