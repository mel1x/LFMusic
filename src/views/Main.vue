<script setup lang="ts">
import TimeIcon from '../components/Icons/TimeIcon.vue';

// Logic will be added later
</script>

<template>
    <div class="pageRoot">
        <div class="statsBlock">
            <div class="start">

            </div>

            <div class="mid">
                <div class="statBadge" v-tooltip="'Test'"><TimeIcon /> 6ч 32м</div>
            </div>

            <div class="end">

            </div>
        </div>
        <!-- <div class="mixBlock">

        </div> -->
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);

    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;

    transition: .24s ease;

    overflow-y: auto;
}
.statsBlock {
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    width: 100%;
    padding: 4px;
    border-radius: 32px;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
}
.start {
    flex-direction: row;
    align-items: center;
}
.statBadge {
    padding: 4px 9px 4px 5px;
    border-radius: 24px;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 5px;
    font-size: 13px;

    user-select: none;

    transition: .2s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.06);
        color: #fff;
    }
}
.mixBlock {
    margin-top: 16px;
    width: 100%;
    height: 240px;
    border-radius: 20px;
    background: linear-gradient(#151f30 0%, #23334f 100%);
}
</style>