<script setup lang="ts">
</script>

<template>
    <div class="mainWrapper">
        <aside>

        </aside>
        <main>

        </main>
    </div>
</template>

<style scoped>
.mainWrapper {
    display: flex;
    flex-direction: row;
}
aside {
    width: 260px;
    height: calc(100vh - 38px - 104px);
}
main {
    width: calc(100% - 260px - 12px);
    height: calc(100vh - 38px - 104px);
    border-radius: 16px;
    border: 0.0625rem solid hsla(0, 0%, 100%, 0.04);
}
</style>
