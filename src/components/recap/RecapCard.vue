<script setup lang="ts">
import { computed } from 'vue'
import type { RecapArtist, RecapTrack, YearRecap } from '../../composables/useYearRecap'
import { useI18n } from '../../composables/useI18n'
import { useExternalSources } from '../../composables/useExternalSources'
import { formatTotalDuration, pluralizeRu } from '../../utils/format'
import { genreDisplayName } from '../../utils/genres'
import { peakIndex, recapCovers, type CharacterId, type MomentMode, type RecapCard } from './recapCards'
import CountUp from './CountUp.vue'
import RecapCover from './RecapCover.vue'

const props = defineProps<{
  card: RecapCard
  recap: YearRecap
  playingUrl: string | null
  paused?: boolean
}>()
const emit = defineEmits<{ (e: 'sound', url: string | null): void; (e: 'replay'): void }>()

const { t, language } = useI18n()
const { avatarFor } = useExternalSources()

const locale = computed(() => (language.value === 'en' ? 'en-US' : 'ru-RU'))

function num(n: number): string {
  return Math.round(n).toLocaleString(locale.value)
}

/** Plural word for `n` from a "one|few|many" (ru) or "one|many" (en) locale string. */
function unit(n: number, key: string): string {
  const forms = t(`recap.units.${key}`).split('|')
  if (language.value === 'en') return Math.abs(n) === 1 ? forms[0] : forms[1] ?? forms[0]
  return pluralizeRu(n, [forms[0], forms[1] ?? forms[0], forms[2] ?? forms[1] ?? forms[0]])
}

function count(n: number, key: string): string {
  return `${num(n)} ${unit(n, key)}`
}

function day(date: string): string {
  const [y, m, d] = date.split('-').map(Number)
  return new Intl.DateTimeFormat(locale.value, { day: 'numeric', month: 'long' }).format(new Date(y, m - 1, d))
}

function dateTime(unix: number): string {
  return new Intl.DateTimeFormat(locale.value, {
    day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit',
  }).format(new Date(unix * 1000))
}

function monthName(i: number): string {
  return new Intl.DateTimeFormat(locale.value, { month: 'long' }).format(new Date(2025, i, 1))
}

function monthLetter(i: number): string {
  return monthName(i).slice(0, 1).toUpperCase()
}

function hourLabel(h: number): string {
  return `${String(h).padStart(2, '0')}:00`
}

function title(track: RecapTrack): string {
  return track.title || track.file_path.replace(/\\/g, '/').split('/').pop()?.replace(/\.[^.]+$/, '') || t('recap.untitled')
}

function artistOf(track: RecapTrack): string {
  return track.artist || t('recap.unknownArtist')
}

function avatar(artist: RecapArtist): string | null {
  return avatarFor(artist.name) ?? artist.top_track.cover_url
}

function genre(name: string): string {
  return genreDisplayName(name, language.value)
}

function pct(v: number): string {
  return `${Math.round(v * 100)}%`
}

