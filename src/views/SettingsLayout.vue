<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import DonateModal from '../components/DonateModal.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import HeartIcon from '../components/Icons/HeartIcon.vue';
import CloseIcon from '../components/Icons/CloseIcon.vue';
import { settingsCategories, settingsSearchIndex, type SettingsCategoryId } from '../utils/settingsCategories';
import { isAndroidRuntime, isPhoneViewport } from '../composables/usePlatform';
import { useI18n } from '../composables/useI18n';

const { t, translateText } = useI18n();
const route = useRoute();
const router = useRouter();

const donateOpen = ref(false);
const query = ref('');
const contentEl = ref<HTMLElement | null>(null);

const available = computed(() => isAndroidRuntime() || isPhoneViewport()
    ? settingsCategories.filter(c => c.id !== 'external')
    : settingsCategories);
const availableIds = computed(() => new Set(available.value.map(c => c.id)));

const norm = (s: string) => s.toLowerCase().replace(/ё/g, 'е').trim();
const matches = (text: string, q: string) => norm(text).includes(q) || norm(translateText(text)).includes(q);

const hits = computed(() => {
    const q = norm(query.value);
    if (q.length < 2) return [];
    return settingsSearchIndex.filter(e => availableIds.value.has(e.category)
        && (matches(e.title, q) || (e.detail && matches(e.detail, q))));
});

const hitCount = computed(() => {
    const m = new Map<SettingsCategoryId, number>();
    for (const h of hits.value) m.set(h.category, (m.get(h.category) ?? 0) + 1);
    return m;
});

const groups = computed(() => {
    const out: { name: string; items: typeof available.value }[] = [];
    for (const c of available.value) {
        const g = out.find(x => x.name === c.group);
        if (g) g.items.push(c);
        else out.push({ name: c.group, items: [c] });
    }
    return out;
});

const TITLE_SELECTOR = '.settingLabel, .settingTitle, .sectionTitle, .subTitle, .languageText > span:first-child, .extraModeTitle';
const DESC_SELECTOR = '.settingDescText, .languageText > small';
const ROW_SELECTOR = '.settingElem, .languageSetting';
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let highlightTimer: ReturnType<typeof setTimeout> | null = null;

function clearHighlights() {
    contentEl.value?.querySelectorAll('.lfFound').forEach(el => el.classList.remove('lfFound'));
}

function highlightPage(scroll: boolean, attempt = 0) {
    if (highlightTimer) {
        clearTimeout(highlightTimer);
        highlightTimer = null;
    }
    const root = contentEl.value;
    clearHighlights();
    const q = norm(query.value);
    if (!root || q.length < 2) return;
    const found: HTMLElement[] = [];
    const add = (el: HTMLElement) => {
        const row = (el.closest(ROW_SELECTOR) as HTMLElement | null) ?? el;
        if (!found.includes(row)) found.push(row);
    };
    root.querySelectorAll<HTMLElement>(TITLE_SELECTOR).forEach(el => {
        if (norm(el.textContent ?? '').includes(q)) add(el);
    });
    root.querySelectorAll<HTMLElement>(DESC_SELECTOR).forEach(el => {
        if (norm(el.textContent ?? '').includes(q)) add(el);
    });
    if (!found.length) {
        if (attempt < 12) highlightTimer = setTimeout(() => highlightPage(scroll, attempt + 1), 60);
        return;
    }
    found.sort((x, y) => x.getBoundingClientRect().top - y.getBoundingClientRect().top);
    found.forEach(el => el.classList.add('lfFound'));
    if (scroll) found[0].scrollIntoView({ block: 'center', behavior: 'smooth' });
}

async function runSearch() {
    const q = norm(query.value);
    if (q.length < 2) {
        clearHighlights();
        return;
    }
    const current = settingsCategories.find(c => route.path === `/settings/${c.id}`)?.id;
    const target = current && hitCount.value.has(current) ? current : hits.value[0]?.category;
    if (target && target !== current) {
        await router.push(`/settings/${target}`);
    }
    await nextTick();
    highlightPage(true);
}

watch(query, () => {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(runSearch, 220);
});

watch(() => route.path, () => {
    if (norm(query.value).length >= 2) nextTick(() => highlightPage(true));
});

function clearQuery() {
    query.value = '';
    clearHighlights();
}

onUnmounted(() => {
    if (searchTimer) clearTimeout(searchTimer);
    if (highlightTimer) clearTimeout(highlightTimer);
});

function categoryTo(id: SettingsCategoryId) {
    return `/settings/${id}`;
}
</script>

