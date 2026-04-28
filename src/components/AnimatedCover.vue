<script setup lang="ts">
defineProps<{ src: string }>()
defineOptions({ inheritAttrs: false })
</script>

<template>
    <Transition name="cover-fade" mode="out-in">
        <img :key="src" :src="src" v-bind="$attrs" draggable="false" />
    </Transition>
</template>

<style>
.cover-fade-enter-active,
.cover-fade-leave-active {
    transition: opacity 0.22s ease, transform 0.22s ease;
}
.cover-fade-enter-from {
    opacity: 0;
    transform: scale(1.02);
}
.cover-fade-leave-to {
    opacity: 0;
    transform: scale(0.99);
}
</style>
