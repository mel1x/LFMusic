import { createRouter, createWebHashHistory } from 'vue-router'
import AudioPlayer from "./views/AudioPlayer.vue"
import Main from './views/Main.vue'
import Catalog from './views/Catalog.vue'
import Search from './views/Search.vue'
import Collections from './views/Collections.vue'

const routes = [
  { path: '/search', component: Search },
  { path: '/', component: Main },
  { path: '/catalog', component: Catalog },
  { path: '/collections', component: Collections },
  { path: '/test', component: AudioPlayer },
  { path: '/settings', component: () => import('./views/Settings.vue') },
]

export default createRouter({
  history: createWebHashHistory(),
  routes,
})