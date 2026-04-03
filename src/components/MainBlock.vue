<script setup lang="ts">
import ViewFrame from './ViewFrame.vue';
import CloseIcon from './Icons/CloseIcon.vue';
</script>

<template>
  <div class="mainWrapper">
    <aside>
      <RouterLink to="/search" class="navLink"><CloseIcon />Поиск</RouterLink>
      <RouterLink to="/" class="navLink"><CloseIcon />Главная</RouterLink>
      <RouterLink to="/catalog" class="navLink"><CloseIcon />Каталог</RouterLink>
      <RouterLink to="/collections" class="navLink"><CloseIcon />Коллекция</RouterLink>
      <RouterLink to="/settings" class="navLink"><CloseIcon />Настройки</RouterLink>
      <RouterLink to="/test" class="navLink"><CloseIcon />Audio Player</RouterLink>
    </aside>
    <div class="frameWrapper">
      <ViewFrame>
        <RouterView />
      </ViewFrame>
    </div>
  </div>
</template>

<style scoped>
.mainWrapper {
  display: flex;
  flex-direction: row;
}
aside {
  display: flex;
  flex-direction: column;
  padding: 32px 14px 20px;
  gap: 8px;
  width: 260px;
  height: calc(100vh - 38px - 104px);
}
.navLink {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 12px;
  color: hsla(0, 0%, 80%, 0.5);
  width: 100%;
  font-weight: 600;
  padding: 10px 14px;
  font-size: 16px;

  transition: .2s;

  &:hover {
    color: hsla(0, 0%, 100%, 0.95);
  }
}
.navLink.router-link-active {
  color: hsla(0, 0%, 100%, 0.95);
}
.frameWrapper {
  width: calc(100% - 260px - 12px);
  height: calc(100vh - 38px - 104px);
}
</style>