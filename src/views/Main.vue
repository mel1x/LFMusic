<script setup lang="ts">
import TimeIcon from '../components/Icons/TimeIcon.vue';

// Logic will be added later
</script>

<template>
    <div class="pageRoot">
        <div class="mixBlock">

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

    transition: .24s ease;

    overflow-y: auto;
}
.mixBlock {
    margin-top: 16px;
    width: 100%;
    height: 240px;
    border-radius: 20px;
}
</style>