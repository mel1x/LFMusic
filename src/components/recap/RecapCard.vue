<script setup lang="ts">
import { computed } from 'vue'
import type { RecapArtist, RecapTrack, YearRecap } from '../../composables/useYearRecap'
import { useI18n } from '../../composables/useI18n'
import { useExternalSources } from '../../composables/useExternalSources'
import { formatTotalDuration, pluralizeRu } from '../../utils/format'
import { genreDisplayName } from '../../utils/genres'
import { peakIndex, recapCovers, type RecapCard } from './recapCards'
import CountUp from './CountUp.vue'
import RecapCover from './RecapCover.vue'

// One story card. Visual language follows the rest of the app: a dark surface
// with a single light of the cover's accent (or the cover itself, blurred),
// covers as the main material, and quiet type with clear hierarchy.

const props = defineProps<{ card: RecapCard; recap: YearRecap; playingUrl: string | null }>()
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

const minutesEquivalent = computed(() => {
  const hours = r.value.total_secs / 3600
  if (hours >= 48) return t('recap.minutes.days', { days: count(Math.floor(hours / 24), 'days') })
  return t('recap.minutes.hours', { hours: count(Math.max(1, Math.round(hours)), 'hours') })
})

// Whole rows only: 3 per row in the intro wall, 4 in the variety grid.
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
  return r.value.hours.map((v, h) => ({ h, height: Math.max(3, (v / max) * 100), peak: h === peak }))
})

const monthBars = computed(() => {
  const max = Math.max(1, ...r.value.months)
  const peak = peakIndex(r.value.months)
  return r.value.months.map((v, m) => ({ m, height: Math.max(3, (v / max) * 100), peak: m === peak }))
})
const peakMonth = computed(() => monthName(peakIndex(r.value.months)))

const maxGenreShare = computed(() => Math.max(0.01, ...(props.card.genres ?? []).map(g => g.share)))

const fullShare = computed(() => (r.value.plays > 0 ? Math.min(1, r.value.full_listens / r.value.plays) : 0))

const cardStyle = computed(() => {
  const a = props.card.accent
  return a ? { '--rc-h': `${a.hue}`, '--rc-s': `${a.sat}%` } : {}
})

const backdropCover = computed(() => {
  const c = props.card
  return c.track?.cover_url ?? c.album?.top_track.cover_url ?? c.obsession?.track.cover_url
    ?? (c.artist ? avatar(c.artist) : null)
})

function playRow(url: string | null) {
  if (url) emit('sound', url)
}
</script>

