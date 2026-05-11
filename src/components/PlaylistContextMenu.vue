<script setup lang="ts">
import ContextMenu from './ContextMenu.vue'
import PinIcon from './Icons/PinIcon.vue'
import DeleteIcon from './Icons/DeleteIcon.vue'
import ArrowUpIcon from './Icons/ArrowUpIcon.vue'

withDefaults(defineProps<{
    x: number
    y: number
    alignRight?: boolean
    anchorBottom?: boolean
    centerX?: boolean
    centerY?: boolean
    isPinned?: boolean
    showMoveToTop?: boolean
    showDelete?: boolean
    /** Использовать стрелку вверх и подпись «Поднять вверх»
     *  (для закреплений в боковой панели). По умолчанию —
     *  стрелка влево и подпись «Перенести в начало». */
    moveToTopUp?: boolean
}>(), {
    showDelete: true,
    moveToTopUp: false,
})

const emit = defineEmits<{
    close: []
    togglePin: []
    delete: []
    moveToTop: []
}>()

function onTogglePin() {
    emit('togglePin')
    emit('close')
}

function onDelete() {
    emit('delete')
    emit('close')
}

function onMoveToTop() {
    emit('moveToTop')
    emit('close')
}
</script>

<template>
    <ContextMenu
        :x="x"
        :y="y"
        :alignRight="alignRight"
        :anchorBottom="anchorBottom"
        :centerX="centerX"
        :centerY="centerY"
        mobileSheet="content"
        @close="emit('close')"
    >
        <button v-if="showMoveToTop" class="ctx-item" @click="onMoveToTop">
            <ArrowUpIcon v-if="moveToTopUp" width="16" height="16" />
            <svg v-else xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M19 12H5"/>
                <path d="m12 19-7-7 7-7"/>
            </svg>
            <span>{{ moveToTopUp ? 'Поднять вверх' : 'Перенести в начало' }}</span>
        </button>
        <button class="ctx-item" @click="onTogglePin">
            <PinIcon width="16" height="16" />
            <span>{{ isPinned ? 'Открепить' : 'Закрепить' }}</span>
        </button>
        <button v-if="showDelete" class="ctx-item" @click="onDelete">
            <DeleteIcon width="16" height="16" />
            <span>Удалить плейлист</span>
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
