<script setup lang="ts">
import DownloadIcon from '../components/Icons/DownloadIcon.vue';
</script>

<template>
    <div class="pageRoot">
        <h1 class="pageTitle">Скачивание</h1>
        <p class="pageSubtitle">Загрузка отсутствующих треков из внешних источников</p>

        <div class="emptyState">
            <div class="emptyIcon"><DownloadIcon width="22" height="22" /></div>
            <div class="emptyText">Здесь пока пусто</div>
            <div class="emptyDesc">Раздел в разработке.</div>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
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
    font-weight: 600;
    font-size: 16px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.emptyState {
    flex: 1;
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
}
.emptyIcon {
    padding: 10px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #ffffff;
}
.emptyText {
    user-select: none;
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.7);
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 270px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.3);
}
</style>
