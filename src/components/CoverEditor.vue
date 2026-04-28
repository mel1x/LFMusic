<script setup lang="ts">
import { computed } from 'vue'
import EditIcon from './Icons/EditIcon.vue'
import CloseIcon from './Icons/CloseIcon.vue'
import { useCustomCovers, type CoverKind } from '../composables/useCustomCovers'

const props = defineProps<{
    kind: CoverKind
    coverKey: string | number
    size?: 'small' | 'large'
    iconSize?: number
}>()

const { hasCustomCover, pickAndSetCustomCover, removeCustomCover } = useCustomCovers()

const isCustom = computed(() => hasCustomCover(props.kind, props.coverKey))
const sizeClass = computed(() => `size-${props.size ?? 'small'}`)
const iconPx = computed(() => {
    if (props.iconSize) return props.iconSize
    return props.size === 'large' ? 22 : 16
})

async function onEdit(e: MouseEvent) {
    e.stopPropagation()
    await pickAndSetCustomCover(props.kind, props.coverKey)
}

async function onReset(e: MouseEvent) {
    e.stopPropagation()
    await removeCustomCover(props.kind, props.coverKey)
}
</script>

<template>
    <div
        class="coverEditButtons"
        :class="sizeClass"
        @click.stop
        @mousedown.stop
        @contextmenu.stop
    >
        <button
            class="coverEditBtn editBtn"
            @click="onEdit"
            v-tooltip="isCustom ? 'Изменить обложку' : 'Установить обложку'"
        >
            <EditIcon :width="iconPx" :height="iconPx" />
        </button>
        <Transition name="cover-edit-reset">
            <button
                v-if="isCustom"
                class="coverEditBtn resetBtn"
                @click="onReset"
                v-tooltip="'Сбросить обложку'"
            >
                <CloseIcon :width="iconPx" :height="iconPx" />
            </button>
        </Transition>
    </div>
</template>

<style>
/* ───────── Затемнение обложки только при наведении на кнопки ───────── */
.cover-edit-host {
    position: relative;
}
.cover-edit-host > img {
    transition: filter .28s ease, transform .28s ease;
}
.cover-edit-host:has(.coverEditBtn:hover) > img {
    filter: brightness(0.55);
}

/* ───────── Кнопки редактирования ───────── */
.coverEditButtons {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%) scale(0.92);
    z-index: 4;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    opacity: 0;
    pointer-events: none;
    transition: opacity .26s ease, transform .26s ease;
}
.cover-edit-host:hover .coverEditButtons {
    opacity: 1;
    transform: translate(-50%, -50%) scale(1);
    pointer-events: auto;
}

.coverEditBtn {
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    border-radius: 9999px;
    color: hsla(0, 0%, 100%, 0.95);
    background-color: hsla(0, 0%, 0%, 0.55);
    backdrop-filter: blur(8px);
    border: none;
    transition: background-color .2s ease, color .2s ease, transform .15s ease;
}
.coverEditBtn:hover {
    background-color: hsla(0, 0%, 0%, 0.75);
    color: hsla(0, 0%, 100%, 1);
    transform: scale(1.05);
}
.coverEditBtn:active {
    transform: scale(0.94);
}

.coverEditButtons.size-small .coverEditBtn { padding: 9px; }
.coverEditButtons.size-large .coverEditBtn { padding: 12px; }

.coverEditBtn.resetBtn {
    color: hsla(0, 0%, 100%, 0.85);
}
.coverEditBtn.resetBtn:hover {
    color: hsl(0, 70%, 75%);
}

/* Анимация появления/исчезновения крестика */
.cover-edit-reset-enter-active,
.cover-edit-reset-leave-active {
    transition: opacity .22s ease, transform .22s ease;
}
.cover-edit-reset-enter-from,
.cover-edit-reset-leave-to {
    opacity: 0;
    transform: scale(0.6) rotate(-30deg);
}
</style>
