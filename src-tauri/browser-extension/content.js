// Injected on demand by background.js (chrome.action.onClicked), re-injected
// on every icon click. Idempotent: a second click toggles the existing panel
// instead of building a duplicate.
(function () {
  if (window.__lfmusicBridge) {
    window.__lfmusicBridge.toggle();
    return;
  }

  const t = (key) => chrome.i18n.getMessage(key) || key;

  const host = document.createElement("div");
  host.id = "lfmusic-bridge-host";
  // `all: initial` first, so nothing the host page inherits (font-size,
  // line-height, weird resets) leaks into elements before our own CSS
  // applies — then re-assert the positioning that reset would have undone.
  host.style.all = "initial";
  host.style.position = "fixed";
  host.style.top = "16px";
  host.style.right = "16px";
  host.style.zIndex = "2147483647";

  const shadow = host.attachShadow({ mode: "open" });
  shadow.innerHTML = `
    <style>
      :host { all: initial; }
      * { margin: 0; padding: 0; box-sizing: border-box; }
      .card {
        position: relative;
        width: 320px;
        font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        color: hsla(0, 0%, 100%, 0.92);
        border-radius: 16px;
        overflow: hidden;
        box-shadow: 0 12px 40px hsla(0, 0%, 0%, 0.45), 0 0 0 1px hsla(0, 0%, 100%, 0.06);
        background:
          radial-gradient(120% 90% at 12% -20%, hsla(258, 65%, 45%, 0.35), transparent 60%),
          linear-gradient(180deg, hsla(258, 45%, 15%, 1) 0%, hsl(258, 50%, 8%) 75%);
        padding: 18px 18px 16px;
      }
      .close {
        position: absolute;
        top: 10px;
        right: 10px;
        width: 24px;
        height: 24px;
        display: flex;
        align-items: center;
        justify-content: center;
        border: none;
        border-radius: 50%;
        background: hsla(0, 0%, 0%, 0.3);
        color: hsla(0, 0%, 100%, 0.75);
        cursor: pointer;
        transition: background-color .2s ease, transform .2s ease;
      }
      .close:hover { background: hsla(0, 0%, 0%, 0.5); }
      .close:active { transform: scale(0.92); }
      .head { display: flex; align-items: center; gap: 10px; margin: 0 22px 14px 0; }
      .logo { width: 34px; height: 34px; border-radius: 9px; flex-shrink: 0; box-shadow: 0 2px 10px hsla(0, 0%, 0%, 0.4); }
      .headText h1 {
        font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        font-weight: 800; font-size: 15px; line-height: 1.2; color: #fff;
      }
      .subtitle { margin-top: 2px; font-size: 11px; font-weight: 600; letter-spacing: 0.01em; color: hsla(0, 0%, 100%, 0.5); }
      .desc { margin: 0 0 16px; font-size: 12.5px; line-height: 1.55; font-weight: 500; color: hsla(0, 0%, 100%, 0.62); }
      .btn {
        position: relative; display: block; width: 100%; padding: 10px 14px; border: none;
        border-radius: 999px; font-family: inherit; font-size: 13px; font-weight: 700;
        text-align: center; cursor: pointer;
        transition: background-color .2s ease, color .2s ease, transform .2s ease, opacity .2s ease;
      }
      .btn:active:not(:disabled) { transform: scale(0.97); }
      .btn.primary { background-color: hsla(0, 0%, 100%, 0.95); color: hsl(258, 55%, 20%); }
      .btn.primary:hover:not(:disabled) { background-color: #fff; }
      .btn.primary:disabled { cursor: default; opacity: 0.7; }
      .btn.ghost { margin-top: 8px; background-color: hsla(0, 0%, 100%, 0.06); color: hsla(0, 0%, 100%, 0.75); }
      .btn.ghost:hover { background-color: hsla(0, 0%, 100%, 0.12); color: #fff; }
      .btnLabel { opacity: 1; transition: opacity .18s ease; }
      .btn.loading .btnLabel { opacity: 0; }
      .btnSpinner {
        position: absolute; top: 50%; left: 50%; width: 14px; height: 14px; margin: -7px 0 0 -7px;
        border-radius: 50%; border: 2px solid hsla(258, 55%, 20%, 0.25); border-top-color: hsl(258, 55%, 20%);
        opacity: 0; transition: opacity .18s ease; animation: spin 0.8s linear infinite;
      }
      .btn.loading .btnSpinner { opacity: 1; }
      @keyframes spin { to { transform: rotate(360deg); } }
      .status { margin-top: 12px; min-height: 16px; font-size: 11.5px; font-weight: 600; line-height: 1.5; color: hsla(0, 0%, 100%, 0.5); transition: color .2s ease; }
      .status.success { color: #86efac; }
      .status.error { color: #fda4af; }
      .footnote {
        display: flex; align-items: flex-start; gap: 6px; margin: 14px 0 0; padding-top: 12px;
        border-top: 1px solid hsla(0, 0%, 100%, 0.08); font-size: 10.5px; line-height: 1.5;
        font-weight: 600; color: hsla(0, 0%, 100%, 0.35);
      }
      .footnote svg { flex-shrink: 0; margin-top: 2px; }
    </style>
    <div class="card" role="dialog" aria-modal="true">
      <button class="close" type="button" aria-label="Close">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
      <div class="head">
        <img class="logo" src="${chrome.runtime.getURL("icons/icon48.png")}" alt="" />
        <div class="headText">
          <h1>LFMusic Bridge</h1>
          <div class="subtitle">${t("subtitle")}</div>
        </div>
      </div>
      <p class="desc">${t("desc")}</p>
      <button id="connect" class="btn primary" type="button">
        <span class="btnLabel">${t("connectLabel")}</span>
        <span class="btnSpinner" aria-hidden="true"></span>
      </button>
      <button id="open-music" class="btn ghost" type="button">${t("openMusicLabel")}</button>
      <div id="status" class="status"></div>
      <p class="footnote">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="5" y="11" width="14" height="9" rx="2" />
          <path d="M8 11V7a4 4 0 0 1 8 0v4" />
        </svg>
        <span>${t("footnote")}</span>
      </p>
    </div>
  `;

  document.documentElement.appendChild(host);

  const statusEl = shadow.getElementById("status");
  const connectButton = shadow.getElementById("connect");
  const openMusicButton = shadow.getElementById("open-music");
  const closeButton = shadow.querySelector(".close");

  function setStatus(message, tone = "") {
    statusEl.textContent = message;
    statusEl.className = `status ${tone}`.trim();
  }

  function onOutsideClick(e) {
    // Click events that originate inside the shadow tree are retargeted to
    // `host` itself once they reach document-level listeners, so this check
    // covers everything inside the panel without piercing the shadow root.
    if (!host.contains(e.target)) hide();
  }
  function onKeydown(e) {
    if (e.key === "Escape") hide();
  }

  function show() {
    host.style.display = "";
    document.addEventListener("mousedown", onOutsideClick, true);
    document.addEventListener("keydown", onKeydown, true);
  }
  function hide() {
    host.style.display = "none";
    document.removeEventListener("mousedown", onOutsideClick, true);
    document.removeEventListener("keydown", onKeydown, true);
  }
  function toggle() {
    if (host.style.display === "none") show();
    else hide();
  }

  closeButton?.addEventListener("click", hide);

  connectButton?.addEventListener("click", () => {
    connectButton.disabled = true;
    connectButton.classList.add("loading");
    setStatus(t("statusPinging"));

    chrome.runtime.sendMessage({ type: "lfmusic:connect" }, (response) => {
      connectButton.disabled = false;
      connectButton.classList.remove("loading");
      if (!response) {
        setStatus(t("errorGeneric"), "error");
        return;
      }
      setStatus(response.message, response.ok ? "success" : "error");
    });
  });

  openMusicButton?.addEventListener("click", () => {
    window.open("https://music.youtube.com", "_blank", "noopener");
  });

  chrome.runtime.onMessage.addListener((message) => {
    if (message?.type === "lfmusic:progress") setStatus(message.message);
  });

  window.__lfmusicBridge = { toggle, show, hide };
  show();
})();
