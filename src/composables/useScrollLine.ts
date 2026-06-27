import { ref, computed, nextTick } from 'vue'

/**
 * Горизонтальный ряд карточек со стрелочной навигацией (как в обычном поиске).
 * Считает адаптивную ширину карточки под ширину контейнера (`--card-w`),
 * отслеживает позицию прокрутки и отдаёт `canPrev`/`canNext` + перелистывание на
 * «страницу». Сам скроллбар скрыт стилями `.lineList`; листают кнопками в шапке.
 *
 * Использование: навесить `el` через `:ref`, прокинуть `--card-w` и `visibility`
 * из `measured`, вызвать `observe()` после монтирования списка.
 */
export function useScrollLine(baseItemWidth = 190, gap = 16) {
  const el = ref<HTMLElement | null>(null)
  const scrollLeft = ref(0)
  const scrollMax = ref(0)
  const itemWidth = ref(baseItemWidth)
  const measured = ref(false)
  let ro: ResizeObserver | null = null

  async function update() {
    const node = el.value
    if (!node) return
    const cw = node.clientWidth
    if (cw > 0) {
      const n = Math.max(1, Math.floor((cw + gap) / (baseItemWidth + gap)))
      itemWidth.value = (cw - (n - 1) * gap) / n
      measured.value = true
    }
    await nextTick()
    scrollLeft.value = node.scrollLeft
    scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth)
  }
  function onScroll() {
    const node = el.value
    if (!node) return
    scrollLeft.value = node.scrollLeft
    scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth)
  }
  function scrollByPage(dir: 1 | -1) {
    const node = el.value
    if (!node) return
    node.scrollBy({ left: dir * node.clientWidth * 0.9, behavior: 'smooth' })
  }
  function observe() {
    const node = el.value
    if (!node) return
    update()
    if (!ro) {
      ro = new ResizeObserver(() => update())
      ro.observe(node)
    }
  }
  function unobserve() { ro?.disconnect(); ro = null }

  const canPrev = computed(() => scrollLeft.value > 1)
  const canNext = computed(() => scrollLeft.value < scrollMax.value - 1)

  return { el, canPrev, canNext, itemWidth, measured, onScroll, scrollByPage, observe, unobserve, update }
}
