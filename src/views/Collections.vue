<script setup lang="ts">

</script>

<template>
    <div class="pageRoot">
        <h1 class="pageTitle">Коллекция</h1>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle">Мои плейлисты</div>
                <div class="lineControls">
                    <button class="prevBtn"></button>
                    <button class="nextBtn"></button>
                </div>
            </div>
            <div class="lineList">

            </div>
        </div>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle">Альбомы</div>
                <div class="lineControls">
                    <button class="prevBtn"></button>
                    <button class="nextBtn"></button>
                </div>
            </div>
            <div class="lineList">

            </div>
        </div>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle">Исполнители</div>
                <div class="lineControls">
                    <button class="prevBtn"></button>
                    <button class="nextBtn"></button>
                </div>
            </div>
            <div class="lineList">

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
    height: 100%;
    background-color: var(--light-bg);
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background: transparent; }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
    margin-bottom: 48px;
}
</style>