<template>
  <div class="rc" :class="[`tone-${card.tone}`, `kind-${card.kind}`]" :style="cardStyle">
    <div class="rcBg" aria-hidden="true">
      <template v-if="card.tone === 'cover' && backdropCover">
        <img class="rcBackdrop" :src="backdropCover" alt="" />
        <div class="rcBackdropShade"></div>
      </template>
      <div v-if="card.kind === 'intro' && introCovers.length >= 3" class="rcMosaic">
        <RecapCover v-for="(url, i) in introCovers" :key="url" class="rcMosaicCover" :style="{ '--i': i }" :url="url" />
      </div>
      <div class="rcGrain"></div>
    </div>

    <div class="rcBody">
      <!-- Not enough history -->
      <template v-if="card.kind === 'empty'">
        <div class="rcCenter">
          <div class="rcKicker s0">{{ t('recap.title', { year: recap.year }) }}</div>
          <div class="rcTitle s1">{{ t('recap.empty.title') }}</div>
          <p class="rcText s2">{{ t('recap.empty.text') }}</p>
        </div>
      </template>

      <!-- Intro -->
      <template v-else-if="card.kind === 'intro'">
        <div class="rcMain end">
          <div class="rcKicker s0">LFMusic</div>
          <div class="rcDisplay s1">{{ t('recap.intro.title', { year: recap.year }) }}</div>
          <p class="rcText s2">{{ t('recap.intro.text') }}</p>
        </div>
        <div class="rcFoot s3">{{ t('recap.intro.hint') }}</div>
      </template>

      <!-- Minutes -->
      <template v-else-if="card.kind === 'minutes'">
        <div class="rcKicker s0">{{ t('recap.minutes.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber s1"><CountUp :value="minutesTotal" :locale="locale" /></div>
          <div class="rcUnit s1">{{ unit(minutesTotal, 'minutes') }}</div>
          <p class="rcText s2">{{ minutesEquivalent }}</p>
        </div>
        <div class="rcFoot s3">
          <div class="rcMeter"><span :style="{ '--w': recap.days_listened / daysInYear }"></span></div>
          {{ t('recap.minutes.daysListened', { days: count(recap.days_listened, 'days'), total: daysInYear }) }}
        </div>
      </template>

      <!-- First track of the year -->
      <template v-else-if="card.kind === 'first' && card.track">
        <div class="rcKicker s0">{{ t('recap.first.kicker') }}</div>
        <div class="rcMain">
          <RecapCover class="rcHeroCover s1" :url="card.track.cover_url" />
          <div class="rcTrackTitle s2">{{ title(card.track) }}</div>
          <div class="rcTrackArtist s2">{{ artistOf(card.track) }}</div>
        </div>
        <div class="rcFoot s3" v-if="recap.first_listen_at">
          {{ t('recap.first.text', { date: dateTime(recap.first_listen_at) }) }}
        </div>
      </template>

      <!-- Variety -->
      <template v-else-if="card.kind === 'variety'">
        <div class="rcKicker s0">{{ t('recap.variety.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber s1"><CountUp :value="recap.unique_tracks" :locale="locale" /></div>
          <div class="rcUnit s1">{{ unit(recap.unique_tracks, 'differentTracks') }}</div>
          <div v-if="varietyCovers.length >= 4" class="rcGrid">
            <RecapCover v-for="(url, i) in varietyCovers" :key="url" class="rcGridCover" :style="{ '--i': i }" :url="url" />
          </div>
        </div>
        <div class="rcRows s3">
          <div class="rcRowStat"><span>{{ t('recap.labels.plays') }}</span><b>{{ num(recap.plays) }}</b></div>
          <div class="rcRowStat"><span>{{ t('recap.labels.artists') }}</span><b>{{ num(recap.unique_artists) }}</b></div>
          <div v-if="recap.unique_albums > 0" class="rcRowStat"><span>{{ t('recap.labels.albums') }}</span><b>{{ num(recap.unique_albums) }}</b></div>
        </div>
      </template>

      <!-- Track of the year -->
      <template v-else-if="card.kind === 'topTrack' && card.track">
        <div class="rcKicker s0">{{ t('recap.topTrack.kicker') }}</div>
        <div class="rcMain">
          <RecapCover class="rcHeroCover s1" :url="card.track.cover_url" />
          <div class="rcTrackTitle s2">{{ title(card.track) }}</div>
          <div class="rcTrackArtist s2">{{ artistOf(card.track) }}</div>
        </div>
        <div class="rcRows s3">
          <div class="rcRowStat"><span>{{ t('recap.labels.played') }}</span><b>{{ count(card.track.plays, 'times') }}</b></div>
          <div class="rcRowStat"><span>{{ t('recap.labels.time') }}</span><b>{{ formatTotalDuration(card.track.secs) }}</b></div>
        </div>
      </template>

      <!-- Top tracks -->
      <template v-else-if="card.kind === 'topTracks' && card.tracks">
        <div class="rcKicker s0">{{ t('recap.topTracks.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.topTracks.title') }}</div>
        <div class="rcList">
          <button
            v-for="(tr, i) in card.tracks"
            :key="tr.file_path"
            type="button"
            class="rcRow"
            :class="{ playing: tr.snippet_url && playingUrl === tr.snippet_url }"
            :style="{ '--i': i }"
            :disabled="!tr.snippet_url"
            @click="playRow(tr.snippet_url)"
          >
            <span class="rcRank">
              <span v-if="tr.snippet_url && playingUrl === tr.snippet_url" class="rcEq"><i></i><i></i><i></i></span>
              <template v-else>{{ i + 1 }}</template>
            </span>
            <RecapCover class="rcRowCover" :url="tr.cover_url" />
            <span class="rcRowText">
              <span class="rcRowTitle">{{ title(tr) }}</span>
              <span class="rcRowSub">{{ artistOf(tr) }}</span>
            </span>
            <span class="rcRowValue">{{ count(tr.plays, 'times') }}</span>
          </button>
        </div>
      </template>

      <!-- Artist of the year / discovery -->
      <template v-else-if="(card.kind === 'topArtist' || card.kind === 'discovery') && card.artist">
        <div class="rcKicker s0">{{ t(`recap.${card.kind}.kicker`) }}</div>
        <div class="rcMain center">
          <RecapCover class="rcAvatar s1" round :url="avatar(card.artist)" />
          <div class="rcArtistName s2">{{ card.artist.name }}</div>
          <p class="rcText s2" v-if="card.kind === 'topArtist'">
            {{ t('recap.topArtist.text', { minutes: count(Math.round(card.artist.secs / 60), 'minutes') }) }}
          </p>
          <p class="rcText s2" v-else>
            {{ t('recap.discovery.text', { plays: count(card.artist.plays, 'plays') }) }}
          </p>
        </div>
        <div class="rcRows s3">
          <div class="rcRowStat"><span>{{ t('recap.labels.favoriteTrack') }}</span><b>{{ title(card.artist.top_track) }}</b></div>
        </div>
      </template>

      <!-- Top artists -->
      <template v-else-if="card.kind === 'topArtists' && card.artists">
        <div class="rcKicker s0">{{ t('recap.topArtists.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.topArtists.title') }}</div>
        <div class="rcList">
          <button
            v-for="(a, i) in card.artists"
            :key="a.name"
            type="button"
            class="rcRow"
            :class="{ playing: a.top_track.snippet_url && playingUrl === a.top_track.snippet_url }"
            :style="{ '--i': i }"
            :disabled="!a.top_track.snippet_url"
            @click="playRow(a.top_track.snippet_url)"
          >
            <span class="rcRank">
              <span v-if="a.top_track.snippet_url && playingUrl === a.top_track.snippet_url" class="rcEq"><i></i><i></i><i></i></span>
              <template v-else>{{ i + 1 }}</template>
            </span>
            <RecapCover class="rcRowCover" round :url="avatar(a)" />
            <span class="rcRowText">
              <span class="rcRowTitle">{{ a.name }}</span>
              <span class="rcRowSub">{{ count(a.track_count, 'tracks') }}</span>
            </span>
            <span class="rcRowValue">{{ count(Math.round(a.secs / 60), 'minutesShort') }}</span>
          </button>
        </div>
      </template>

      <!-- Album of the year -->
      <template v-else-if="card.kind === 'topAlbum' && card.album">
        <div class="rcKicker s0">{{ t('recap.topAlbum.kicker') }}</div>
        <div class="rcMain">
          <RecapCover class="rcHeroCover s1" :url="card.album.top_track.cover_url" />
          <div class="rcTrackTitle s2">{{ card.album.name }}</div>
          <div class="rcTrackArtist s2">{{ card.album.artist || t('recap.unknownArtist') }}</div>
        </div>
        <div class="rcRows s3">
          <div class="rcRowStat"><span>{{ t('recap.labels.plays') }}</span><b>{{ num(card.album.plays) }}</b></div>
          <div class="rcRowStat"><span>{{ t('recap.labels.time') }}</span><b>{{ formatTotalDuration(card.album.secs) }}</b></div>
        </div>
      </template>

      <!-- Genres -->
      <template v-else-if="card.kind === 'genres' && card.genres">
        <div class="rcKicker s0">{{ t('recap.genres.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.genres.title') }}</div>
        <div class="rcMain">
          <div class="rcGenres">
            <div v-for="(g, i) in card.genres" :key="g.name" class="rcGenre" :style="{ '--i': i }">
              <div class="rcGenreHead">
                <span class="rcGenreName">{{ genre(g.name) }}</span>
                <span class="rcGenreShare">{{ pct(g.share) }}</span>
              </div>
              <div class="rcBar"><span :style="{ '--w': g.share / maxGenreShare }"></span></div>
            </div>
          </div>
        </div>
        <div class="rcFoot s3" v-if="recap.unique_genres > card.genres.length">
          {{ t('recap.genres.more', { count: count(recap.unique_genres, 'genres') }) }}
        </div>
      </template>

      <!-- Time of day -->
      <template v-else-if="card.kind === 'moments' && card.moment">
        <div class="rcKicker s0">{{ t('recap.moments.kicker') }}</div>
        <div class="rcMain">
          <div class="rcTitle large s1">{{ t(`recap.moments.${card.moment.mode}.title`) }}</div>
          <p class="rcText s2" v-if="card.moment.count > 0">
            {{ t(`recap.moments.${card.moment.mode}.text`, { tracks: count(card.moment.count, 'tracks') }) }}
          </p>
          <p class="rcText s2" v-else>{{ t(`recap.moments.${card.moment.mode}.text2`) }}</p>
        </div>
        <div class="rcChart s3">
          <div class="rcBars">
            <span
              v-for="b in hourBars"
              :key="b.h"
              :class="{ peak: b.peak }"
              :style="{ '--hgt': b.height, '--i': b.h }"
            ></span>
          </div>
          <div class="rcAxis"><span>00</span><span>06</span><span>12</span><span>18</span><span>24</span></div>
          <div class="rcFoot">{{ t('recap.moments.peak', { hour: hourLabel(card.moment.peakHour) }) }}</div>
        </div>
      </template>

      <!-- One track on repeat -->
      <template v-else-if="card.kind === 'obsession' && card.obsession">
        <div class="rcKicker s0">{{ t('recap.obsession.kicker') }}</div>
        <div class="rcMain">
          <div class="rcHeroWrap s1">
            <RecapCover class="rcHeroCover" :url="card.obsession.track.cover_url" />
            <span class="rcBadge">x{{ card.obsession.plays }}</span>
          </div>
          <div class="rcTrackTitle s2">{{ title(card.obsession.track) }}</div>
          <div class="rcTrackArtist s2">{{ artistOf(card.obsession.track) }}</div>
        </div>
        <div class="rcFoot s3">
          {{ t('recap.obsession.text', { date: day(card.obsession.day), times: count(card.obsession.plays, 'times') }) }}
        </div>
      </template>

      <!-- Longest session -->
      <template v-else-if="card.kind === 'marathon' && card.session">
        <div class="rcKicker s0">{{ t('recap.marathon.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber s1">
            <CountUp :value="Math.floor(card.session.secs / 3600)" :locale="locale" /><span class="rcNumberUnit">{{ t('recap.units.h') }}</span>
            <CountUp :value="Math.floor((card.session.secs % 3600) / 60)" :locale="locale" /><span class="rcNumberUnit">{{ t('recap.units.min') }}</span>
          </div>
          <p class="rcText s2">{{ t('recap.marathon.text') }}</p>
        </div>
        <div class="rcRows s3">
          <div class="rcRowStat"><span>{{ t('recap.labels.tracksInRow') }}</span><b>{{ num(card.session.tracks) }}</b></div>
          <div class="rcRowStat"><span>{{ t('recap.labels.date') }}</span><b>{{ day(card.session.day) }}</b></div>
        </div>
      </template>

      <!-- Months, streak, record day -->
      <template v-else-if="card.kind === 'rhythm'">
        <div class="rcKicker s0">{{ t('recap.rhythm.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.rhythm.title', { month: peakMonth }) }}</div>
        <div class="rcChart grow s2">
          <div class="rcBars months">
            <span
              v-for="b in monthBars"
              :key="b.m"
              :class="{ peak: b.peak }"
              :style="{ '--hgt': b.height, '--i': b.m }"
            ></span>
          </div>
          <div class="rcAxis months"><span v-for="m in 12" :key="m">{{ monthLetter(m - 1) }}</span></div>
        </div>
        <div class="rcRows s3">
          <div class="rcRowStat"><span>{{ t('recap.labels.streak') }}</span><b>{{ count(recap.longest_streak, 'days') }}</b></div>
          <div v-if="recap.busiest_day" class="rcRowStat">
            <span>{{ t('recap.labels.record') }}</span>
            <b>{{ formatTotalDuration(recap.busiest_day.secs) }}, {{ day(recap.busiest_day.day) }}</b>
          </div>
        </div>
      </template>

      <!-- Collection growth -->
      <template v-else-if="card.kind === 'collection'">
        <div class="rcKicker s0">{{ t('recap.collection.kicker') }}</div>
        <div class="rcTitle s1">{{ t('recap.collection.title') }}</div>
        <div class="rcMain">
          <div class="rcFacts">
            <div v-if="recap.likes > 0" class="rcFact" style="--i: 0">
              <span class="rcFactValue">+{{ num(recap.likes) }}</span>
              <span class="rcFactLabel">{{ t('recap.collection.likes', { tracks: unit(recap.likes, 'tracks') }) }}</span>
            </div>
            <div v-if="recap.playlists_created > 0" class="rcFact" style="--i: 1">
              <span class="rcFactValue">{{ num(recap.playlists_created) }}</span>
              <span class="rcFactLabel">{{ unit(recap.playlists_created, 'newPlaylists') }}</span>
            </div>
            <div v-if="recap.downloads > 0" class="rcFact" style="--i: 2">
              <span class="rcFactValue">{{ num(recap.downloads) }}</span>
              <span class="rcFactLabel">{{ unit(recap.downloads, 'downloads') }}</span>
            </div>
          </div>
        </div>
        <div v-if="recap.top_playlist" class="rcPlaylist s3">
          <RecapCover class="rcRowCover" :url="recap.top_playlist.cover_url" />
          <span class="rcRowText">
            <span class="rcRowSub">{{ t('recap.collection.playlist') }}</span>
            <span class="rcRowTitle">{{ recap.top_playlist.kind === 'favorites' ? t('recap.collection.favorites') : recap.top_playlist.name }}</span>
          </span>
        </div>
      </template>

      <!-- Listening habits -->
      <template v-else-if="card.kind === 'habits'">
        <div class="rcKicker s0">{{ t('recap.habits.kicker') }}</div>
        <div class="rcMain">
          <div class="rcNumber s1"><CountUp :value="Math.round(fullShare * 100)" :locale="locale" suffix="%" /></div>
          <p class="rcText s2">{{ t('recap.habits.full') }}</p>
        </div>
        <div class="rcRows s3">
          <div v-if="recap.shuffle_share >= 0.05" class="rcRowStat"><span>{{ t('recap.labels.shuffle') }}</span><b>{{ pct(recap.shuffle_share) }}</b></div>
          <div class="rcRowStat"><span>{{ t('recap.labels.skips') }}</span><b>{{ num(recap.skips) }}</b></div>
          <div v-if="recap.search_starts > 0" class="rcRowStat"><span>{{ t('recap.labels.search') }}</span><b>{{ num(recap.search_starts) }}</b></div>
          <div v-if="recap.mobile_share >= 0.05" class="rcRowStat"><span>{{ t('recap.labels.mobile') }}</span><b>{{ pct(recap.mobile_share) }}</b></div>
        </div>
      </template>

      <!-- Character -->
      <template v-else-if="card.kind === 'character' && card.character">
        <div class="rcKicker s0">{{ t('recap.character.kicker') }}</div>
        <div class="rcMain">
          <div class="rcDisplay s1">{{ t(`recap.character.${card.character.id}.title`) }}</div>
          <p class="rcText s2">{{ t(`recap.character.${card.character.id}.desc`) }}</p>
          <div class="rcChips s3" v-if="card.character.traits.length > 0">
            <span v-for="trait in card.character.traits" :key="trait" class="rcChip">
              {{ t(`recap.character.${trait}.title`) }}
            </span>
          </div>
        </div>
      </template>

      <!-- Summary -->
      <template v-else-if="card.kind === 'outro'">
        <div class="rcKicker s0">{{ t('recap.title', { year: recap.year }) }}</div>
        <div class="rcTitle s1">{{ t('recap.outro.title') }}</div>
        <div class="rcMain">
          <div class="rcSummaryTop s2" v-if="recap.top_tracks[0]">
            <RecapCover class="rcSummaryCover" :url="recap.top_tracks[0].cover_url" />
            <div class="rcRowText">
              <span class="rcRowSub">{{ t('recap.outro.track') }}</span>
              <span class="rcRowTitle">{{ title(recap.top_tracks[0]) }}</span>
              <span class="rcRowSub">{{ artistOf(recap.top_tracks[0]) }}</span>
            </div>
          </div>
          <div class="rcRows s2">
            <div class="rcRowStat"><span>{{ t('recap.outro.minutes') }}</span><b>{{ num(minutesTotal) }}</b></div>
            <div v-if="recap.top_artists[0]" class="rcRowStat"><span>{{ t('recap.outro.artist') }}</span><b>{{ recap.top_artists[0].name }}</b></div>
            <div v-if="recap.top_genres[0]" class="rcRowStat"><span>{{ t('recap.outro.genre') }}</span><b>{{ genre(recap.top_genres[0].name) }}</b></div>
            <div v-if="card.character" class="rcRowStat"><span>{{ t('recap.outro.character') }}</span><b>{{ t(`recap.character.${card.character.id}.title`) }}</b></div>
          </div>
        </div>
        <button type="button" class="rcButton s3" @click="emit('replay')">{{ t('recap.outro.replay') }}</button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.rc {
  --h: var(--rc-h, var(--accent-h, 250));
  --s: var(--rc-s, var(--accent-s, 45%));
  --display: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  /* Tamed accent for small highlights: bars, peaks, eq. */
  --accent-soft: hsl(var(--h), min(var(--s), 70%), 72%);
  position: absolute;
  inset: 0;
  overflow: hidden;
  color: hsla(0, 0%, 100%, 0.96);
  user-select: none;
  background: hsl(var(--h), min(var(--s), 26%), 8%);
  /* Type is sized against the card (cqw/cqh), which follows the window size. */
  container-type: size;
}

/* ── Backgrounds ─────────────────────────────────────────────────────────── */

.rcBg {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(130% 62% at 50% -6%, hsla(var(--h), var(--s), 56%, 0.24), transparent 64%),
    radial-gradient(120% 45% at 50% 108%, hsla(var(--h), var(--s), 48%, 0.1), transparent 70%);
}
.tone-cover .rcBg {
  background: none;
}
.rcBackdrop {
  position: absolute;
  inset: -20%;
  width: 140%;
  height: 140%;
  object-fit: cover;
  filter: blur(60px) saturate(1.1);
  opacity: 0.6;
}
.rcBackdropShade {
  position: absolute;
  inset: 0;
  background: linear-gradient(
    180deg,
    hsla(var(--h), 22%, 7%, 0.5) 0%,
    hsla(var(--h), 22%, 7%, 0.35) 40%,
    hsla(var(--h), 22%, 6%, 0.94) 100%
  );
}
.rcGrain {
  position: absolute;
  inset: 0;
  opacity: 0.04;
  mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='160' height='160'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/></filter><rect width='100%' height='100%' filter='url(%23n)'/></svg>");
}

/* Intro: the year's covers as a quiet wall behind the title. */
.rcMosaic {
  position: absolute;
  inset: 0 0 auto 0;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 6px;
  padding: 6px;
  opacity: 0.5;
  -webkit-mask-image: linear-gradient(180deg, #000 30%, transparent 78%);
  mask-image: linear-gradient(180deg, #000 30%, transparent 78%);
}
.rcMosaicCover {
  aspect-ratio: 1;
  border-radius: 8px;
  animation: rcFade .8s ease both;
  animation-delay: calc(var(--i) * 0.05s);
}

/* ── Layout ──────────────────────────────────────────────────────────────── */

.rcBody {
  position: relative;
  z-index: 1;
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 72px 28px 28px;
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
  padding-bottom: 24px;
}
.rcMain.center {
  align-items: center;
  text-align: center;
}
.rcCenter {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  gap: 10px;
}

/* Staggered entrance: s0..s3 appear one after another. */
.s0, .s1, .s2, .s3 {
  animation: rcRise .5s cubic-bezier(.2, .8, .2, 1) both;
}
.s1 { animation-delay: .08s; }
.s2 { animation-delay: .18s; }
.s3 { animation-delay: .3s; }
@keyframes rcRise {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: none; }
}
@keyframes rcFade {
  from { opacity: 0; }
  to { opacity: 1; }
}

/* ── Type ────────────────────────────────────────────────────────────────── */

.rcKicker {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: .12em;
  text-transform: uppercase;
  color: hsla(var(--h), min(var(--s), 60%), 82%, 0.8);
}
.rcDisplay {
  margin-top: 10px;
  font-family: var(--display);
  font-size: clamp(36px, 12.5cqw, 60px);
  line-height: 1.02;
  font-weight: 800;
  letter-spacing: -0.025em;
  text-wrap: balance;
}
.rcTitle {
  margin-top: 8px;
  font-family: var(--display);
  font-size: clamp(22px, 7cqw, 30px);
  line-height: 1.15;
  font-weight: 800;
  letter-spacing: -0.01em;
  text-wrap: balance;
}
.rcTitle.large {
  font-size: clamp(30px, 10cqw, 44px);
  line-height: 1.05;
}
.rcNumber {
  font-family: var(--display);
  font-size: clamp(52px, 19cqw, 88px);
  line-height: 0.95;
  font-weight: 800;
  letter-spacing: -0.035em;
}
.rcNumberUnit {
  font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  font-size: 0.32em;
  font-weight: 600;
  letter-spacing: 0;
  margin: 0 0.5em 0 0.15em;
  color: hsla(0, 0%, 100%, 0.6);
}
.rcUnit {
  margin-top: 6px;
  font-size: 17px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.72);
}
.rcText {
  margin: 12px 0 0;
  max-width: 330px;
  font-size: 15px;
  line-height: 1.5;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.66);
  text-wrap: pretty;
}
.rcFoot {
  font-size: 13px;
  line-height: 1.45;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.5);
}

/* Days with music out of the year. */
.rcMeter {
  height: 4px;
  margin-bottom: 10px;
  border-radius: 999px;
  background: hsla(0, 0%, 100%, 0.1);
  overflow: hidden;
}
.rcMeter span {
  display: block;
  height: 100%;
  width: calc(var(--w) * 100%);
  border-radius: inherit;
  background: var(--accent-soft);
  transform-origin: left;
  animation: rcGrow 1s cubic-bezier(.2, .8, .2, 1) .5s both;
}

/* Label / value rows with hairline dividers, like settings rows. */
.rcRows {
  display: flex;
  flex-direction: column;
}
.rcRowStat {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  padding: 11px 0;
  border-top: 1px solid hsla(0, 0%, 100%, 0.08);
  font-size: 14px;
}
.rcRowStat span {
  flex-shrink: 0;
  color: hsla(0, 0%, 100%, 0.55);
}
.rcRowStat b {
  min-width: 0;
  font-weight: 600;
  text-align: right;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-variant-numeric: tabular-nums;
}

.rcChips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 18px;
}
.rcChip {
  padding: 6px 12px;
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.85);
  background: hsla(0, 0%, 100%, 0.08);
}

/* ── Covers ──────────────────────────────────────────────────────────────── */

.rcHeroCover {
  width: min(100%, 46cqh);
  aspect-ratio: 1;
  border-radius: 12px;
  box-shadow: 0 18px 50px hsla(0, 0%, 0%, 0.45);
}
.rcHeroWrap {
  position: relative;
  width: min(100%, 46cqh);
}
.rcHeroWrap .rcHeroCover {
  width: 100%;
}
.rcBadge {
  position: absolute;
  left: 12px;
  bottom: 12px;
  padding: 5px 10px;
  border-radius: 999px;
  font-size: 13px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: #fff;
  background: hsla(0, 0%, 0%, 0.55);
  backdrop-filter: blur(10px);
}
.rcTrackTitle {
  margin-top: 22px;
  max-width: 100%;
  font-family: var(--display);
  font-size: clamp(20px, 6.6cqw, 28px);
  line-height: 1.15;
  font-weight: 800;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}
.rcTrackArtist {
  margin-top: 4px;
  max-width: 100%;
  font-size: 15px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.62);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.rcAvatar {
  width: min(62%, 40cqh);
  aspect-ratio: 1;
  box-shadow: 0 18px 50px hsla(0, 0%, 0%, 0.45);
}
.rcArtistName {
  margin-top: 22px;
  max-width: 100%;
  font-family: var(--display);
  font-size: clamp(26px, 8.6cqw, 40px);
  line-height: 1.05;
  font-weight: 800;
  letter-spacing: -0.02em;
  overflow-wrap: anywhere;
}
.rcMain.center .rcText {
  margin-left: auto;
  margin-right: auto;
}

.rcGrid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
  margin-top: 24px;
}
.rcGridCover {
  aspect-ratio: 1;
  border-radius: 6px;
  animation: rcRise .5s cubic-bezier(.2, .8, .2, 1) both;
  animation-delay: calc(.25s + var(--i) * .04s);
}

/* ── Lists ───────────────────────────────────────────────────────────────── */

.rcList {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
  margin-top: 12px;
}
.rcRow {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  height: 60px;
  padding: 0 10px 0 4px;
  border-radius: 10px;
  text-align: left;
  color: inherit;
  background: transparent;
  cursor: pointer;
  transition: background-color .18s ease;
  animation: rcRise .45s cubic-bezier(.2, .8, .2, 1) both;
  animation-delay: calc(.2s + var(--i) * .05s);
}
.rcRow:disabled {
  cursor: default;
}
.rcRow:not(:disabled):hover {
  background: hsla(0, 0%, 100%, 0.05);
}
.rcRow.playing {
  background: hsla(var(--h), var(--s), 60%, 0.12);
}
.rcRank {
  width: 22px;
  flex-shrink: 0;
  display: flex;
  justify-content: center;
  font-size: 13px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.4);
}
.rcEq {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 12px;
}
.rcEq i {
  width: 3px;
  height: 100%;
  border-radius: 1px;
  background: var(--accent-soft);
  animation: rcEq .9s ease-in-out infinite alternate;
}
.rcEq i:nth-child(2) { animation-delay: -.3s; }
.rcEq i:nth-child(3) { animation-delay: -.6s; }
@keyframes rcEq {
  from { transform: scaleY(.3); }
  to { transform: scaleY(1); }
}
.rcRowCover {
  width: 44px;
  height: 44px;
  border-radius: 6px;
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
  font-size: 14.5px;
  font-weight: 600;
}
.rcRowSub {
  font-size: 12.5px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.55);
}
.rcRowValue {
  flex-shrink: 0;
  font-size: 12.5px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.5);
}

/* ── Genres ──────────────────────────────────────────────────────────────── */

.rcGenres {
  display: flex;
  flex-direction: column;
  gap: 22px;
}
.rcGenre {
  animation: rcRise .5s cubic-bezier(.2, .8, .2, 1) both;
  animation-delay: calc(.2s + var(--i) * .08s);
}
.rcGenreHead {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}
.rcGenreName {
  min-width: 0;
  font-size: 17px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rcGenreShare {
  font-size: 15px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.75);
}
.rcBar {
  margin-top: 9px;
  height: 4px;
  border-radius: 999px;
  background: hsla(0, 0%, 100%, 0.1);
  overflow: hidden;
}
.rcBar span {
  display: block;
  height: 100%;
  width: calc(var(--w) * 100%);
  border-radius: inherit;
  background: var(--accent-soft);
  transform-origin: left;
  animation: rcGrow .9s cubic-bezier(.2, .8, .2, 1) both;
}
.rcGenre .rcBar span {
  animation-delay: calc(.35s + var(--i) * .08s);
}
@keyframes rcGrow {
  from { transform: scaleX(0); }
  to { transform: scaleX(1); }
}

/* ── Charts ──────────────────────────────────────────────────────────────── */

.rcChart.grow {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  padding: 20px 0 18px;
}
.rcBars {
  display: flex;
  align-items: flex-end;
  gap: 3px;
  height: 96px;
}
.rcBars.months {
  height: min(180px, 26cqh);
  gap: 6px;
}
.rcBars span {
  flex: 1;
  height: calc(var(--hgt) * 1%);
  border-radius: 2px;
  background: hsla(0, 0%, 100%, 0.16);
  transform-origin: bottom;
  animation: rcGrowUp .6s cubic-bezier(.2, .8, .2, 1) both;
  animation-delay: calc(.3s + var(--i) * .015s);
}
.rcBars.months span {
  border-radius: 3px;
  animation-delay: calc(.3s + var(--i) * .03s);
}
.rcBars span.peak {
  background: var(--accent-soft);
}
@keyframes rcGrowUp {
  from { transform: scaleY(0); }
  to { transform: scaleY(1); }
}
.rcAxis {
  display: flex;
  justify-content: space-between;
  margin: 8px 0 14px;
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: hsla(0, 0%, 100%, 0.38);
}
.rcChart.grow .rcAxis {
  margin-bottom: 0;
}
.rcAxis.months span {
  flex: 1;
  text-align: center;
}

/* ── Collection ──────────────────────────────────────────────────────────── */

.rcFacts {
  display: flex;
  flex-direction: column;
}
.rcFact {
  display: flex;
  align-items: baseline;
  gap: 14px;
  padding: 14px 0;
  border-top: 1px solid hsla(0, 0%, 100%, 0.08);
  animation: rcRise .5s cubic-bezier(.2, .8, .2, 1) both;
  animation-delay: calc(.2s + var(--i) * .08s);
}
.rcFact:first-child {
  border-top: none;
}
.rcFactValue {
  min-width: 2.6em;
  font-family: var(--display);
  font-size: clamp(30px, 10cqw, 44px);
  line-height: 1;
  font-weight: 800;
  letter-spacing: -0.02em;
  font-variant-numeric: tabular-nums;
}
.rcFactLabel {
  font-size: 15px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.66);
}
.rcPlaylist {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-top: 14px;
  border-top: 1px solid hsla(0, 0%, 100%, 0.08);
}

/* ── Summary ─────────────────────────────────────────────────────────────── */

.rcSummaryTop {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-bottom: 18px;
}
.rcSummaryCover {
  width: 84px;
  height: 84px;
  border-radius: 10px;
  box-shadow: 0 12px 32px hsla(0, 0%, 0%, 0.4);
}
.rcSummaryTop .rcRowTitle {
  font-family: var(--display);
  font-size: 19px;
  font-weight: 800;
}
.rcButton {
  height: 44px;
  margin-top: 18px;
  border-radius: 12px;
  font-size: 14px;
  font-weight: 700;
  color: #fff;
  background: hsla(var(--h), var(--s), 58%, 0.28);
  cursor: pointer;
  transition: background-color .18s ease;
}
.rcButton:hover {
  background: hsla(var(--h), var(--s), 58%, 0.4);
}

/* Short cards (small windows, phones in landscape). */
@media (max-height: 640px) {
  .rcBody { padding: 62px 22px 22px; }
  .rcRow { height: 52px; }
  .rcRowCover { width: 38px; height: 38px; }
  .rcRowStat { padding: 8px 0; }
}

@media (prefers-reduced-motion: reduce) {
  .rcEq i {
    animation: none;
  }
}
</style>
