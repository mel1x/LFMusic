// Takes the README screenshots: the real LFMusic UI on the fake demo library
// (onboarding-videos/demo/mock.ts), no real music or artwork involved.
//
// Usage: start the dev server (`npx vite`), then `node docs/tools/screens.mjs [name]`.
// Output: docs/screens/<name>.png
import { chromium } from 'playwright-core'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
const OUT = path.join(ROOT, 'docs', 'screens')
const BASE = process.env.DEMO_URL ?? 'http://localhost:1420'
const sleep = (ms) => new Promise(r => setTimeout(r, ms))

const DESKTOP = { width: 1460, height: 860 }
const PHONE = { width: 390, height: 844 }

const SHOTS = {
  mix: { route: '/', track: 38 },
  catalog: { route: '/catalog', track: 2 },
  album: { route: '/album/' + encodeURIComponent('Gravity Assist'), track: 35 },
  stats: {
    route: '/', track: 22,
    async prepare(page) {
      await page.click('.statsBtn')
      await sleep(1800)
    },
  },
  collection: { route: '/collections', track: 14 },
  fullscreen: {
    route: '/', track: 8,
    async prepare(page) {
      await page.click('.fullscreenBtn')
      await sleep(1500)
    },
    mouse: { x: 730, y: 700 },
  },
  'phone-mix': { route: '/', track: 38, viewport: PHONE },
  'phone-album': { route: '/album/' + encodeURIComponent('Afterglow City'), track: 1, viewport: PHONE },
}

async function shoot(browser, name) {
  const s = SHOTS[name]
  const viewport = s.viewport ?? DESKTOP
  const phone = viewport === PHONE
  const page = await browser.newPage({
    viewport, deviceScaleFactor: phone ? 3 : 1.5,
    isMobile: phone, hasTouch: phone,
  })
  page.on('console', m => { if (m.type() === 'error') console.log(`  [${name}]`, m.text()) })
  const params = new URLSearchParams({ lang: 'en', track: String(s.track), route: s.route })
  await page.goto(`${BASE}/onboarding-videos/demo.html?${params}`)
  await sleep(3500)
  // start playback so the accent and the visualizer come alive, then pause at a nice spot
  await page.evaluate(() => document.querySelector('.playBtn')?.click())
  await sleep(1500)
  if (s.prepare) await s.prepare(page)
  const m = s.mouse ?? { x: viewport.width - 2, y: 2 }
  await page.mouse.move(m.x, m.y)
  await sleep(600)
  fs.mkdirSync(OUT, { recursive: true })
  await page.screenshot({ path: path.join(OUT, `${name}.png`) })
  await page.close()
  console.log(`  ${name}`)
}

const only = process.argv[2]
const browser = await chromium.launch({ channel: process.env.BROWSER_CHANNEL ?? 'chrome' })
try {
  for (const name of only ? [only] : Object.keys(SHOTS)) await shoot(browser, name)
} finally {
  await browser.close()
}