const r = computed(() => props.recap)
const minutesTotal = computed(() => Math.round(r.value.total_secs / 60))
const daysInYear = computed(() => {
  const y = r.value.year
  return (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0 ? 366 : 365
})
const yearDaysPct = computed(() => Math.min(100, Math.round((r.value.days_listened / daysInYear.value) * 100)))

const minutesEquivalent = computed(() => {
  const hours = r.value.total_secs / 3600
  if (hours >= 48) return t('recap.minutes.days', { days: count(Math.floor(hours / 24), 'days') })
  return t('recap.minutes.hours', { hours: count(Math.max(1, Math.round(hours)), 'hours') })
})

const introCovers = computed(() => {
  const covers = recapCovers(r.value, 9)
  return covers.slice(0, covers.length - (covers.length % 3))
})
const varietyCovers = computed(() => {
  const covers = recapCovers(r.value, 8)
  return covers.slice(0, covers.length - (covers.length % 4))
})

const hourBars = computed(() => {
  const max = Math.max(1, ...r.value.hours)
  const peak = peakIndex(r.value.hours)
  return r.value.hours.map((v, h) => ({ h, height: Math.max(6, (v / max) * 100), peak: h === peak }))
})

const monthBars = computed(() => {
  const max = Math.max(1, ...r.value.months)
  const peak = peakIndex(r.value.months)
  return r.value.months.map((v, m) => ({ m, height: Math.max(6, (v / max) * 100), peak: m === peak }))
})
const peakMonth = computed(() => monthName(peakIndex(r.value.months)))

const maxTrackPlays = computed(() => Math.max(1, ...(props.card.tracks ?? []).map(tr => tr.plays)))
const maxArtistSecs = computed(() => Math.max(1, ...(props.card.artists ?? []).map(a => a.secs)))
const maxGenreShare = computed(() => Math.max(0.01, ...(props.card.genres ?? []).map(g => g.share)))

const fullShare = computed(() => (r.value.plays > 0 ? Math.min(1, r.value.full_listens / r.value.plays) : 0))
const RING_R = 72
const RING_C = 2 * Math.PI * RING_R
const ringOffset = computed(() => RING_C * (1 - fullShare.value))

const cardStyle = computed(() => {
  const a = props.card.accent
  if (!a) return {}
  const shift = a.shift ?? 38
  return {
    '--rc-h': `${a.hue}`,
    '--rc-h2': `${(a.hue + shift) % 360}`,
    '--rc-s': `${a.sat}%`,
  }
})

const backdropCover = computed(() => {
  const c = props.card
  return c.track?.cover_url ?? c.album?.top_track.cover_url ?? c.obsession?.track.cover_url
    ?? (c.artist ? avatar(c.artist) : null)
})

const MOMENT_ICON: Record<MomentMode, string> = {
  night: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  morning: '<path d="M12 2v8"/><path d="m4.93 10.93 1.41 1.41"/><path d="M2 18h2"/><path d="M20 18h2"/><path d="m19.07 10.93-1.41 1.41"/><path d="M22 22H2"/><path d="m8 6 4-4 4 4"/><path d="M16 18a4 4 0 0 0-8 0"/>',
  day: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>',
  evening: '<path d="M12 10V2"/><path d="m4.93 10.93 1.41 1.41"/><path d="M2 18h2"/><path d="M20 18h2"/><path d="m19.07 10.93-1.41 1.41"/><path d="M22 22H2"/><path d="m16 6-4 4-4-4"/><path d="M16 18a4 4 0 0 0-8 0"/>',
}

const CHARACTER_ICON: Record<CharacterId, string> = {
  nightOwl: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  fan: '<path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/>',
  explorer: '<circle cx="12" cy="12" r="10"/><polygon points="16.24 7.76 14.12 14.12 7.76 16.24 9.88 9.88 16.24 7.76"/>',
  marathoner: '<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/>',
  energy: '<path d="M2 12h4l3-9 6 18 3-9h4"/>',
  calm: '<path d="M2 6c.6.5 1.2 1 2.5 1C7 7 7 5 9.5 5c2.6 0 2.4 2 5 2 2.5 0 2.5-2 5-2 1.3 0 1.9.5 2.5 1"/><path d="M2 12c.6.5 1.2 1 2.5 1 2.5 0 2.5-2 5-2 2.6 0 2.4 2 5 2 2.5 0 2.5-2 5-2 1.3 0 1.9.5 2.5 1"/><path d="M2 18c.6.5 1.2 1 2.5 1 2.5 0 2.5-2 5-2 2.6 0 2.4 2 5 2 2.5 0 2.5-2 5-2 1.3 0 1.9.5 2.5 1"/>',
  dancer: '<path d="M2 10v3"/><path d="M6 6v11"/><path d="M10 3v18"/><path d="M14 8v7"/><path d="M18 5v13"/><path d="M22 10v3"/>',
  completist: '<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>',
  shuffler: '<path d="M2 18h1.4c1.3 0 2.5-.6 3.3-1.7l6.1-8.6c.7-1.1 2-1.7 3.3-1.7H22"/><path d="m18 2 4 4-4 4"/><path d="M2 6h1.9c1.5 0 2.9.9 3.6 2.2"/><path d="M22 18h-5.9c-1.3 0-2.6-.7-3.3-1.8l-.5-.8"/><path d="m18 14 4 4-4 4"/>',
  melomane: '<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>',
}

const MARATHON_WAVE = [24, 48, 72, 40, 88, 60, 96, 52, 80, 68, 44, 76, 56, 36]

function playRow(url: string | null) {
  if (url) emit('sound', url)
}
</script>

<template>
  <div class="rc" :class="[`tone-${card.tone}`, `kind-${card.kind}`, { paused }]" :style="cardStyle">
    <div class="rcDecor" aria-hidden="true">
      <template v-if="card.tone === 'cover' && backdropCover">
        <img class="rcCoverAura" :src="backdropCover" alt="" />
        <div class="rcCoverVeil"></div>
      </template>
      <div v-if="card.kind === 'intro' && introCovers.length >= 3" class="rcIntroMosaic">
        <div class="rcIntroGrid">
          <RecapCover v-for="(url, i) in introCovers" :key="url" class="rcMosaicCover" :style="{ '--i': i }" :url="url" />
        </div>
      </div>
      <div v-if="card.kind === 'intro'" class="rcGhostYear">{{ recap.year }}</div>
    </div>

    <div class="rcBody">
      <!-- EMPTY -->
      <template v-if="card.kind === 'empty'">
        <div class="rcCenter">
          <div class="rcEmblem s0">
            <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M9 18V5l12-2v13" />
              <circle cx="6" cy="18" r="3" />
              <circle cx="18" cy="16" r="3" />
            </svg>
          </div>
          <div class="rcPill s0">{{ t('recap.title', { year: recap.year }) }}</div>
          <div class="rcTitle s1">{{ t('recap.empty.title') }}</div>
          <p class="rcText s2">{{ t('recap.empty.text') }}</p>
        </div>
      </template>

      <!-- INTRO -->
      <template v-else-if="card.kind === 'intro'">
        <div class="rcMain end">
          <div class="rcPill s0"><span class="rcDot"></span>LFMusic</div>
          <div class="rcDisplay xl s1">{{ t('recap.intro.title', { year: recap.year }) }}</div>
          <p class="rcText s2">{{ t('recap.intro.text') }}</p>
        </div>
        <div class="rcTapHint s3">
          <span>{{ t('recap.intro.hint') }}</span>
          <svg class="rcTapArrow" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M5 12h14" />
            <path d="m12 5 7 7-7 7" />
          </svg>
        </div>
      </template>

      <!-- MINUTES -->
      <template v-else-if="card.kind === 'minutes'">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.minutes.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber xl s1"><CountUp :value="minutesTotal" :locale="locale" /></div>
          <div class="rcUnit s1">{{ unit(minutesTotal, 'minutes') }}</div>
          <div class="rcHighlightPill s2">{{ minutesEquivalent }}</div>
        </div>
        <div class="rcPanel s3">
          <div class="rcPanelHead">
            <span class="rcPanelSub">{{ t('recap.minutes.daysListened', { days: count(recap.days_listened, 'days'), total: daysInYear }) }}</span>
            <span class="rcPanelPct">{{ yearDaysPct }}%</span>
          </div>
          <div class="rcMeter"><span :style="{ '--w': recap.days_listened / daysInYear }"></span></div>
        </div>
      </template>

      <!-- FIRST TRACK -->
      <template v-else-if="card.kind === 'first' && card.track">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.first.kicker') }}</div>
        <div class="rcMain center">
          <div class="rcCoverStage s1">
            <div class="rcCoverHalo"></div>
            <RecapCover class="rcHeroCover" :url="card.track.cover_url" />
          </div>
          <div class="rcTrackTitle center s2">{{ title(card.track) }}</div>
          <div class="rcTrackArtist center s2">{{ artistOf(card.track) }}</div>
        </div>
        <div v-if="recap.first_listen_at" class="rcPanel compact center s3">
          <span class="rcPanelText">{{ t('recap.first.text', { date: dateTime(recap.first_listen_at) }) }}</span>
        </div>
      </template>

      <!-- VARIETY -->
      <template v-else-if="card.kind === 'variety'">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.variety.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber s1"><CountUp :value="recap.unique_tracks" :locale="locale" /></div>
          <div class="rcUnit s1">{{ unit(recap.unique_tracks, 'differentTracks') }}</div>
          <div v-if="varietyCovers.length >= 4" class="rcGrid">
            <RecapCover v-for="(url, i) in varietyCovers" :key="url" class="rcGridCover" :style="{ '--i': i }" :url="url" />
          </div>
        </div>
        <div class="rcBento" :class="recap.unique_albums > 0 ? 'cols3' : 'cols2'">
          <div class="rcTile s3">
            <span class="rcTileVal">{{ num(recap.plays) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.plays') }}</span>
          </div>
          <div class="rcTile s3">
            <span class="rcTileVal">{{ num(recap.unique_artists) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.artists') }}</span>
          </div>
          <div v-if="recap.unique_albums > 0" class="rcTile s3">
            <span class="rcTileVal">{{ num(recap.unique_albums) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.albums') }}</span>
          </div>
        </div>
      </template>

      <!-- TOP TRACK -->
      <template v-else-if="card.kind === 'topTrack' && card.track">
        <div class="rcTopRow s0">
          <div class="rcPill"><span class="rcDot"></span>{{ t('recap.topTrack.kicker') }}</div>
          <span class="rcRankBadge">#1</span>
        </div>
        <div class="rcMain">
          <div class="rcVinylStage s1">
            <div class="rcVinyl" aria-hidden="true">
              <div class="rcVinylLabel"></div>
            </div>
            <RecapCover class="rcHeroCover vinylFront" :url="card.track.cover_url" />
          </div>
          <div class="rcTrackTitle s2">{{ title(card.track) }}</div>
          <div class="rcTrackArtist s2">{{ artistOf(card.track) }}</div>
        </div>
        <div class="rcBento cols2 s3">
          <div class="rcTile">
            <span class="rcTileVal">{{ count(card.track.plays, 'times') }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.played') }}</span>
          </div>
          <div class="rcTile">
            <span class="rcTileVal">{{ formatTotalDuration(card.track.secs) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.time') }}</span>
          </div>
        </div>
      </template>

      <!-- TOP TRACKS -->
      <template v-else-if="card.kind === 'topTracks' && card.tracks">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.topTracks.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.topTracks.title') }}</div>
        <div class="rcList">
          <button
            v-for="(tr, i) in card.tracks"
            :key="tr.file_path"
            type="button"
            class="rcRow"
            :class="{ first: i === 0, playing: tr.snippet_url && playingUrl === tr.snippet_url }"
            :style="{ '--i': i }"
            :disabled="!tr.snippet_url"
            @click="playRow(tr.snippet_url)"
          >
            <span class="rcRank" :class="{ top: i === 0 }">
              <span v-if="tr.snippet_url && playingUrl === tr.snippet_url" class="rcEq"><i></i><i></i><i></i></span>
              <template v-else>{{ i + 1 }}</template>
            </span>
            <RecapCover class="rcRowCover" :url="tr.cover_url" />
            <span class="rcRowText">
              <span class="rcRowTitle">{{ title(tr) }}</span>
              <span class="rcRowSub">{{ artistOf(tr) }}</span>
            </span>
            <span class="rcRowValue">{{ count(tr.plays, 'times') }}</span>
            <span class="rcRowFill" :style="{ '--w': tr.plays / maxTrackPlays }" aria-hidden="true"></span>
          </button>
        </div>
      </template>

      <!-- TOP ARTIST / DISCOVERY -->
      <template v-else-if="(card.kind === 'topArtist' || card.kind === 'discovery') && card.artist">
        <div class="rcTopRow s0">
          <div class="rcPill"><span class="rcDot"></span>{{ t(`recap.${card.kind}.kicker`) }}</div>
          <span v-if="card.kind === 'topArtist'" class="rcRankBadge">#1</span>
        </div>
        <div class="rcMain center">
          <div class="rcAvatarStage s1">
            <span class="rcSonicRing r1" aria-hidden="true"></span>
            <span class="rcSonicRing r2" aria-hidden="true"></span>
            <RecapCover class="rcAvatar" round :url="avatar(card.artist)" />
          </div>
          <div class="rcArtistName s2">{{ card.artist.name }}</div>
          <div class="rcHighlightPill center s2" v-if="card.kind === 'topArtist'">
            {{ t('recap.topArtist.text', { minutes: count(Math.round(card.artist.secs / 60), 'minutes') }) }}
          </div>
          <div class="rcHighlightPill center s2" v-else>
            {{ t('recap.discovery.text', { plays: count(card.artist.plays, 'plays') }) }}
          </div>
        </div>
        <div class="rcMiniCard s3">
          <RecapCover class="rcMiniCover" :url="card.artist.top_track.cover_url" />
          <div class="rcRowText">
            <span class="rcMiniLabel">{{ t('recap.labels.favoriteTrack') }}</span>
            <span class="rcRowTitle">{{ title(card.artist.top_track) }}</span>
          </div>
        </div>
      </template>

      <!-- TOP ARTISTS -->
      <template v-else-if="card.kind === 'topArtists' && card.artists">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.topArtists.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.topArtists.title') }}</div>
        <div class="rcList">
          <button
            v-for="(a, i) in card.artists"
            :key="a.name"
            type="button"
            class="rcRow"
            :class="{ first: i === 0, playing: a.top_track.snippet_url && playingUrl === a.top_track.snippet_url }"
            :style="{ '--i': i }"
            :disabled="!a.top_track.snippet_url"
            @click="playRow(a.top_track.snippet_url)"
          >
            <span class="rcRank" :class="{ top: i === 0 }">
              <span v-if="a.top_track.snippet_url && playingUrl === a.top_track.snippet_url" class="rcEq"><i></i><i></i><i></i></span>
              <template v-else>{{ i + 1 }}</template>
            </span>
            <RecapCover class="rcRowCover" round :url="avatar(a)" />
            <span class="rcRowText">
              <span class="rcRowTitle">{{ a.name }}</span>
              <span class="rcRowSub">{{ count(a.track_count, 'tracks') }}</span>
            </span>
            <span class="rcRowValue">{{ count(Math.round(a.secs / 60), 'minutesShort') }}</span>
            <span class="rcRowFill" :style="{ '--w': a.secs / maxArtistSecs }" aria-hidden="true"></span>
          </button>
        </div>
      </template>

      <!-- TOP ALBUM -->
      <template v-else-if="card.kind === 'topAlbum' && card.album">
        <div class="rcTopRow s0">
          <div class="rcPill"><span class="rcDot"></span>{{ t('recap.topAlbum.kicker') }}</div>
          <span class="rcRankBadge">#1</span>
        </div>
        <div class="rcMain center">
          <div class="rcSleeveStack s1">
            <span class="rcSleeveLayer l2" aria-hidden="true"></span>
            <span class="rcSleeveLayer l1" aria-hidden="true"></span>
            <RecapCover class="rcHeroCover sleeveFront" :url="card.album.top_track.cover_url" />
          </div>
          <div class="rcTrackTitle center s2">{{ card.album.name }}</div>
          <div class="rcTrackArtist center s2">{{ card.album.artist || t('recap.unknownArtist') }}</div>
        </div>
        <div class="rcBento cols2 s3">
          <div class="rcTile">
            <span class="rcTileVal">{{ num(card.album.plays) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.plays') }}</span>
          </div>
          <div class="rcTile">
            <span class="rcTileVal">{{ formatTotalDuration(card.album.secs) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.time') }}</span>
          </div>
        </div>
      </template>

      <!-- GENRES -->
      <template v-else-if="card.kind === 'genres' && card.genres">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.genres.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.genres.title') }}</div>
        <div class="rcMain">
          <div class="rcPanel genresPanel s2">
            <div class="rcGenres">
              <div v-for="(g, i) in card.genres" :key="g.name" class="rcGenre" :class="{ lead: i === 0 }" :style="{ '--i': i }">
                <div class="rcGenreHead">
                  <span class="rcGenreName">{{ genre(g.name) }}</span>
                  <span class="rcGenreShare">{{ pct(g.share) }}</span>
                </div>
                <div class="rcBar"><span :style="{ '--w': g.share / maxGenreShare }"></span></div>
              </div>
            </div>
          </div>
        </div>
        <div v-if="recap.unique_genres > card.genres.length" class="rcFootPill s3">
          {{ t('recap.genres.more', { count: count(recap.unique_genres, 'genres') }) }}
        </div>
      </template>

      <!-- MOMENTS -->
      <template v-else-if="card.kind === 'moments' && card.moment">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.moments.kicker') }}</div>
        <div class="rcMain">
          <div class="rcEmblem small s1">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="MOMENT_ICON[card.moment.mode]"></svg>
          </div>
          <div class="rcTitle large s1">{{ t(`recap.moments.${card.moment.mode}.title`) }}</div>
          <p class="rcText s2" v-if="card.moment.count > 0">
            {{ t(`recap.moments.${card.moment.mode}.text`, { tracks: count(card.moment.count, 'tracks') }) }}
          </p>
          <p class="rcText s2" v-else>{{ t(`recap.moments.${card.moment.mode}.text2`) }}</p>
        </div>
        <div class="rcPanel chartPanel s3">
          <div class="rcChartHead">
            <span class="rcPanelSub">{{ t('recap.moments.peak', { hour: hourLabel(card.moment.peakHour) }) }}</span>
            <span class="rcPeakChip">{{ hourLabel(card.moment.peakHour) }}</span>
          </div>
          <div class="rcBars">
            <span
              v-for="b in hourBars"
              :key="b.h"
              :class="{ peak: b.peak }"
              :style="{ '--hgt': b.height, '--i': b.h }"
            ></span>
          </div>
          <div class="rcAxis"><span>00</span><span>06</span><span>12</span><span>18</span><span>24</span></div>
        </div>
      </template>

      <!-- OBSESSION -->
      <template v-else-if="card.kind === 'obsession' && card.obsession">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.obsession.kicker') }}</div>
        <div class="rcMain center">
          <div class="rcEchoStage s1">
            <span class="rcEcho e2" aria-hidden="true"></span>
            <span class="rcEcho e1" aria-hidden="true"></span>
            <div class="rcHeroWrap">
              <RecapCover class="rcHeroCover" :url="card.obsession.track.cover_url" />
              <span class="rcBadge">x{{ card.obsession.plays }}</span>
            </div>
          </div>
          <div class="rcTrackTitle center s2">{{ title(card.obsession.track) }}</div>
          <div class="rcTrackArtist center s2">{{ artistOf(card.obsession.track) }}</div>
        </div>
        <div class="rcPanel compact center s3">
          <span class="rcPanelText">
            {{ t('recap.obsession.text', { date: day(card.obsession.day), times: count(card.obsession.plays, 'times') }) }}
          </span>
        </div>
      </template>

      <!-- MARATHON -->
      <template v-else-if="card.kind === 'marathon' && card.session">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.marathon.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber marathon s1">
            <CountUp :value="Math.floor(card.session.secs / 3600)" :locale="locale" /><span class="rcNumberUnit">{{ t('recap.units.h') }}</span>
            <CountUp :value="Math.floor((card.session.secs % 3600) / 60)" :locale="locale" /><span class="rcNumberUnit">{{ t('recap.units.min') }}</span>
          </div>
          <p class="rcText s2">{{ t('recap.marathon.text') }}</p>
          <div class="rcPulseWave s2" aria-hidden="true">
            <span v-for="(h, i) in MARATHON_WAVE" :key="i" :style="{ '--hgt': h, '--i': i }"></span>
          </div>
        </div>
        <div class="rcBento cols2 s3">
          <div class="rcTile">
            <span class="rcTileVal">{{ num(card.session.tracks) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.tracksInRow') }}</span>
          </div>
          <div class="rcTile">
            <span class="rcTileVal">{{ day(card.session.day) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.date') }}</span>
          </div>
        </div>
      </template>

      <!-- RHYTHM -->
      <template v-else-if="card.kind === 'rhythm'">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.rhythm.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.rhythm.title', { month: peakMonth }) }}</div>
        <div class="rcMain">
          <div class="rcPanel chartPanel s2">
            <div class="rcBars months">
              <span
                v-for="b in monthBars"
                :key="b.m"
                :class="{ peak: b.peak }"
                :style="{ '--hgt': b.height, '--i': b.m }"
              ></span>
            </div>
            <div class="rcAxis months">
              <span v-for="m in 12" :key="m" :class="{ peak: m - 1 === peakIndex(recap.months) }">{{ monthLetter(m - 1) }}</span>
            </div>
          </div>
        </div>
        <div class="rcBento s3" :class="recap.busiest_day ? 'cols2' : 'cols1'">
          <div class="rcTile">
            <span class="rcTileVal">{{ count(recap.longest_streak, 'days') }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.streak') }}</span>
          </div>
          <div v-if="recap.busiest_day" class="rcTile">
            <span class="rcTileVal">{{ formatTotalDuration(recap.busiest_day.secs) }}</span>
            <span class="rcTileLbl">{{ day(recap.busiest_day.day) }}</span>
          </div>
        </div>
      </template>

      <!-- COLLECTION -->
      <template v-else-if="card.kind === 'collection'">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.collection.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.collection.title') }}</div>
        <div class="rcMain">
          <div class="rcFacts">
            <div v-if="recap.likes > 0" class="rcFactCard" style="--i: 0">
              <span class="rcFactIcon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/>
                </svg>
              </span>
              <span class="rcFactValue">+<CountUp :value="recap.likes" :locale="locale" :duration="1100" /></span>
              <span class="rcFactLabel">{{ t('recap.collection.likes', { tracks: unit(recap.likes, 'tracks') }) }}</span>
            </div>
            <div v-if="recap.playlists_created > 0" class="rcFactCard" style="--i: 1">
              <span class="rcFactIcon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15V6"/><path d="M18.5 18a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5Z"/><path d="M12 12H3"/><path d="M16 6H3"/><path d="M12 18H3"/>
                </svg>
              </span>
              <span class="rcFactValue"><CountUp :value="recap.playlists_created" :locale="locale" :duration="900" /></span>
              <span class="rcFactLabel">{{ unit(recap.playlists_created, 'newPlaylists') }}</span>
            </div>
            <div v-if="recap.downloads > 0" class="rcFactCard" style="--i: 2">
              <span class="rcFactIcon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/>
                </svg>
              </span>
              <span class="rcFactValue"><CountUp :value="recap.downloads" :locale="locale" :duration="1100" /></span>
              <span class="rcFactLabel">{{ unit(recap.downloads, 'downloads') }}</span>
            </div>
          </div>
        </div>
        <div v-if="recap.top_playlist" class="rcMiniCard s3">
          <RecapCover class="rcMiniCover" :url="recap.top_playlist.cover_url" />
          <div class="rcRowText">
            <span class="rcMiniLabel">{{ t('recap.collection.playlist') }}</span>
            <span class="rcRowTitle">{{ recap.top_playlist.kind === 'favorites' ? t('recap.collection.favorites') : recap.top_playlist.name }}</span>
          </div>
        </div>
      </template>

      <!-- HABITS -->
      <template v-else-if="card.kind === 'habits'">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.habits.kicker') }}</div>
        <div class="rcMain center">
          <div class="rcGaugeWrap s1">
            <svg class="rcGauge" viewBox="0 0 168 168" aria-hidden="true">
              <circle class="rcGaugeTrack" cx="84" cy="84" :r="RING_R" />
              <circle
                class="rcGaugeFill"
                cx="84"
                cy="84"
                :r="RING_R"
                :stroke-dasharray="RING_C"
                :style="{ '--c': RING_C, '--off': ringOffset }"
              />
            </svg>
            <div class="rcGaugeCenter">
              <div class="rcGaugeNum"><CountUp :value="Math.round(fullShare * 100)" :locale="locale" suffix="%" /></div>
            </div>
          </div>
          <p class="rcText center s2">{{ t('recap.habits.full') }}</p>
        </div>
        <div class="rcBento cols2 s3">
          <div v-if="recap.shuffle_share >= 0.05" class="rcTile">
            <span class="rcTileVal">{{ pct(recap.shuffle_share) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.shuffle') }}</span>
          </div>
          <div class="rcTile">
            <span class="rcTileVal">{{ num(recap.skips) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.skips') }}</span>
          </div>
          <div v-if="recap.search_starts > 0" class="rcTile">
            <span class="rcTileVal">{{ num(recap.search_starts) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.search') }}</span>
          </div>
          <div v-if="recap.mobile_share >= 0.05" class="rcTile">
            <span class="rcTileVal">{{ pct(recap.mobile_share) }}</span>
            <span class="rcTileLbl">{{ t('recap.labels.mobile') }}</span>
          </div>
        </div>
      </template>

      <!-- CHARACTER -->
      <template v-else-if="card.kind === 'character' && card.character">
        <div class="rcPill s0"><span class="rcDot"></span>{{ t('recap.character.kicker') }}</div>
        <div class="rcMain center">
          <div class="rcEmblem hero s1">
            <svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" v-html="CHARACTER_ICON[card.character.id]"></svg>
          </div>
          <div class="rcDisplay center s1">{{ t(`recap.character.${card.character.id}.title`) }}</div>
          <p class="rcText center s2">{{ t(`recap.character.${card.character.id}.desc`) }}</p>
          <div class="rcChips s3" v-if="card.character.traits.length > 0">
            <span v-for="trait in card.character.traits" :key="trait" class="rcChip">
              <span class="rcChipDot"></span>
              {{ t(`recap.character.${trait}.title`) }}
            </span>
          </div>
        </div>
      </template>

      <!-- OUTRO / SUMMARY POSTER -->
      <template v-else-if="card.kind === 'outro'">
        <div class="rcTopRow s0">
          <div class="rcPill"><span class="rcDot"></span>LFMusic</div>
          <span class="rcYearBadge">{{ recap.year }}</span>
        </div>
        <div class="rcTitle s1">{{ t('recap.outro.title') }}</div>
        <div class="rcMain outroMain">
          <div class="rcSummaryHero s2" v-if="recap.top_tracks[0]">
            <RecapCover class="rcSummaryCover" :url="recap.top_tracks[0].cover_url" />
            <div class="rcRowText">
              <span class="rcMiniLabel">{{ t('recap.outro.track') }}</span>
              <span class="rcSummaryTrack">{{ title(recap.top_tracks[0]) }}</span>
              <span class="rcRowSub">{{ artistOf(recap.top_tracks[0]) }}</span>
            </div>
          </div>
          <div class="rcBento cols2 s2">
            <div class="rcTile">
              <span class="rcTileVal accent">{{ num(minutesTotal) }}</span>
              <span class="rcTileLbl">{{ t('recap.outro.minutes') }}</span>
            </div>
            <div v-if="recap.top_artists[0]" class="rcTile">
              <span class="rcTileVal">{{ recap.top_artists[0].name }}</span>
              <span class="rcTileLbl">{{ t('recap.outro.artist') }}</span>
            </div>
            <div v-if="recap.top_genres[0]" class="rcTile">
              <span class="rcTileVal">{{ genre(recap.top_genres[0].name) }}</span>
              <span class="rcTileLbl">{{ t('recap.outro.genre') }}</span>
            </div>
            <div v-if="card.character" class="rcTile">
              <span class="rcTileVal">{{ t(`recap.character.${card.character.id}.title`) }}</span>
              <span class="rcTileLbl">{{ t('recap.outro.character') }}</span>
            </div>
          </div>
        </div>
        <button type="button" class="rcButton s3" @click="emit('replay')">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
            <path d="M3 3v5h5" />
          </svg>
          <span>{{ t('recap.outro.replay') }}</span>
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.rc {
  --h: var(--rc-h, var(--accent-h, 255));
  --h2: var(--rc-h2, 295);
  --s: var(--rc-s, var(--accent-s, 52%));
  --display: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  --accent-soft: hsl(var(--h), min(var(--s), 76%), 74%);
  --accent-bright: hsl(var(--h), min(var(--s), 82%), 66%);
  --accent-alt: hsl(var(--h2), min(var(--s), 74%), 72%);
  --surface: hsla(0, 0%, 4%, 0.42);
  --surface-strong: hsla(0, 0%, 4%, 0.56);
  --ease-out: cubic-bezier(0.16, 1, 0.3, 1);
  position: absolute;
  inset: 0;
  z-index: 1;
  overflow: hidden;
  color: hsla(0, 0%, 100%, 0.97);
  user-select: none;
  background: transparent;
  container-type: size;
}

.rcDecor {
  position: absolute;
  inset: 0;
  pointer-events: none;
  overflow: hidden;
}

.rcCoverAura {
  position: absolute;
  inset: -18%;
  width: 136%;
  height: 136%;
  object-fit: cover;
  filter: blur(56px) saturate(1.2);
  opacity: 0.32;
  mix-blend-mode: screen;
  animation: rcBreath 12s ease-in-out infinite alternate;
}
.rcCoverVeil {
  position: absolute;
  inset: 0;
  background: linear-gradient(
    180deg,
    hsla(var(--h), 24%, 6%, 0.35) 0%,
    hsla(var(--h), 24%, 6%, 0.12) 42%,
    hsla(var(--h), 24%, 5%, 0.65) 100%
  );
}
@keyframes rcBreath {
  from { transform: scale(1) translate3d(0, 0, 0); }
  to { transform: scale(1.08) translate3d(0, -1.5%, 0); }
}

.rcIntroMosaic {
  position: absolute;
  top: -20px;
  left: -14%;
  right: -14%;
  height: 62%;
  -webkit-mask-image: linear-gradient(180deg, rgba(0, 0, 0, 0.82) 15%, rgba(0, 0, 0, 0.48) 56%, transparent 94%);
  mask-image: linear-gradient(180deg, rgba(0, 0, 0, 0.82) 15%, rgba(0, 0, 0, 0.48) 56%, transparent 94%);
}
.rcIntroGrid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
  transform: rotate(-8deg) scale(1.06);
  animation: rcDrift 18s ease-in-out infinite alternate;
}
@keyframes rcDrift {
  from { transform: rotate(-8deg) scale(1.06) translateY(0); }
  to { transform: rotate(-6.5deg) scale(1.09) translateY(-12px); }
}
.rcMosaicCover {
  aspect-ratio: 1;
  border-radius: 12px;
  box-shadow: 0 12px 30px hsla(0, 0%, 0%, 0.4);
  animation: rcFade 0.8s var(--ease-out) both;
  animation-delay: calc(var(--i) * 0.05s);
}
.rcGhostYear {
  position: absolute;
  right: -14px;
  bottom: 18%;
  font-family: var(--display);
  font-size: clamp(110px, 40cqw, 190px);
  font-weight: 900;
  line-height: 0.82;
  letter-spacing: -0.06em;
  color: hsla(0, 0%, 100%, 0.06);
  pointer-events: none;
}

.rcBody {
  position: relative;
  z-index: 1;
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 68px 24px 26px;
  box-sizing: border-box;
}
.rcMain {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
}
.rcMain.end {
  justify-content: flex-end;
  padding-bottom: 18px;
}
.rcMain.center {
  align-items: center;
  text-align: center;
}
.rcMain.outroMain {
  gap: 12px;
}
.rcCenter {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  gap: 12px;
}

/* Entrance choreography */
.s0, .s1, .s2, .s3 {
  animation: rcRise 0.62s var(--ease-out) both;
}
.s1 { animation-delay: 0.07s; }
.s2 { animation-delay: 0.16s; }
.s3 { animation-delay: 0.26s; }
@keyframes rcRise {
  from { opacity: 0; transform: translateY(14px) scale(0.985); }
  to { opacity: 1; transform: none; }
}
@keyframes rcFade {
  from { opacity: 0; transform: scale(0.95); }
  to { opacity: 1; transform: none; }
}

/* Kicker pill & badges */
.rcTopRow {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.rcPill {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 5px 12px;
  border-radius: 999px;
  background: var(--surface);
  backdrop-filter: blur(14px);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.11em;
  text-transform: uppercase;
  color: hsla(0, 0%, 100%, 0.9);
}
.rcCenter .rcPill {
  align-self: center;
}
.rcDot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent-soft);
  box-shadow: 0 0 10px var(--accent-bright);
  flex-shrink: 0;
}
.rcRankBadge,
.rcYearBadge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 26px;
  padding: 0 11px;
  border-radius: 999px;
  font-family: var(--display);
  font-size: 12px;
  font-weight: 800;
  font-variant-numeric: tabular-nums;
  color: #fff;
  background: hsla(var(--h), min(var(--s), 75%), 52%, 0.65);
  backdrop-filter: blur(12px);
}

/* Typography */
.rcDisplay {
  margin-top: 12px;
  font-family: var(--display);
  font-size: clamp(34px, 12cqw, 56px);
  line-height: 1.02;
  font-weight: 900;
  letter-spacing: -0.03em;
  text-wrap: balance;
  text-shadow: 0 8px 32px hsla(0, 0%, 0%, 0.35);
}
.rcDisplay.xl {
  font-size: clamp(40px, 14cqw, 64px);
}
.rcDisplay.center {
  text-align: center;
}
.rcTitle {
  margin-top: 10px;
  font-family: var(--display);
  font-size: clamp(22px, 6.8cqw, 30px);
  line-height: 1.12;
  font-weight: 800;
  letter-spacing: -0.02em;
  text-wrap: balance;
  text-shadow: 0 4px 20px hsla(0, 0%, 0%, 0.28);
}
.rcTitle.large {
  font-size: clamp(30px, 10cqw, 44px);
  line-height: 1.04;
}
.rcNumber {
  font-family: var(--display);
  font-size: clamp(54px, 19.5cqw, 86px);
  line-height: 0.94;
  font-weight: 900;
  letter-spacing: -0.04em;
  text-shadow: 0 10px 36px hsla(0, 0%, 0%, 0.35);
}
.rcNumber.xl {
  font-size: clamp(58px, 21cqw, 92px);
}
.rcNumber.marathon {
  font-size: clamp(46px, 16.5cqw, 76px);
}
.rcNumberUnit {
  font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  font-size: 0.34em;
  font-weight: 700;
  letter-spacing: 0;
  margin: 0 0.42em 0 0.14em;
  color: hsla(0, 0%, 100%, 0.64);
}
.rcUnit {
  margin-top: 8px;
  font-size: 18px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.8);
}
.rcText {
  margin: 12px 0 0;
  max-width: 330px;
  font-size: 15px;
  line-height: 1.5;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.76);
  text-wrap: pretty;
}
.rcText.center {
  margin-left: auto;
  margin-right: auto;
  text-align: center;
}

.rcHighlightPill {
  align-self: flex-start;
  margin-top: 16px;
  padding: 9px 15px;
  border-radius: 14px;
  background: var(--surface);
  backdrop-filter: blur(16px);
  font-size: 14px;
  font-weight: 600;
  line-height: 1.35;
  color: hsla(0, 0%, 100%, 0.92);
}
.rcHighlightPill.center {
  align-self: center;
}

.rcTapHint {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 9px 16px;
  border-radius: 999px;
  background: var(--surface);
  backdrop-filter: blur(16px);
  font-size: 13px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.88);
}
.rcTapArrow {
  animation: rcNudge 1.8s ease-in-out infinite;
}
@keyframes rcNudge {
  0%, 100% { transform: translateX(0); }
  50% { transform: translateX(4px); }
}

/* Frosted panels & bento tiles */
.rcPanel {
  padding: 15px 16px;
  border-radius: 18px;
  background: var(--surface);
  backdrop-filter: blur(18px);
}
.rcPanel.compact {
  padding: 12px 16px;
  border-radius: 14px;
}
.rcPanel.center {
  text-align: center;
}
.rcPanelHead,
.rcChartHead {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 10px;
}
.rcPanelSub {
  font-size: 12.5px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.72);
}
.rcPanelText {
  font-size: 13.5px;
  font-weight: 500;
  line-height: 1.45;
  color: hsla(0, 0%, 100%, 0.84);
}
.rcPanelPct,
.rcPeakChip {
  flex-shrink: 0;
  padding: 3px 9px;
  border-radius: 999px;
  font-size: 11.5px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: #fff;
  background: hsla(var(--h), min(var(--s), 75%), 52%, 0.45);
}

.rcMeter {
  height: 7px;
  border-radius: 999px;
  background: hsla(0, 0%, 100%, 0.12);
  overflow: hidden;
}
.rcMeter span {
  display: block;
  height: 100%;
  width: calc(var(--w) * 100%);
  border-radius: inherit;
  background: linear-gradient(90deg, var(--accent-bright), var(--accent-alt));
  transform-origin: left;
  animation: rcGrow 1s var(--ease-out) 0.4s both;
}

.rcBento {
  display: grid;
  gap: 8px;
}
.rcBento.cols1 { grid-template-columns: 1fr; }
.rcBento.cols2 { grid-template-columns: repeat(2, minmax(0, 1fr)); }
.rcBento.cols3 { grid-template-columns: repeat(3, minmax(0, 1fr)); }

.rcTile {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 4px;
  min-width: 0;
  padding: 13px 14px;
  border-radius: 16px;
  background: var(--surface);
  backdrop-filter: blur(18px);
}
.rcTileVal {
  font-family: var(--display);
  font-size: clamp(16px, 5cqw, 20px);
  font-weight: 800;
  line-height: 1.15;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rcTileVal.accent {
  color: var(--accent-soft);
}
.rcTileLbl {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: hsla(0, 0%, 100%, 0.56);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Cover stages: First, Top Track (vinyl), Top Album (sleeve stack), Obsession (echo) */
.rcCoverStage {
  position: relative;
  width: min(76%, 40cqh);
}
.rcCoverHalo {
  position: absolute;
  inset: -12%;
  border-radius: 26px;
  background: radial-gradient(circle, hsla(var(--h), min(var(--s), 80%), 60%, 0.42), transparent 70%);
  filter: blur(22px);
  pointer-events: none;
}
.rcHeroCover {
  position: relative;
  z-index: 1;
  width: 100%;
  aspect-ratio: 1;
  border-radius: 16px;
  box-shadow: 0 22px 54px hsla(0, 0%, 0%, 0.55);
}

/* Vinyl disc behind topTrack cover */
.rcVinylStage {
  position: relative;
  width: min(72%, 38cqh);
  margin-left: 2%;
}
.rcVinyl {
  position: absolute;
  top: 5%;
  right: -28%;
  width: 90%;
  aspect-ratio: 1;
  border-radius: 50%;
  background:
    radial-gradient(circle at center,
      hsl(var(--h), min(var(--s), 20%), 12%) 0%,
      hsl(var(--h), min(var(--s), 20%), 12%) 18%,
      transparent 18.5%),
    repeating-radial-gradient(circle at center,
      hsla(0, 0%, 100%, 0.06) 0px,
      hsla(0, 0%, 100%, 0.06) 1px,
      hsla(0, 0%, 4%, 0.96) 2px,
      hsla(0, 0%, 8%, 0.96) 4px);
  box-shadow: 0 14px 36px hsla(0, 0%, 0%, 0.55);
  display: flex;
  align-items: center;
  justify-content: center;
  animation: rcSpin 14s linear infinite;
}
.rcVinylLabel {
  width: 34%;
  height: 34%;
  border-radius: 50%;
  background: radial-gradient(circle at 35% 35%, var(--accent-soft), hsl(var(--h), min(var(--s), 70%), 38%));
  position: relative;
}
.rcVinylLabel::after {
  content: '';
  position: absolute;
  inset: 38%;
  border-radius: 50%;
  background: hsla(0, 0%, 6%, 0.95);
}
@keyframes rcSpin {
  to { transform: rotate(360deg); }
}

/* Layered sleeve stack for Top Album */
.rcSleeveStack {
  position: relative;
  width: min(74%, 38cqh);
  margin-top: 12px;
}
.rcSleeveLayer {
  position: absolute;
  inset: 0;
  border-radius: 16px;
  background: hsla(0, 0%, 100%, 0.08);
  backdrop-filter: blur(8px);
}
.rcSleeveLayer.l1 {
  transform: translateY(-10px) scale(0.93);
  background: hsla(var(--h), min(var(--s), 60%), 64%, 0.2);
}
.rcSleeveLayer.l2 {
  transform: translateY(-20px) scale(0.86);
  background: hsla(var(--h2), min(var(--s), 55%), 58%, 0.14);
}

/* Echo frames for Obsession */
.rcEchoStage {
  position: relative;
  width: min(74%, 38cqh);
}
.rcEcho {
  position: absolute;
  inset: 0;
  border-radius: 16px;
  background: hsla(var(--h), min(var(--s), 70%), 65%, 0.16);
}
.rcEcho.e1 {
  transform: translate(-10px, 8px) rotate(-3.5deg);
}
.rcEcho.e2 {
  transform: translate(11px, -8px) rotate(3.5deg);
  background: hsla(var(--h2), min(var(--s), 65%), 62%, 0.14);
}
.rcHeroWrap {
  position: relative;
  z-index: 1;
  width: 100%;
}
.rcBadge {
  position: absolute;
  left: 12px;
  bottom: 12px;
  z-index: 2;
  padding: 6px 13px;
  border-radius: 999px;
  font-family: var(--display);
  font-size: 14px;
  font-weight: 800;
  font-variant-numeric: tabular-nums;
  color: #fff;
  background: hsla(var(--h), min(var(--s), 80%), 48%, 0.86);
  backdrop-filter: blur(12px);
  box-shadow: 0 8px 20px hsla(0, 0%, 0%, 0.4);
}

.rcTrackTitle {
  margin-top: 20px;
  max-width: 100%;
  font-family: var(--display);
  font-size: clamp(21px, 6.8cqw, 28px);
  line-height: 1.14;
  font-weight: 800;
  letter-spacing: -0.02em;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  text-shadow: 0 4px 18px hsla(0, 0%, 0%, 0.35);
}
.rcTrackTitle.center,
.rcTrackArtist.center {
  text-align: center;
}
.rcTrackArtist {
  margin-top: 5px;
  max-width: 100%;
  font-size: 15px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.7);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Artist avatar with pulsing sonic rings */
.rcAvatarStage {
  position: relative;
  width: min(58%, 35cqh);
  aspect-ratio: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.rcAvatar {
  position: relative;
  z-index: 1;
  width: 100%;
  height: 100%;
  box-shadow: 0 22px 54px hsla(0, 0%, 0%, 0.55);
}
.rcSonicRing {
  position: absolute;
  inset: -12%;
  border-radius: 50%;
  background: radial-gradient(circle, transparent 58%, hsla(var(--h), min(var(--s), 78%), 70%, 0.24) 72%, transparent 80%);
  animation: rcPulseRing 4.2s ease-out infinite;
}
.rcSonicRing.r2 {
  animation-delay: -2.1s;
}
@keyframes rcPulseRing {
  0% { transform: scale(0.86); opacity: 0; }
  35% { opacity: 0.9; }
  100% { transform: scale(1.36); opacity: 0; }
}
.rcArtistName {
  margin-top: 22px;
  max-width: 100%;
  font-family: var(--display);
  font-size: clamp(26px, 8.8cqw, 40px);
  line-height: 1.06;
  font-weight: 900;
  letter-spacing: -0.025em;
  overflow-wrap: anywhere;
  text-shadow: 0 6px 26px hsla(0, 0%, 0%, 0.35);
}

/* Favorite track / Top playlist mini-card */
.rcMiniCard {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border-radius: 16px;
  background: var(--surface);
  backdrop-filter: blur(18px);
}
.rcMiniCover {
  width: 44px;
  height: 44px;
  border-radius: 9px;
}
.rcMiniLabel {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--accent-soft);
}

/* Variety cover grid */
.rcGrid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
  margin-top: 22px;
}
.rcGridCover {
  aspect-ratio: 1;
  border-radius: 10px;
  box-shadow: 0 10px 24px hsla(0, 0%, 0%, 0.35);
  animation: rcRise 0.55s var(--ease-out) both;
  animation-delay: calc(0.22s + var(--i) * 0.045s);
}

/* Top 5 interactive card list */
.rcList {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 8px;
  margin-top: 14px;
}
.rcRow {
  position: relative;
  overflow: hidden;
  display: flex;
  align-items: center;
  gap: 11px;
  width: 100%;
  height: 60px;
  padding: 0 13px 0 10px;
  border-radius: 15px;
  text-align: left;
  color: inherit;
  background: var(--surface);
  backdrop-filter: blur(16px);
  cursor: pointer;
  transition: background-color var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
  animation: rcRise 0.5s var(--ease-out) both;
  animation-delay: calc(0.16s + var(--i) * 0.055s);
}
.rcRow.first {
  background: linear-gradient(120deg, hsla(var(--h), min(var(--s), 75%), 52%, 0.34), var(--surface-strong));
}
.rcRow:disabled {
  cursor: default;
}
.rcRow:not(:disabled):hover {
  background: hsla(0, 0%, 100%, 0.12);
}
.rcRow.playing {
  background: hsla(var(--h), min(var(--s), 75%), 54%, 0.34);
}
.rcRowFill {
  position: absolute;
  left: 0;
  bottom: 0;
  height: 3px;
  width: calc(var(--w) * 100%);
  background: linear-gradient(90deg, var(--accent-bright), var(--accent-alt));
  opacity: 0.72;
  transform-origin: left;
  animation: rcGrow 0.9s var(--ease-out) both;
  animation-delay: calc(0.32s + var(--i) * 0.06s);
}
.rcRank {
  width: 22px;
  flex-shrink: 0;
  display: flex;
  justify-content: center;
  font-family: var(--display);
  font-size: 13px;
  font-weight: 800;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.5);
}
.rcRank.top {
  color: var(--accent-soft);
  font-size: 15px;
}
.rcEq {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 13px;
}
.rcEq i {
  width: 3px;
  height: 100%;
  border-radius: 1.5px;
  background: var(--accent-soft);
  animation: rcEq 0.85s var(--ease-smooth-in-out) infinite alternate;
}
.rcEq i:nth-child(2) { animation-delay: -0.28s; }
.rcEq i:nth-child(3) { animation-delay: -0.56s; }
@keyframes rcEq {
  from { transform: scaleY(0.28); }
  to { transform: scaleY(1); }
}
.rcRowCover {
  width: 42px;
  height: 42px;
  border-radius: 8px;
}
.rcRowText {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.rcRowTitle, .rcRowSub {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rcRowTitle {
  font-size: 14px;
  font-weight: 700;
}
.rcRowSub {
  font-size: 12px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.6);
}
.rcRowValue {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.65);
}

/* Genres */
.genresPanel {
  padding: 18px 18px 20px;
}
.rcGenres {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.rcGenre {
  animation: rcRise 0.52s var(--ease-out) both;
  animation-delay: calc(0.18s + var(--i) * 0.07s);
}
.rcGenreHead {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}
.rcGenreName {
  min-width: 0;
  font-size: 16px;
  font-weight: 700;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rcGenre.lead .rcGenreName {
  font-family: var(--display);
  font-size: 19px;
  font-weight: 800;
}
.rcGenreShare {
  font-size: 14px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.8);
}
.rcGenre.lead .rcGenreShare {
  color: var(--accent-soft);
}
.rcBar {
  margin-top: 8px;
  height: 7px;
  border-radius: 999px;
  background: hsla(0, 0%, 100%, 0.1);
  overflow: hidden;
}
.rcBar span {
  display: block;
  height: 100%;
  width: calc(var(--w) * 100%);
  border-radius: inherit;
  background: linear-gradient(90deg, var(--accent-bright), var(--accent-alt));
  transform-origin: left;
  animation: rcGrow 0.9s var(--ease-out) both;
}
.rcGenre .rcBar span {
  animation-delay: calc(0.32s + var(--i) * 0.07s);
}
@keyframes rcGrow {
  from { transform: scaleX(0); }
  to { transform: scaleX(1); }
}
.rcFootPill {
  align-self: flex-start;
  padding: 7px 14px;
  border-radius: 999px;
  background: var(--surface);
  backdrop-filter: blur(14px);
  font-size: 12.5px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.76);
}

/* Charts: Moments (24h) & Rhythm (12m) */
.chartPanel {
  padding: 16px 16px 14px;
}
.rcBars {
  display: flex;
  align-items: flex-end;
  gap: 3px;
  height: 92px;
}
.rcBars.months {
  height: min(176px, 26cqh);
  gap: 6px;
}
.rcBars span {
  flex: 1;
  height: calc(var(--hgt) * 1%);
  border-radius: 4px;
  background: hsla(0, 0%, 100%, 0.22);
  transform-origin: bottom;
  animation: rcGrowUp 0.65s var(--ease-out) both;
  animation-delay: calc(0.28s + var(--i) * 0.015s);
}
.rcBars.months span {
  border-radius: 5px;
  animation-delay: calc(0.28s + var(--i) * 0.03s);
}
.rcBars span.peak {
  background: linear-gradient(180deg, var(--accent-alt), var(--accent-bright));
  box-shadow: 0 0 18px hsla(var(--h), min(var(--s), 85%), 62%, 0.48);
}
@keyframes rcGrowUp {
  from { transform: scaleY(0); }
  to { transform: scaleY(1); }
}
.rcAxis {
  display: flex;
  justify-content: space-between;
  margin-top: 9px;
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.45);
}
.rcAxis.months span {
  flex: 1;
  text-align: center;
}
.rcAxis.months span.peak {
  color: var(--accent-soft);
  font-weight: 800;
}

/* Marathon decorative pulse wave */
.rcPulseWave {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 46px;
  margin-top: 26px;
}
.rcPulseWave span {
  width: 6px;
  height: calc(var(--hgt) * 1%);
  border-radius: 999px;
  background: linear-gradient(180deg, var(--accent-soft), hsla(var(--h), min(var(--s), 70%), 55%, 0.35));
  animation: rcWaveBeat 2.2s ease-in-out infinite alternate;
  animation-delay: calc(var(--i) * -0.14s);
}
@keyframes rcWaveBeat {
  from { transform: scaleY(0.55); opacity: 0.5; }
  to { transform: scaleY(1); opacity: 1; }
}

/* Collection Bento Facts */
.rcFacts {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.rcFactCard {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 16px;
  border-radius: 18px;
  background: var(--surface);
  backdrop-filter: blur(18px);
  animation: rcRise 0.55s var(--ease-out) both;
  animation-delay: calc(0.18s + var(--i) * 0.08s);
}
.rcFactIcon {
  width: 40px;
  height: 40px;
  border-radius: 12px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--accent-soft);
  background: hsla(var(--h), min(var(--s), 70%), 55%, 0.2);
}
.rcFactValue {
  font-family: var(--display);
  font-size: clamp(24px, 8cqw, 34px);
  line-height: 1;
  font-weight: 800;
  letter-spacing: -0.025em;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}
.rcFactLabel {
  font-size: 13.5px;
  font-weight: 500;
  line-height: 1.3;
  color: hsla(0, 0%, 100%, 0.72);
}

/* Habits SVG Ring Gauge */
.rcGaugeWrap {
  position: relative;
  width: min(56%, 30cqh);
  aspect-ratio: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.rcGauge {
  width: 100%;
  height: 100%;
  transform: rotate(-90deg);
}
.rcGaugeTrack {
  fill: var(--surface);
  stroke: hsla(0, 0%, 100%, 0.12);
  stroke-width: 10;
}
.rcGaugeFill {
  fill: none;
  stroke: var(--accent-soft);
  stroke-width: 10;
  stroke-linecap: round;
  stroke-dashoffset: var(--off);
  filter: drop-shadow(0 0 10px hsla(var(--h), min(var(--s), 80%), 62%, 0.5));
  animation: rcRingDraw 1.25s var(--ease-out) 0.25s both;
}
@keyframes rcRingDraw {
  from { stroke-dashoffset: var(--c); }
  to { stroke-dashoffset: var(--off); }
}
.rcGaugeCenter {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.rcGaugeNum {
  font-family: var(--display);
  font-size: clamp(32px, 11cqw, 46px);
  font-weight: 900;
  letter-spacing: -0.03em;
  line-height: 1;
}

/* Emblem medallions (Empty, Moments, Character) */
.rcEmblem {
  width: 58px;
  height: 58px;
  border-radius: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--accent-soft);
  background: var(--surface);
  backdrop-filter: blur(18px);
}
.rcEmblem.small {
  width: 46px;
  height: 46px;
  border-radius: 14px;
  margin-bottom: 14px;
}
.rcEmblem.hero {
  width: 78px;
  height: 78px;
  border-radius: 24px;
  margin-bottom: 10px;
  color: #fff;
  background: linear-gradient(145deg, hsla(var(--h), min(var(--s), 75%), 56%, 0.42), var(--surface-strong));
  box-shadow: 0 16px 40px hsla(0, 0%, 0%, 0.42);
}

.rcChips {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
  margin-top: 22px;
}
.rcChip {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 7px 14px;
  border-radius: 999px;
  font-size: 13px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.92);
  background: var(--surface);
  backdrop-filter: blur(14px);
}
.rcChipDot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent-alt);
}

/* Outro poster card */
.rcSummaryHero {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 12px;
  border-radius: 18px;
  background: linear-gradient(125deg, hsla(var(--h), min(var(--s), 72%), 52%, 0.34), var(--surface-strong));
  backdrop-filter: blur(18px);
}
.rcSummaryCover {
  width: 74px;
  height: 74px;
  border-radius: 11px;
  box-shadow: 0 10px 26px hsla(0, 0%, 0%, 0.42);
}
.rcSummaryTrack {
  font-family: var(--display);
  font-size: 18px;
  font-weight: 800;
  line-height: 1.2;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.rcButton {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 9px;
  width: 100%;
  height: 48px;
  margin-top: 14px;
  border-radius: 999px;
  font-size: 14px;
  font-weight: 700;
  color: hsl(var(--h), min(var(--s), 45%), 14%);
  background: hsla(0, 0%, 100%, 0.96);
  cursor: pointer;
  transition: background-color var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
}
.rcButton:hover {
  background: hsla(0, 0%, 100%, 0.84);
}
.rcButton:active {
  transform: scale(0.985);
}

/* Pause continuous motion while story is held/paused */
.rc.paused .rcVinyl,
.rc.paused .rcSonicRing,
.rc.paused .rcPulseWave span,
.rc.paused .rcCoverAura,
.rc.paused .rcIntroGrid,
.rc.paused .rcTapArrow,
.rc.paused .rcEq i {
  animation-play-state: paused;
}

@media (max-height: 680px) {
  .rcBody { padding: 58px 20px 20px; }
  .rcRow { height: 52px; }
  .rcRowCover { width: 36px; height: 36px; }
  .rcTile { padding: 10px 12px; }
  .rcFactCard { padding: 11px 13px; }
  .rcGenres { gap: 13px; }
  .rcSummaryCover { width: 62px; height: 62px; }
  .rcButton { height: 44px; }
}

@media (prefers-reduced-motion: reduce) {
  .rcVinyl,
  .rcSonicRing,
  .rcPulseWave span,
  .rcCoverAura,
  .rcIntroGrid,
  .rcTapArrow,
  .rcEq i {
    animation: none;
  }
}
</style>
