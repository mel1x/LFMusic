import { createRouter, createWebHashHistory } from 'vue-router'
import AudioPlayer from "./views/AudioPlayer.vue"
import Settings from './views/Settings.vue'

const routes = [
  { path: '/', component: AudioPlayer },
  { path: '/settings', component: Settings },
]

export default createRouter({
  history: createWebHashHistory(),
  routes,
})