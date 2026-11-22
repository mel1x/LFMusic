<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRoute } from 'vue-router';
import MobileScrollHeader from './MobileScrollHeader.vue';
import { settingsCategories } from '../utils/settingsCategories';

defineProps<{ title: string; description?: string }>();

const route = useRoute();

const pageRootEl = ref<HTMLElement | null>(null);
const scrollY = ref(0);

function onPageScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop;
}

function scrollToTop() {
    pageRootEl.value?.scrollTo({ top: 0, behavior: 'smooth' });
}

const current = computed(() => settingsCategories.find(c => route.path === `/settings/${c.id}`));
</script>

<template>
    <div class="pageRoot mobilePage" ref="pageRootEl" @scroll="onPageScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="title" :scrolled="scrollY > 16" @top="scrollToTop" />
        </div>
        <div class="settingsMain">
            <div class="pageHead">
                <svg v-if="current" class="headIcon" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="current.icon"></svg>
                <h1 class="pageTitle">{{ title }}</h1>
            </div>
            <p v-if="description" class="pageDescription">{{ description }}</p>
            <div class="pageBody lfSettingsBody">
                <slot />
            </div>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 0;
    display: flex;
    flex-direction: column;
    width: 100%;
    min-height: 100%;
    background-color: var(--light-bg);
    transition: var(--duration-hover) var(--ease-smooth);
}
.settingsMain {
    max-width: 820px;
    display: flex;
    flex-direction: column;
    padding: 20px 28px 28px;
}
.pageHead {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 6px;
}
.headIcon {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.85);
}
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 24px;
    line-height: 1.15;
    color: #ffffff;
}
.pageDescription {
    user-select: none;
    margin-top: 6px;
    font-weight: 500;
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.45);
}
.pageBody {
    display: flex;
    flex-direction: column;
    margin-top: 20px;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingsMain {
        max-width: none;
        padding: 14px 14px 24px;
    }
    .pageHead {
        margin-top: 4px;
    }
    .headIcon {
        display: none;
    }
    .pageTitle {
        font-size: 28px;
        font-weight: 900;
    }
    .pageDescription {
        font-size: 14px;
        line-height: 1.35;
    }
    .pageBody {
        margin-top: 26px;
    }
}
</style>
