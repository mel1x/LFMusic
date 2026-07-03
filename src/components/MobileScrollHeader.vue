<script setup lang="ts">
import { useRouter } from 'vue-router'
import { goBackOrHome } from '../router'
import ArrowUpIcon from './Icons/ArrowUpIcon.vue'
import BackIcon from './Icons/BackIcon.vue'
import SearchIcon from './Icons/SearchIcon.vue'

defineProps<{
    title: string
    scrolled: boolean
}>()

const emit = defineEmits<{
    top: []
}>()

const router = useRouter()

function goBack() {
    goBackOrHome(router)
}

function openSearch() {
    router.push('/search')
}
</script>

<template>
    <div class="mobileScrollHeader" :class="{ scrolled }">
        <button class="mobileScrollButton" type="button" aria-label="Назад" @click="goBack">
            <BackIcon width="22" height="22" />
        </button>
        <div class="mobileScrollTitle">{{ title }}</div>
        <div class="mobileScrollActions">
            <button class="mobileScrollButton" type="button" aria-label="Поиск" @click="openSearch">
                <SearchIcon width="22" height="22" />
            </button>
            <button class="mobileScrollButton" type="button" aria-label="Вверх" @click="emit('top')">
                <ArrowUpIcon width="22" height="22" />
            </button>
        </div>
    </div>
</template>

<style scoped>
.mobileScrollHeader {
    display: none;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    :global(body.lfmusic-phone .stickyHeaderAnchor.hasMobileScrollHeader) {
        top: 0 !important;
        margin: 0 !important;
        z-index: 70 !important;
        pointer-events: none;
    }

    :global(body.lfmusic-phone .stickyHeaderAnchor.hasMobileScrollHeader .stickyHeader) {
        display: none !important;
    }

    :global(body.lfmusic-phone .stickyHeaderAnchor.hasMobileScrollHeader + .backBtn),
    :global(body.lfmusic-phone .stickyHeaderAnchor.hasMobileScrollHeader + .scrollHeader > .backBtn) {
        display: none !important;
    }

    .mobileScrollHeader {
        position: fixed;
        top: var(--safe-top);
        left: 0;
        right: 0;
        z-index: 70;
        display: grid;
        grid-template-columns: 48px minmax(0, 1fr) auto;
        align-items: center;
        height: 56px;
        padding: 6px calc(10px + var(--safe-right)) 6px calc(10px + var(--safe-left));
        color: hsla(0, 0%, 100%, 0.96);
        pointer-events: auto;
        background: hsla(var(--accent-h), min(var(--accent-s), 42%), 10%, 0);
        -webkit-backdrop-filter: blur(0);
        backdrop-filter: blur(0);
        transition: background-color .22s ease, -webkit-backdrop-filter .22s ease, backdrop-filter .22s ease, box-shadow .22s ease;
    }

    .mobileScrollHeader.scrolled {
        background: hsla(var(--accent-h), min(var(--accent-s), 42%), 10%, 0.72);
        -webkit-backdrop-filter: blur(18px);
        backdrop-filter: blur(18px);
        box-shadow: 0 10px 28px rgba(0, 0, 0, 0.18);
    }

    .mobileScrollButton {
        width: 44px;
        height: 44px;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 0;
        border-radius: 999px;
        background: transparent;
        color: hsla(0, 0%, 100%, 0.96);
        cursor: pointer;
        transition: color .18s ease, transform .18s ease;
    }

    .mobileScrollButton:active {
        transform: scale(0.94);
    }

    .mobileScrollTitle {
        min-width: 0;
        padding: 0 10px;
        text-align: center;
        font-size: 15px;
        line-height: 1.15;
        font-weight: 800;
        color: hsla(0, 0%, 100%, 0.94);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        opacity: 0;
        transform: translateY(3px);
        transition: opacity .22s ease, transform .22s ease;
        user-select: none;
    }

    .mobileScrollHeader.scrolled .mobileScrollTitle {
        opacity: 1;
        transform: translateY(0);
    }

    .mobileScrollActions {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        gap: 2px;
    }
}
</style>
