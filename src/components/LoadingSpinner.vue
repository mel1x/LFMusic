<script setup lang="ts">
defineOptions({ inheritAttrs: false })
</script>

<template>
    <div class="loadingState" v-bind="$attrs">
        <div class="loadingSpinner"></div>
    </div>
</template>

<style>
/* Global (unscoped) transition used by cover-gate swaps across views. */
.gate-fade-enter-active,
.gate-fade-leave-active {
    transition: opacity 0.24s ease;
}
.gate-fade-enter-from,
.gate-fade-leave-to {
    opacity: 0;
}
</style>

<style scoped>
.loadingState {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 60px 0;
    min-height: 160px;
}
.loadingSpinner {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 3px solid hsla(0, 0%, 100%, 0.1);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 65%, 0.9);
    animation: spin 0.8s linear infinite;
}
@keyframes spin {
    to { transform: rotate(360deg); }
}
</style>
