<script setup lang="ts">
import { computed } from 'vue'
import ContextMenu from './ContextMenu.vue'
import EditIcon from './Icons/EditIcon.vue'
import PinIcon from './Icons/PinIcon.vue'
import { useCustomCovers } from '../composables/useCustomCovers'
import { usePins } from '../composables/usePins'

defineProps<{
    x: number
    y: number
    centerX?: boolean
}>()

const emit = defineEmits<{
    close: []
}>()

const { pickAndSetCustomCover } = useCustomCovers()
const { isPinned, togglePin } = usePins()
const pinned = computed(() => isPinned('catalog', 'catalog'))

async function onChangeCover() {
    emit('close')
    await pickAndSetCustomCover('catalog', 'catalog')
}

function onTogglePin() {
    togglePin({ kind: 'catalog', key: 'catalog', name: 'Мой каталог' })
    emit('close')
}
</script>

<template>
    <ContextMenu
        :x="x"
        :y="y"
        :centerX="centerX"
        mobileSheet="content"
        @close="emit('close')"
    >
        <button class="ctx-item" @click="onChangeCover">
            <EditIcon width="16" height="16" />
            <span>Изменить обложку</span>
        </button>
        <button class="ctx-item" @click="onTogglePin">
            <PinIcon width="16" height="16" />
            <span>{{ pinned ? 'Открепить' : 'Закрепить' }}</span>
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
