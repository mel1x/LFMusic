import { createRouter, createWebHashHistory } from 'vue-router'
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
]

export default createRouter({
  history: createWebHashHistory(),
  routes,
})
