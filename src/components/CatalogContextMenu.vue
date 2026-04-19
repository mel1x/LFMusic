<script setup lang="ts">
import { ref } from 'vue'
import ContextMenu from './ContextMenu.vue'
import PinIcon from './Icons/PinIcon.vue'

defineProps<{
    x: number
    y: number
    centerX?: boolean
}>()

const emit = defineEmits<{
    close: []
}>()

const isPinned = ref(false)

function onTogglePin() {
    isPinned.value = !isPinned.value
}
</script>

<template>
    <ContextMenu :x="x" :y="y" :centerX="centerX" @close="emit('close')">
        <button class="ctx-item" @click="onTogglePin">
            <PinIcon width="16" height="16" />
            <span>{{ isPinned ? 'Открепить' : 'Закрепить' }}</span>
        </button>
    </ContextMenu>
</template>

<style scoped>
.ctx-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: 0.24s ease;
    user-select: none;
    white-space: nowrap;
    text-align: left;
}
.ctx-item:hover {
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 1);
}
.ctx-item svg {
    transition: 0.24s ease;
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.5);
}
.ctx-item:hover svg {
    color: hsla(0, 0%, 100%, 0.8);
}
</style>
