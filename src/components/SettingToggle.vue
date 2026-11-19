<script setup lang="ts">
import { computed } from 'vue';

const props = defineProps<{
    item: { key?: string; label: string; description?: string; warning?: string };
    checked: boolean;
    compact?: boolean;
    phoneHidden?: boolean;
}>();
defineEmits<{ (e: 'toggle'): void }>();

const ICONS: Record<string, string> = {
    crossfade_on_seek: '<path d="m18 8 4 4-4 4"/><path d="M2 12h20"/><path d="m6 8-4 4 4 4"/>',
    crossfade_between_tracks: '<circle cx="9" cy="12" r="6"/><circle cx="15" cy="12" r="6"/>',
    show_player_extra_block: '<path d="M2 10v3"/><path d="M6 6v11"/><path d="M10 3v18"/><path d="M14 8v7"/><path d="M18 5v13"/><path d="M22 10v3"/>',
    show_donate_badge: '<path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/>',
    show_github_badge: '<path d="M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"/><circle cx="7.5" cy="7.5" r=".5" fill="currentColor"/>',
    discord_rpc_enabled: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
    dynamic_accent_color: '<circle cx="13.5" cy="6.5" r=".5" fill="currentColor"/><circle cx="17.5" cy="10.5" r=".5" fill="currentColor"/><circle cx="8.5" cy="7.5" r=".5" fill="currentColor"/><circle cx="6.5" cy="12.5" r=".5" fill="currentColor"/><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z"/>',
    external_album_order: '<path d="M10 12h11"/><path d="M10 18h11"/><path d="M10 6h11"/><path d="M4 10h2"/><path d="M4 6h1v4"/><path d="M6 18H4c0-1 2-2 2-3s-1-1.5-2-1"/>',
    external_supplement_missing: '<path d="M11 12H3"/><path d="M16 6H3"/><path d="M16 18H3"/><path d="M18 9v6"/><path d="M21 12h-6"/>',
    external_track_covers: '<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>',
    external_artist_avatars: '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>',
    holiday_events: '<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .962 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.962 0z"/><path d="M20 2v4"/><path d="M22 4h-4"/>',
    show_downloads_page: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M12 15V3"/><path d="m7 10 5 5 5-5"/>',
    auto_update_check: '<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>',
    dev_mode: '<path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/>',
};
const DEFAULT_ICON = '<path d="M20 7h-9"/><path d="M14 17H5"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/>';

const iconPaths = computed(() => (props.item.key && ICONS[props.item.key]) || DEFAULT_ICON);
</script>

<template>
    <label class="settingElem" :class="{ compact, phoneHiddenSetting: phoneHidden, checked }">
        <span v-if="!compact" class="settingIcon" aria-hidden="true">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="iconPaths"></svg>
        </span>
        <span class="settingLabelText">
            <span class="settingLabelRow">
                <span class="settingLabel">{{ item.label }}</span>
                <span
                    v-if="item.warning"
                    class="settingWarningIcon"
                    v-tooltip="item.warning"
                    aria-label="Предупреждение"
                >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
                        <path d="M12 9v4" />
                        <path d="M12 17h.01" />
                    </svg>
                </span>
            </span>
            <span v-if="item.description && !compact" class="settingDescText">{{ item.description }}</span>
        </span>
        <input type="checkbox" class="settingCheckbox"
            :checked="checked"
            @change="$emit('toggle')">
    </label>
</template>

<style scoped>
.settingElem {
    padding: 10px 0;
    display: flex;
    flex-direction: row;
    align-items: center;
    width: 100%;
    gap: 14px;
    cursor: pointer;
    user-select: none;
}
.settingElem.compact {
    width: auto;
    padding: 0;
    gap: 10px;
}
.settingElem.compact .settingLabelText {
    order: 2;
}
.settingElem.compact .settingCheckbox {
    order: 1;
}
.settingIcon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 11px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.62);
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.settingElem.checked .settingIcon {
    color: hsl(var(--accent-h), min(var(--accent-s), 80%), 80%);
}
.settingElem:hover .settingIcon {
    background-color: hsla(0, 0%, 100%, 0.09);
}
.settingLabelText {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
}
.settingLabelRow {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
}
.settingLabel {
    font-weight: 600;
    font-size: 14px;
    color: hsla(0, 0%, 100%, 0.92);
}
.settingDescText {
    font-weight: 500;
    font-size: 12.5px;
    line-height: 1.4;
    color: hsla(0, 0%, 100%, 0.45);
}
.settingWarningIcon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    color: hsl(38, 80%, 66%);
    opacity: .75;
    cursor: help;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.settingWarningIcon:hover {
    opacity: 1;
}
.settingCheckbox {
    appearance: none;
    flex-shrink: 0;
    width: 36px;
    height: 20px;
    margin: 0;
    background: hsla(0, 0%, 100%, 0.14);
    border-radius: 31px;
    position: relative;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.settingCheckbox::after {
    content: '';
    position: absolute;
    width: 14px;
    height: 14px;
    top: 3px;
    left: 3px;
    background: hsla(0, 0%, 100%, 0.75);
    border-radius: 50%;
    transition: transform .22s var(--icon-ease), background-color var(--duration-hover) var(--ease-smooth);
}
.settingCheckbox:checked {
    background: #fff;
}
.settingCheckbox:checked::after {
    transform: translateX(16px);
    background: #141414;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingElem {
        min-height: 46px;
        gap: 12px;
    }
    .settingElem.compact {
        min-height: 0;
    }
    .phoneHiddenSetting {
        display: none;
    }
}
</style>
