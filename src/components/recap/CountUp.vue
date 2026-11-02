<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

// Number that counts up from zero once the card appears.

const props = withDefaults(defineProps<{
  value: number
  locale: string
  duration?: number
  delay?: number
  suffix?: string
}>(), { duration: 1400, delay: 150, suffix: '' })

const shown = ref(0)
let frame: number | null = null
let timer: ReturnType<typeof setTimeout> | null = null

onMounted(() => {
  timer = setTimeout(() => {
    const start = performance.now()
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / props.duration)
      shown.value = Math.round(props.value * (1 - Math.pow(1 - t, 3)))
      frame = t < 1 ? requestAnimationFrame(step) : null
    }
    frame = requestAnimationFrame(step)
  }, props.delay)
})

onUnmounted(() => {
  if (timer) clearTimeout(timer)
  if (frame !== null) cancelAnimationFrame(frame)
})
</script>

<template>
  <span class="countUp">{{ shown.toLocaleString(locale) }}{{ suffix }}</span>
</template>

<style scoped>
.countUp {
  font-variant-numeric: tabular-nums;
}
</style>
