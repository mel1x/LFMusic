<script setup lang="ts">
import { computed, ref } from 'vue';
import ConfirmModal from './ConfirmModal.vue';
import { useUpdater } from '../composables/useUpdater';
import { usePlayer } from '../composables/usePlayer';
import { useI18n } from '../composables/useI18n';
import { isAndroidRuntime } from '../composables/usePlatform';

// Android has no title bar badge, so a found update is offered once per version.
const DISMISSED_KEY = 'lfmusic.updateDismissed';

const { availableVersion, status, progress, install } = useUpdater();
const { saveState } = usePlayer();
const { t } = useI18n();

function readDismissed(): string {
    try {
        return localStorage.getItem(DISMISSED_KEY) ?? '';
    } catch {
        return '';
    }
}

const dismissed = ref(readDismissed());

const open = computed(() => isAndroidRuntime()
    && !!availableVersion.value
    && dismissed.value !== availableVersion.value);

const message = computed(() => {
    if (status.value === 'downloading') return t('updater.downloading', { percent: progress.value });
    if (status.value === 'error') return t('updater.installError');
    return t('updater.androidMessage');
});

function later() {
    dismissed.value = availableVersion.value ?? '';
    try {
        localStorage.setItem(DISMISSED_KEY, dismissed.value);
    } catch {
        // Not persisted: the prompt shows again on the next start.
    }
}
</script>

<template>
    <ConfirmModal
        :open="open"
        :title="t('updater.available', { version: availableVersion ?? '' })"
        :message="message"
        :confirmText="t('updater.install')"
        :cancelText="t('updater.later')"
        @confirm="install(saveState)"
        @cancel="later"
    />
</template>
