<script setup lang="ts">
import { useAndroidShell } from '../composables/useAndroidShell';

const {
    showAndroidPermissionWarning,
    androidPermissionWarningText,
    requestAndroidPermissions,
    refreshAndroidPermissions,
} = useAndroidShell();
</script>

<template>
    <div v-if="showAndroidPermissionWarning" class="androidPermissionWarning" role="alertdialog" aria-modal="true">
        <div class="androidPermissionCard">
            <div class="androidPermissionKicker">Требуются разрешения</div>
            <div class="androidPermissionTitle">LFMusic нужен доступ</div>
            <div class="androidPermissionText">{{ androidPermissionWarningText }}</div>
            <div class="androidPermissionActions">
                <button class="androidPermissionButton primary" type="button" @click="requestAndroidPermissions">
                    Выдать разрешения
                </button>
                <button class="androidPermissionButton" type="button" @click="refreshAndroidPermissions">
                    Проверить снова
                </button>
            </div>
        </div>
    </div>
</template>

<!-- Стили НЕ scoped: в App.vue остаются глобальные мобильные оверрайды
     (.androidPermissionWarning/.androidPermissionCard) без !important, поэтому база
     должна сохранять прежнюю (глобальную) специфичность. -->
<style>
.androidPermissionWarning {
    position: fixed;
    inset: 0;
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: calc(var(--safe-top) + 18px) calc(var(--safe-right) + 18px) calc(var(--safe-bottom) + 18px) calc(var(--safe-left) + 18px);
    background: hsla(0, 0%, 0%, 0.58);
    backdrop-filter: blur(18px);
}
.androidPermissionCard {
    width: min(420px, 100%);
    border: 1px solid hsla(0, 0%, 100%, 0.1);
    border-radius: 24px;
    padding: 22px;
    background: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%);
    box-shadow: 0 26px 80px hsla(0, 0%, 0%, 0.5);
    color: #fff;
}
.androidPermissionKicker {
    margin-bottom: 8px;
    font-size: 12px;
    line-height: 1.2;
    font-weight: 800;
    letter-spacing: 0;
    color: hsla(0, 0%, 100%, 0.5);
}
.androidPermissionTitle {
    font-size: 24px;
    line-height: 1.12;
    font-weight: 900;
}
.androidPermissionText {
    margin-top: 12px;
    font-size: 14px;
    line-height: 1.45;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.72);
}
.androidPermissionActions {
    display: flex;
    gap: 10px;
    margin-top: 20px;
}
.androidPermissionButton {
    min-height: 44px;
    flex: 1 1 0;
    min-width: 0;
    border-radius: 14px;
    padding: 10px 14px;
    background: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.88);
    font-size: 13px;
    font-weight: 800;
    cursor: pointer;
}
.androidPermissionButton.primary {
    background: #fff;
    color: #0f0f0f;
}
</style>
