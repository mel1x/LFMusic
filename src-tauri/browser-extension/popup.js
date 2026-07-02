const statusEl = document.getElementById("status");
const connectButton = document.getElementById("connect");
const openMusicButton = document.getElementById("open-music");
const BASE_URL = "http://127.0.0.1:51823";
const MUSIC_URL = "https://music.youtube.com/";
const YOUTUBE_URL = "https://www.youtube.com/";
const ROOT_YOUTUBE_URL = "https://youtube.com/";
const GOOGLE_URL = "https://accounts.google.com/";
const AUTH_COOKIE_NAMES = ["__Secure-3PSID", "__Secure-1PSID", "LOGIN_INFO", "SID"];

function setStatus(message, tone = "") {
  if (!statusEl) return;
  statusEl.textContent = message;
  statusEl.className = `status ${tone}`.trim();
}

function mergeCookies(...groups) {
  // Later groups win on name collisions (music.youtube.com's copy of a
  // cookie is preferred over a more generic google.com one).
  const merged = new Map();
  for (const group of groups) {
    for (const cookie of group) {
      merged.set(cookie.name, cookie);
    }
  }
  return [...merged.values()];
}

async function collectCookies() {
  const [googleCookies, rootYoutubeCookies, youtubeCookies, musicCookies] = await Promise.all([
    chrome.cookies.getAll({ url: GOOGLE_URL }),
    chrome.cookies.getAll({ url: ROOT_YOUTUBE_URL }),
    chrome.cookies.getAll({ url: YOUTUBE_URL }),
    chrome.cookies.getAll({ url: MUSIC_URL }),
  ]);

  const merged = mergeCookies(googleCookies, rootYoutubeCookies, youtubeCookies, musicCookies);
  if (!merged.length) {
    throw new Error("В этом браузере не найдены cookies YouTube Music.");
  }
  if (!merged.some((c) => AUTH_COOKIE_NAMES.includes(c.name))) {
    throw new Error("Не найдены cookies авторизации — войдите в аккаунт YouTube в этом браузере.");
  }

  return merged.map((c) => ({
    name: c.name,
    value: c.value,
    domain: c.domain,
    path: c.path,
    secure: c.secure,
    httpOnly: c.httpOnly,
    expirationDate: c.expirationDate ?? null,
  }));
}

async function callBridge(path, options = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 8000);

  try {
    const response = await fetch(`${BASE_URL}${path}`, {
      ...options,
      signal: controller.signal,
      headers: {
        "Content-Type": "application/json",
        ...(options.headers || {}),
      },
    });

    const payload = await response.json();
    if (!response.ok || !payload?.success) {
      throw new Error(payload?.error || "Запрос к LFMusic не удался.");
    }

    return payload;
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error("LFMusic не ответил. Убедитесь, что приложение запущено.");
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

connectButton?.addEventListener("click", async () => {
  connectButton.disabled = true;
  setStatus("Собираем сессию браузера...");

  try {
    setStatus("Проверяем подключение к LFMusic...");
    await callBridge("/bridge/ping");
    const cookies = await collectCookies();
    setStatus("Отправляем сессию в LFMusic...");
    await callBridge("/bridge/import-session", {
      method: "POST",
      body: JSON.stringify({ cookies }),
    });

    setStatus("Сессия отправлена. Вернитесь в LFMusic.", "success");
  } catch (error) {
    setStatus(error instanceof Error ? error.message : "Не удалось подключиться к LFMusic.", "error");
  } finally {
    connectButton.disabled = false;
  }
});

openMusicButton?.addEventListener("click", () => {
  chrome.tabs.create({ url: "https://music.youtube.com" });
});
