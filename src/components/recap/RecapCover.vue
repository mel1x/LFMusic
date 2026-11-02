<script setup lang="ts">
import { ref, watch } from 'vue'

// Cover for recap cards; a missing or deleted file falls back to a tinted note.

const props = defineProps<{ url: string | null | undefined; round?: boolean }>()

const failed = ref(false)
watch(() => props.url, () => { failed.value = false })
</script>

<template>
  <div class="recapCover" :class="{ round }">
    <img v-if="url && !failed" :src="url" alt="" draggable="false" @error="failed = true" />
    <svg v-else class="recapCoverNote" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9 18V5l12-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="18" cy="16" r="3" />
    </svg>
  </div>
</template>

<style scoped>
.recapCover {
  position: relative;
  overflow: hidden;
  border-radius: 10px;
  background: linear-gradient(145deg, hsla(var(--rc-h), var(--rc-s), 60%, 0.45), hsla(var(--rc-h), var(--rc-s), 25%, 0.6));
  display: flex;
  align-items: center;
  justify-content: center;
  color: hsla(0, 0%, 100%, 0.75);
  flex-shrink: 0;
}
.recapCover.round {
  border-radius: 50%;
}
.recapCover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.recapCoverNote {
  width: 38%;
  height: 38%;
}
</style>
