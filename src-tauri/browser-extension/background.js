// MV3 service worker: owns everything that needs an extension-privileged
// context. chrome.cookies is unavailable to content scripts, and a page's own
// CSP can block a content script's fetch() — so both cookie collection and
// the bridge HTTP calls live here, driven by messages from content.js.
const BASE_URL = "http://127.0.0.1:51823";
const MUSIC_URL = "https://music.youtube.com/";
const YOUTUBE_URL = "https://www.youtube.com/";
const ROOT_YOUTUBE_URL = "https://youtube.com/";
const GOOGLE_URL = "https://accounts.google.com/";
const AUTH_COOKIE_NAMES = ["__Secure-3PSID", "__Secure-1PSID", "LOGIN_INFO", "SID"];

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
    throw new Error(chrome.i18n.getMessage("errorNoCookies"));
  }
  if (!merged.some((c) => AUTH_COOKIE_NAMES.includes(c.name))) {
    throw new Error(chrome.i18n.getMessage("errorNoAuthCookies"));
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
      throw new Error(payload?.error || chrome.i18n.getMessage("errorBridgeRequest"));
    }

    return payload;
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error(chrome.i18n.getMessage("errorNoResponse"));
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

function notifyTab(tabId, message) {
  if (tabId == null) return;
  chrome.tabs.sendMessage(tabId, { type: "lfmusic:progress", message }).catch(() => {});
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type !== "lfmusic:connect") return false;

  const tabId = sender.tab?.id ?? null;
  (async () => {
    try {
      notifyTab(tabId, chrome.i18n.getMessage("statusPinging"));
      await callBridge("/bridge/ping");
      notifyTab(tabId, chrome.i18n.getMessage("statusCollecting"));
      const cookies = await collectCookies();
      notifyTab(tabId, chrome.i18n.getMessage("statusSending"));
      await callBridge("/bridge/import-session", {
        method: "POST",
        body: JSON.stringify({ cookies }),
      });
      sendResponse({ ok: true, message: chrome.i18n.getMessage("statusSuccess") });
    } catch (error) {
      sendResponse({
        ok: false,
        message: error instanceof Error ? error.message : chrome.i18n.getMessage("errorGeneric"),
      });
    }
  })();

  return true; // keep the message channel open for the async sendResponse above
});

// No default_popup: clicking the toolbar icon toggles an in-page panel
// (content.js) instead, so it can sit flush in the corner of the page with
// real rounded corners instead of the browser's square popup frame.
chrome.action.onClicked.addListener((tab) => {
  if (tab.id == null) return;
  chrome.scripting.executeScript({ target: { tabId: tab.id }, files: ["content.js"] }).catch(() => {});
});
