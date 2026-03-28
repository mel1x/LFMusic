<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window';
import MaximizeIcon from './components/Icons/MaximizeIcon.vue';
import MinimizeIcon from './components/Icons/MinimizeIcon.vue';
import CloseIcon from './components/Icons/CloseIcon.vue';

import MainBlock from './components/MainBlock.vue';
import Player from './components/Player.vue';
import GithubIcon from './components/Icons/GithubIcon.vue';

const win = getCurrentWindow();

const minimizeWindow = () => win.minimize();
const maximizeWindow = () => win.toggleMaximize();
const closeWindow = () => win.close();
</script>

<template>
  <div class="hat" data-tauri-drag-region>
    <div class="winTitle">
      <div class="mainTitle" data-tauri-drag-region>LFMusic</div>
      <div class="authorTitle" data-tauri-drag-region>by mel1x</div>
    </div>
    <div class="winControls">
      <a href="https://github.com/mel1x/LFMusic" target="_blank" rel="noopener noreferrer"><GithubIcon/>Github</a>
      <button class="controlsButton" @click="minimizeWindow"><MinimizeIcon /></button>
      <button class="controlsButton" @click="maximizeWindow"><MaximizeIcon /></button>
      <button class="controlsButton" @click="closeWindow"><CloseIcon /></button>
    </div>
  </div>
  <MainBlock class="mainFrame" />
  <Player class="playerPart" />
</template>

<style>
* {
  margin: 0;
  padding: 0;
  border: 0;
  font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  box-sizing: border-box;
  outline: none;
  text-decoration: none;
}
svg {
  display: block;
}
body {
  background-color: #0d0d0d;
}
.hat {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  height: 38px;
  background-color: #0d0d0d;
}
.winTitle {
  margin-top: 2px;
  padding: 0px 20px;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.mainTitle {
  user-select: none;
  font-size: 15px;
  font-weight: 800;
  color: #fff;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;  
  letter-spacing: -0.025em;
}
.authorTitle {
  user-select: none;
  font-size: 11px;
  line-height: 1;
  font-weight: 600;
  text-transform: uppercase;
  color: #727272;
}
.winControls {
  height: 100%;
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: center;
}
.winControls a {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 3px;
  justify-content: center;
  color: #ffffffd5;
  font-size: 12px;
  font-weight: 600;
  margin-right: 20px;
  padding: 2px 8px 3px 3px;
  border-radius: 16px;
  background: linear-gradient(172deg, #454b5c, #181c26);
  filter: grayscale();
}
.controlsButton {
  cursor: pointer;
  width: 46px;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background-color: transparent;
  transition: background-color 0.14s ease-in-out;

  &:hover {
    background-color: #1b1b1b;
  }
}
.controlsButton:nth-child(3) {
  transition: background-color 0.19s ease-in-out;

  &:hover {
    background-color: #f23f42;
  }
}
.mainFrame {
  width: 100%;
  height: calc(100vh - 38px - 104px);
}
.playerPart {
  width: 100%;
  height: 104px;
}
</style>
