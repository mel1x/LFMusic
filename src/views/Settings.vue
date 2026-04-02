<script setup lang="ts"></script>

<template>
    <div class="pageRoot">
        <h1 class="pageTitle">Настройки</h1>
        <div class="settingBlock">
            <div class="settingTitle">Локальные папки</div>
            <div class="settingDesc">Папки сканируемые на аудио файлы</div>
            <div class="foldersBlock">

            </div>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    min-height: 100%;
    background-color: #101010;
}
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
    margin-bottom: 32px;
}
.settingBlock {
    display: flex;
    flex-direction: column;
}
.settingTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 700;
    font-size: 18px;
    color: #ffffff;
}
.settingDesc {
    margin-top: 3px;
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 600;
    font-size: 15px;
    color: #707070;
}
</style>
