<script setup lang="ts">
defineProps<{
    item: { label: string; description?: string; warning?: string };
    checked: boolean;
    compact?: boolean;
    phoneHidden?: boolean;
}>();
defineEmits<{ (e: 'toggle'): void }>();
</script>

<template>
    <label class="settingElem" :class="{ compact, phoneHiddenSetting: phoneHidden }">
        <input type="checkbox" class="settingCheckbox"
            :checked="checked"
            @change="$emit('toggle')">
        <span class="settingLabelText">
            <span>{{ item.label }}</span>
            <span
                v-if="item.description"
                class="settingHelpIcon"
                v-tooltip="item.description"
                aria-label="Об этой настройке"
            >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <circle cx="12" cy="12" r="10" />
                    <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
                    <path d="M12 17h.01" />
                </svg>
            </span>
            <span
                v-if="item.warning"
                class="settingWarningIcon"
                v-tooltip="item.warning"
                aria-label="Предупреждение"
            >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
                    <path d="M12 9v4" />
                    <path d="M12 17h.01" />
                </svg>
            </span>
        </span>
    </label>
</template>

<style scoped>
.settingElem {
    padding: 12px 0px;
    display: flex;
    flex-direction: row;
    align-items: center;
    width: 100%;
    gap: 16px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 600;
    font-size: 16px;
}
.settingElem.compact {
    width: auto;
    padding: 0;
}
.settingElem span {
    user-select: none;
    transition: .24s ease-in-out;

    &:hover {
        color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 90%, 0.62);
    }
}
.settingLabelText {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
}
.settingWarningIcon,
.settingHelpIcon {
    position: relative;
    top: 1px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    margin-left: -5px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 82%, 0.42);
    cursor: help;
    transition: color .18s ease;
}
.settingWarningIcon:hover,
.settingHelpIcon:hover {
    color: hsla(0, 0%, 100%, 0.78) !important;
}
.settingCheckbox {
    appearance: none;
    flex-shrink: 0;
    width: 32px;
    height: 20px;
    background: var(--dark-bg);
    border-radius: 31px;
    position: relative;
    cursor: pointer;
    transition: 0.24s ease;
}
.settingCheckbox::after {
    content: '';
    position: absolute;
    width: 16px;
    height: 16px;
    top: 2px;
    left: 2px;
    background: hsla(0, 0%, 89%, 1);
    border-radius: 50%;
    transition: 0.24s ease;
}
.settingCheckbox:checked {
    background: #fff;
}
.settingCheckbox:checked + span {
    color: hsla(0, 0%, 90.1%, 1);
}
.settingCheckbox:checked::after {
    transform: translateX(12px);
    background: var(--dark-bg);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingElem {
        min-height: 46px;
        align-items: center;
        gap: 12px;
        font-size: 15px;
    }
    .settingElem.compact {
        min-height: 0;
    }
    .phoneHiddenSetting {
        display: none;
    }
}
</style>
