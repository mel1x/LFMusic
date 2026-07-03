import { createRouter, createWebHashHistory, type Router } from 'vue-router'
import Main from './views/Main.vue'
import Catalog from './views/Catalog.vue'
import Search from './views/Search.vue'
import Collections from './views/Collections.vue'
import CollectionPlaylists from './views/CollectionPlaylists.vue'
import CollectionAlbums from './views/CollectionAlbums.vue'
import CollectionArtists from './views/CollectionArtists.vue'
import Playlist from './views/Playlist.vue'
import Album from './views/Album.vue'
import Artist from './views/Artist.vue'
import ArtistTracks from './views/ArtistTracks.vue'

const routes = [
  { path: '/downloads', component: () => import('./views/Downloads.vue') },
  { path: '/downloads/album/:id', component: () => import('./views/DownloadAlbum.vue') },
  { path: '/downloads/artist/:id', component: () => import('./views/DownloadArtist.vue') },
  { path: '/search', component: Search },
  { path: '/', component: Main },
  { path: '/catalog', component: Catalog },
  { path: '/collections', component: Collections },
  { path: '/collections/playlists', component: CollectionPlaylists },
  { path: '/collections/albums', component: CollectionAlbums },
  { path: '/collections/artists', component: CollectionArtists },
  { path: '/playlist/:id', component: Playlist, props: true },
  { path: '/album/:name', component: Album, props: true },
  { path: '/artist/:name', component: Artist, props: true },
  { path: '/artist/:name/tracks', component: ArtistTracks, props: true },
  { path: '/settings', component: () => import('./views/Settings.vue') },
  { path: '/settings/appearance', component: () => import('./views/settings/Appearance.vue') },
  { path: '/settings/playback', component: () => import('./views/settings/Playback.vue') },
  { path: '/settings/library', component: () => import('./views/settings/Library.vue') },
  { path: '/settings/general', component: () => import('./views/settings/General.vue') },
  { path: '/settings/external', component: () => import('./views/settings/ExternalSources.vue') },
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

// window.history.state.back is null when there's no previous entry in this
// session (e.g. right after launch, or a deep link) — router.back() would
// silently no-op then, so fall back to the recommendations page instead.
export function goBackOrHome(instance: Router = router) {
  if (window.history.state?.back) {
    instance.back()
  } else {
    instance.push('/')
  }
}

export default router
