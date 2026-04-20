<script setup lang="ts">
import { computed } from 'vue';
import { useRoute } from 'vue-router';
import ViewFrame from './ViewFrame.vue';
import SearchIcon from './Icons/SearchIcon.vue';
import CatalogIcon from './Icons/CatalogIcon.vue';
import CollectionIcon from './Icons/CollectionIcon.vue';
import SettingsIcon from './Icons/SettingsIcon.vue';
import HomeIcon from './Icons/HomeIcon.vue';

const route = useRoute();
const isCollectionsActive = computed(() => route.path.startsWith('/playlist') || route.path.startsWith('/album') || route.path.startsWith('/collections'));
</script>

<template>
  <div class="mainWrapper">
    <aside>
      <RouterLink to="/search" class="navLink"><SearchIcon width="24" height="24" stroke-width="2.25" />Поиск</RouterLink>
      <RouterLink to="/" class="navLink"><HomeIcon width="24" height="24" stroke-width="2.25" />Главная</RouterLink>
      <RouterLink to="/collections" class="navLink" :class="{ 'router-link-active': isCollectionsActive }"><CollectionIcon width="24" height="24" stroke-width="2.25" />Коллекция</RouterLink>
      <RouterLink to="/catalog" class="navLink"><CatalogIcon width="24" height="24" stroke-width="2.25" />Каталог</RouterLink>
      <RouterLink to="/settings" class="navLink"><SettingsIcon width="24" height="24" stroke-width="2.25" />Настройки</RouterLink>
    </aside>
    <div class="frameWrapper">
      <ViewFrame>
        <RouterView v-slot="{ Component }">
          <Transition name="fade" mode="out-in">
            <component :is="Component" />
          </Transition>
        </RouterView>
      </ViewFrame>
    </div>
  </div>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.24s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.mainWrapper {
  display: flex;
  flex-direction: row;
  width: 100%;
  height: 100%;
}
aside {
  display: flex;
  flex-direction: column;
  padding: 32px 14px 20px;
  gap: 8px;
  width: 240px;
  height: 100%;
}
.navLink {
  user-select: none;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 12px;
  color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
  width: 100%;
  font-weight: 600;
  padding: 10px 14px;
  font-size: 14px;

  transition: .2s ease;

  &:hover {
    color: hsla(0, 0%, 100%, 0.95);
  }
}
.navLink svg {
  color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
  transition: .2s ease;

  .navLink:hover & {
    color: #fff;
  }
}
.navLink.router-link-active {
  color: hsla(0, 0%, 100%, 0.95);
}
.navLink.router-link-active svg {
  color: #fff;
}
.frameWrapper {
  border-radius: 16px;
  background-color: var(--light-bg);
  background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
  background-repeat: no-repeat;
  background-attachment: local;
  width: calc(100% - 240px - 12px);
  height: 100%;
  transition: .24s ease;
}
</style>