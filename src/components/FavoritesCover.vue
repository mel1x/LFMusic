<script setup lang="ts">
import favoritesCover from '../assets/favorites.webp'
</script>

<!-- Cover for the built-in "Мне нравится" playlist. Mirrors the catalog cover:
     a grayscale base image tinted with the accent color via mix-blend-mode, so
     it re-colors along with the rest of the UI. Fills its container; the parent
     supplies size/border-radius (e.g. via a `cover` class). -->
<template>
    <div class="favCover">
        <img :src="favoritesCover" class="favCoverImg" alt="" draggable="false" />
        <div class="favCoverTint"></div>
    </div>
</template>

<style scoped>
.favCover {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    border-radius: inherit;
}
.favCoverImg {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    filter: grayscale(1);
    user-select: none;
}
.favCoverTint {
    position: absolute;
    inset: 0;
    background-color: hsla(var(--accent-h), min(var(--accent-s), 45%), 50%, 1);
    mix-blend-mode: color;
    pointer-events: none;
    transition: background-color .28s ease;
}
</style>
