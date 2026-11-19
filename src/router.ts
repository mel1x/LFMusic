import { createRouter, createWebHashHistory, type Router } from 'vue-router'
import { isAndroidRuntime, isPhoneViewport } from './composables/usePlatform'
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
  { path: '/downloads/artist/:id/tracks', component: () => import('./views/DownloadArtistTracks.vue') },
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
  {
    path: '/settings',
    component: () => import('./views/SettingsLayout.vue'),
    children: [
      { path: '', component: () => import('./views/Settings.vue') },
      { path: 'appearance', component: () => import('./views/settings/Appearance.vue') },
      { path: 'playback', component: () => import('./views/settings/Playback.vue') },
      { path: 'library', component: () => import('./views/settings/Library.vue') },
      { path: 'general', component: () => import('./views/settings/General.vue') },
      { path: 'external', component: () => import('./views/settings/ExternalSources.vue') },
    ],
  },
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

router.beforeEach((to) => {
  if (to.path === '/settings/external' && isAndroidRuntime()) return '/settings'
  if (to.path === '/settings' && !isPhoneViewport()) return '/settings/appearance'
})

// No previous entry in this session (launch, deep link): router.back() would do nothing.
export function goBackOrHome(instance: Router = router) {
  if (window.history.state?.back) {
    instance.back()
  } else {
    instance.push('/')
  }
}

export default router
