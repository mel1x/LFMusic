<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'

const props = withDefaults(defineProps<{
    open: boolean
    title: string
    message?: string
    confirmText?: string
    cancelText?: string
}>(), {
    confirmText: 'Подтвердить',
    cancelText: 'Отмена',
})

const emit = defineEmits<{
    confirm: []
    cancel: []
}>()

function onConfirm() {
    emit('confirm')
}

function onCancel() {
    emit('cancel')
}

function onBackdropMousedown(e: MouseEvent) {
    if (e.target === e.currentTarget) onCancel()
}

function onKeydown(e: KeyboardEvent) {
    if (!props.open) return
    if (e.key === 'Escape') {
        e.preventDefault()
        onCancel()
    } else if (e.key === 'Enter') {
        e.preventDefault()
        onConfirm()
    }
}

onMounted(() => {
    document.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
    document.removeEventListener('keydown', onKeydown)
})
</script>

<template>
    <Teleport to="body">
        <Transition name="modal">
            <div
                v-if="open"
                class="modalBackdrop"
                @mousedown="onBackdropMousedown"
            >
                <div class="modalCard" @mousedown.stop>
                    <div class="modalTitle">{{ title }}</div>
                    <div v-if="message" class="modalMessage">{{ message }}</div>
                    <div class="modalActions">
                        <button class="modalBtn" @click="onCancel">{{ cancelText }}</button>
                        <button class="modalBtn confirmBtn" @click="onConfirm">{{ confirmText }}</button>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.modalBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.55);
    backdrop-filter: blur(4px);
}
.modalCard {
    min-width: 340px;
    max-width: 420px;
    padding: 26px 26px 16px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
    gap: 10px;
}
.modalTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 700;
    font-size: 21px;
    color: hsla(0, 0%, 100%, 0.92);
    user-select: none;
}
.modalMessage {
    font-size: 16px;
    line-height: 1.45;
    color: hsla(0, 0%, 100%, 0.6);
    user-select: none;
}
.modalActions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 10px;
}
.modalBtn {
    user-select: none;
    padding: 6px 14px;
    border-radius: 16px;
    border: none;
    background-color: hsla(0, 0%, 100%, 0);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-family: inherit;
    font-weight: 500;
    font-size: 14px;
    cursor: pointer;
    transition: .24s ease;
}
.modalBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #ffffff;
}
.confirmBtn {
    color: hsla(0, 0%, 100%, 0.85);
    background-color: hsla(0, 0%, 100%, 0.06);
}
.confirmBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: #ffffff;
}

.modal-enter-active, .modal-leave-active {
    transition: opacity .18s ease;
}
.modal-enter-active .modalCard, .modal-leave-active .modalCard {
    transition: transform .18s ease, opacity .18s ease;
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .modalCard, .modal-leave-to .modalCard {
    transform: scale(0.96) translateY(4px);
    opacity: 0;
}
</style>