<template>
    <div class="settingsLayout">
        <aside class="settingsNav">
            <div class="navTitle">Настройки</div>
            <label class="navSearch">
                <SearchIcon width="15" height="15" />
                <input
                    v-model="query"
                    type="text"
                    placeholder="Найти настройку"
                    spellcheck="false"
                    @keydown.enter="runSearch"
                    @keydown.esc="clearQuery"
                />
                <button v-if="query" type="button" class="navSearchClear" @click="clearQuery">
                    <CloseIcon width="12" height="12" />
                </button>
            </label>
            <div v-for="g in groups" :key="g.name" class="navGroup">
                <div class="navGroupName">{{ g.name }}</div>
                <RouterLink
                    v-for="c in g.items"
                    :key="c.id"
                    :to="categoryTo(c.id)"
                    class="navItem"
                >
                    <svg class="navItemIcon" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="c.icon"></svg>
                    <span class="navItemTitle">{{ c.title }}</span>
                    <span v-if="hitCount.get(c.id)" class="navHitCount">{{ hitCount.get(c.id) }}</span>
                </RouterLink>
            </div>
            <button type="button" class="navItem navDonate" @click="donateOpen = true">
                <HeartIcon width="17" height="17" />
                <span>{{ t('donate.title') }}</span>
            </button>
        </aside>

        <div ref="contentEl" class="settingsContent">
            <RouterView v-slot="{ Component, route: r }">
                <Transition name="settings-tab" mode="out-in">
                    <component :is="Component" :key="r.path" />
                </Transition>
            </RouterView>
        </div>

        <DonateModal :open="donateOpen" @close="donateOpen = false" />
    </div>
</template>

<style scoped>
.settingsLayout {
    display: flex;
    flex-direction: row;
    align-items: flex-start;
    width: 100%;
    min-height: 100%;
    background-color: var(--light-bg);
}

.settingsNav {
    position: sticky;
    top: 0;
    flex: 0 0 220px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 20px 0 20px 16px;
    box-sizing: content-box;
    user-select: none;
}
.navTitle {
    padding: 4px 10px 14px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 18px;
    color: #fff;
}
.navSearch {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    margin-bottom: 10px;
    padding: 0 8px 0 12px;
    border-radius: 12px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.4);
    cursor: text;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.navSearch:focus-within {
    background-color: hsla(0, 0%, 100%, 0.08);
}
.navSearch input {
    flex: 1;
    width: 0;
    min-width: 0;
    background: none;
    border: none;
    outline: none;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    color: #fff;
}
.navSearch input::placeholder {
    color: hsla(0, 0%, 100%, 0.38);
}
.navSearchClear {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 0.7);
    cursor: pointer;
}
.navSearchClear:hover {
    color: #fff;
    background-color: hsla(0, 0%, 100%, 0.14);
}
.navGroup {
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.navGroupName {
    padding: 12px 10px 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: .04em;
    color: hsla(0, 0%, 100%, 0.38);
}
.navItem {
    display: flex;
    align-items: center;
    gap: 11px;
    width: 100%;
    padding: 9px 10px;
    border-radius: 11px;
    font: inherit;
    font-size: 13.5px;
    font-weight: 500;
    text-align: left;
    color: hsla(0, 0%, 100%, 0.72);
    background-color: transparent;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.navItem svg {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.55);
    transition: color var(--duration-hover) var(--ease-smooth);
}
.navItem:hover {
    background-color: hsla(0, 0%, 100%, 0.04);
    color: #fff;
}
.navItem.router-link-active {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.navItem.router-link-active svg {
    color: hsl(var(--accent-h), min(var(--accent-s), 80%), 78%);
}
.navItemTitle {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.navHitCount {
    flex-shrink: 0;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    box-sizing: border-box;
    border-radius: 999px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: hsl(var(--accent-h), min(var(--accent-s), 80%), 82%);
    background-color: hsla(var(--accent-h), min(var(--accent-s), 70%), 60%, 0.16);
}
.navDonate {
    margin-top: 14px;
}
.settingsContent {
    flex: 1;
    min-width: 0;
    align-self: stretch;
}

/* !important: tab pages carry their own `transition` on the same root. */
.settings-tab-enter-active {
    transition: opacity var(--duration-page) var(--ease-smooth), transform var(--duration-page) var(--ease-smooth) !important;
}
.settings-tab-leave-active {
    transition: none !important;
}
.settings-tab-enter-from {
    opacity: 0;
    transform: translateY(6px);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingsLayout {
        display: block;
    }
    .settingsNav {
        display: none;
    }
}
</style>
