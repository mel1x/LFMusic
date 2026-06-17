<script setup lang="ts">
const categories = [
    {
        id: 'appearance',
        title: 'Оформление',
        desc: 'Цветовая тема, дополнительный блок плеера и бейдж GitHub',
    },
    {
        id: 'playback',
        title: 'Воспроизведение',
        desc: 'Плавные переходы между треками и эквалайзер',
    },
    {
        id: 'library',
        title: 'Библиотека',
        desc: 'Папки с музыкой и имена исполнителей',
    },
    {
        id: 'general',
        title: 'Общее',
        desc: 'Язык интерфейса и интеграции',
    },
] as const;
</script>

<template>
    <div class="pageRoot">
        <h1 class="pageTitle">Настройки</h1>
        <p class="pageSubtitle">Выберите категорию настроек</p>

        <div class="categoryList">
            <RouterLink
                v-for="cat in categories"
                :key="cat.id"
                :to="`/settings/${cat.id}`"
                class="categoryRow"
            >
                <span class="rowText">
                    <span class="rowTitle">{{ cat.title }}</span>
                    <span class="rowDesc">{{ cat.desc }}</span>
                </span>
                <svg class="rowChevron" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="m9 18 6-6-6-6" />
                </svg>
            </RouterLink>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px 28px;
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
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
}
.pageSubtitle {
    user-select: none;
    margin-top: 6px;
    margin-bottom: 28px;
    font-weight: 600;
    font-size: 16px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.categoryList {
    display: flex;
    flex-direction: column;
    max-width: 760px;
    width: 100%;
}
.categoryRow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 20px;
    width: 100%;
    padding: 12px 20px 12px 24px;
    border-radius: 999px;
    cursor: pointer;
    transition: background-color .2s ease;
}
.categoryRow:hover {
    background-color: hsla(0, 0%, 100%, 0.035);
}
.rowText {
    display: flex;
    flex-direction: column;
    gap: 5px;
    flex: 1;
    min-width: 0;
}
.rowTitle {
    user-select: none;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 700;
    font-size: 20px;
    color: #ffffff;
    transition: color .2s ease;
}
.rowDesc {
    user-select: none;
    font-weight: 600;
    font-size: 14px;
    line-height: 1.4;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.rowChevron {
    flex-shrink: 0;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.4);
    transition: color .2s ease, transform .2s ease;
}
.categoryRow:hover .rowChevron {
    color: hsla(0, 0%, 100%, 0.85);
    transform: translateX(4px);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageRoot {
        padding: 18px 14px 24px;
    }
    .pageTitle {
        font-size: 30px;
    }
    .pageSubtitle {
        margin-bottom: 18px;
        font-size: 15px;
    }
    .categoryList {
        max-width: none;
    }
    .categoryRow {
        gap: 14px;
        padding: 18px 12px;
        min-height: 64px;
    }
    .rowTitle {
        font-size: 18px;
    }
}
</style>
