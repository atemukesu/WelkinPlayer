(function () {
  "use strict";

  var tauri = window.__TAURI__ || null;
  var win = null;
  if (tauri && tauri.window && typeof tauri.window.getCurrentWindow === "function") {
    try { win = tauri.window.getCurrentWindow(); } catch (error) { win = null; }
  }

  var GENERIC = {
    "serif": 1, "sans-serif": 1, "monospace": 1, "cursive": 1, "fantasy": 1,
    "system-ui": 1, "ui-serif": 1, "ui-sans-serif": 1, "ui-monospace": 1,
    "ui-rounded": 1, "emoji": 1, "math": 1, "fangsong": 1,
  };

  function cssFontFamily(families) {
    if (!families || !families.length) return "";
    return families.map(function (family) { return String(family).trim(); }).filter(Boolean).map(function (family) {
      if (GENERIC[family.toLowerCase()]) return family;
      return '"' + family.replace(/\\/g, "\\\\").replace(/"/g, '\\"') + '"';
    }).join(", ");
  }

  var state = {
    settings: null,
    lines: [],
    positionMs: 0,
    playing: false,
    visible: true,
    lastTickAt: 0,
    activeIndex: -1,
    activeIndices: [],
    activeSig: "",
    currentNode: null,
    currentKey: "",
    raf: 0,
  };

  var stage = document.getElementById("stage");
  var current = document.getElementById("current");
  var body = document.body;

  function syncAppTheme() {
    var root = document.documentElement.style;
    var dark = localStorage.getItem("welkin-theme") !== "light";
    var accent = localStorage.getItem("welkin-accent") || "amber";
    var accents = {
      amber: "#f0a500", orange: "#ff6a00", cyan: "#00b8d4",
      red: "#e63946", green: "#16c79a", blue: "#3d7dff",
    };
    root.setProperty("--ui-accent", accents[accent] || accents.amber);
    root.setProperty("--surface", dark ? "rgba(19, 21, 25, 0.82)" : "rgba(243, 243, 239, 0.88)");
    root.setProperty("--line", dark ? "rgba(255,255,255,.18)" : "rgba(0,0,0,.16)");
    root.setProperty("--line-strong", dark ? "rgba(255,255,255,.34)" : "rgba(0,0,0,.34)");
    root.setProperty("--dim", dark ? "rgba(255,255,255,.58)" : "rgba(16,18,22,.58)");
  }

  function applySettings(settings) {
    if (!settings) return;
    syncAppTheme();
    state.settings = settings;
    var root = document.documentElement.style;
    root.setProperty("--pad-x", (settings.paddingX == null ? 28 : settings.paddingX) + "px");
    root.setProperty("--pad-y", (settings.paddingY == null ? 18 : settings.paddingY) + "px");
    root.setProperty("--opacity", Math.max(0, Math.min(1, settings.opacity / 100)));
    root.setProperty("--font-size", settings.fontSize + "px");
    root.setProperty("--translation-size", settings.translationSize + "px");
    root.setProperty("--font-weight", String(settings.fontWeight));
    root.setProperty("--text-color", settings.textColor);
    root.setProperty("--active-color", settings.activeColor);
    root.setProperty("--translation-color", settings.translationColor);
    root.setProperty("--stroke-width", settings.stroke ? "1px" : "0px");
    root.setProperty("--stroke-color", settings.stroke ? "rgba(0,0,0,.55)" : "transparent");
    root.setProperty("--align", settings.align || "center");
    body.style.fontFamily = cssFontFamily(settings.fontFamilies) || "";
    body.classList.toggle("karaoke", !!settings.karaoke);
    applyLocked(!!settings.locked);
    if (win) {
      if (typeof win.setAlwaysOnTop === "function") win.setAlwaysOnTop(!!settings.alwaysOnTop).catch(function () {});
      if (typeof win.setSkipTaskbar === "function") win.setSkipTaskbar(!!settings.skipTaskbar).catch(function () {});
    }
    if (state.currentNode) updateProgress(state.positionMs);
  }

  function applyLocked(locked) {
    body.classList.toggle("locked", locked);
    var button = document.getElementById("lock");
    if (button) {
      button.title = locked ? "解锁" : "锁定";
      button.setAttribute("aria-label", locked ? "解锁" : "锁定");
      button.innerHTML = locked
        ? '<svg viewBox="0 0 24 24"><rect x="5" y="10" width="14" height="10" rx="1"/><path d="M8 10V7a4 4 0 0 1 8 0"/></svg>'
        : '<svg viewBox="0 0 24 24"><rect x="5" y="10" width="14" height="10" rx="1"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></svg>';
    }
    if (win && typeof win.setIgnoreCursorEvents === "function") win.setIgnoreCursorEvents(locked).catch(function () {});
    if (window.AndroidDesktopLyric && window.AndroidDesktopLyric.setLocked) {
      try { window.AndroidDesktopLyric.setLocked(locked); } catch (error) {}
    }
  }

  function setVisibility(visible) {
    body.classList.toggle("is-hidden", !visible);
    if (!visible) stopRaf();
    if (win && typeof win.show === "function") {
      if (visible) win.show().catch(function () {}); else win.hide().catch(function () {});
    }
    if (window.AndroidDesktopLyric && window.AndroidDesktopLyric.setVisible) {
      try { window.AndroidDesktopLyric.setVisible(visible); } catch (error) {}
    }
  }

  function activeBlock() {
    var indices = state.activeIndices && state.activeIndices.length ? state.activeIndices : state.activeIndex >= 0 ? [state.activeIndex] : [];
    var result = [];
    for (var i = 0; i < indices.length; i++) {
      var line = state.lines[indices[i]];
      if (line) result.push({ index: indices[i], line: line });
    }
    return result;
  }

  function makeWords(line, secondary) {
    var words = Array.isArray(line.words) && line.words.length ? line.words : [{ text: line.text || "", start: 0, end: 0 }];
    var fragment = document.createDocumentFragment();
    var elements = [];
    for (var i = 0; i < words.length; i++) {
      var element = document.createElement("span");
      element.className = "word";
      if (secondary) element.classList.add("is-unplayed");
      element.textContent = words[i].text;
      fragment.appendChild(element);
      elements.push(element);
    }
    return { fragment: fragment, elements: elements, data: words };
  }

  function makeLine(line, secondary) {
    var paragraph = document.createElement("div");
    paragraph.className = "lyric-line " + (secondary ? "secondary" : "primary");
    if (line.isBG) paragraph.classList.add("line-bg");
    var words = makeWords(line, secondary);
    paragraph.appendChild(words.fragment);
    return { el: paragraph, words: words.elements, wordData: words.data };
  }

  function renderCurrent(animate) {
    var block = activeBlock();
    var primaryEntry = null;
    for (var i = 0; i < block.length; i++) {
      if (block[i].index === state.activeIndex) { primaryEntry = block[i]; break; }
    }
    if (!primaryEntry) primaryEntry = block[0] || null;
    var key = block.map(function (entry) { return entry.index; }).join(",") + ":" + (primaryEntry ? primaryEntry.index : -1);
    if (key === state.currentKey && state.currentNode) return;
    state.currentKey = key;

    var old = state.currentNode;
    if (old && animate) {
      old.classList.add("is-leaving");
      window.setTimeout(function () { if (old.parentNode === current) old.parentNode.removeChild(old); }, 190);
    } else if (old && old.parentNode === current) {
      current.removeChild(old);
    }

    if (!primaryEntry) { state.currentNode = null; return; }
    var blockElement = document.createElement("div");
    blockElement.className = "lyric-block";
    var primary = makeLine(primaryEntry.line, false);
    blockElement.appendChild(primary.el);

    for (var j = 0; j < block.length; j++) {
      if (block[j].index === primaryEntry.index) continue;
      var companion = makeLine(block[j].line, true);
      blockElement.appendChild(companion.el);
    }

    if (state.settings && state.settings.translation && primaryEntry.line.translation) {
      var translation = document.createElement("p");
      translation.className = "translation";
      translation.textContent = primaryEntry.line.translation;
      blockElement.appendChild(translation);
    }
    if (primaryEntry.line.roman) {
      var roman = document.createElement("p");
      roman.className = "roman";
      roman.textContent = primaryEntry.line.roman;
      blockElement.appendChild(roman);
    }

    blockElement.classList.toggle("is-entering", animate);
    current.appendChild(blockElement);
    state.currentNode = blockElement;
    state.currentWords = primary.words;
    state.currentWordData = primary.wordData;
    if (animate) window.setTimeout(function () { blockElement.classList.remove("is-entering"); }, 240);
    updateProgress(state.positionMs);
  }

  function nowMs() {
    return state.playing ? state.positionMs + (performance.now() - state.lastTickAt) : state.positionMs;
  }

  function updateProgress(position) {
    if (!state.currentNode || !state.currentWords || !state.settings) return;
    for (var i = 0; i < state.currentWords.length; i++) {
      var word = state.currentWordData[i];
      var progress = 0;
      if (word) progress = Math.max(0, Math.min(1, (position - word.start) / Math.max(1, word.end - word.start)));
      state.currentWords[i].style.setProperty("--p", progress * 100 + "%");
      state.currentWords[i].classList.toggle("is-unplayed", progress <= 0);
    }
  }

  function frame() {
    state.raf = 0;
    if (!state.visible || !state.settings || !state.settings.karaoke || !state.playing) return;
    updateProgress(nowMs());
    state.raf = requestAnimationFrame(frame);
  }

  function ensureRaf() { if (!state.raf) state.raf = requestAnimationFrame(frame); }
  function stopRaf() { if (state.raf) { cancelAnimationFrame(state.raf); state.raf = 0; } }

  function setLines(payload) {
    if (!payload) return;
    state.lines = Array.isArray(payload.lines) ? payload.lines : [];
    if (payload.settings) applySettings(payload.settings);
    state.currentKey = "";
    renderCurrent(false);
  }

  function setSettings(settings) {
    applySettings(settings);
    state.currentKey = "";
    renderCurrent(false);
  }

  function tick(payload) {
    if (!payload) return;
    state.positionMs = payload.positionMs || 0;
    state.lastTickAt = performance.now();
    state.playing = !!payload.playing;
    state.activeIndex = Number.isFinite(payload.activeIndex) ? payload.activeIndex : -1;
    state.activeIndices = Array.isArray(payload.activeIndices) ? payload.activeIndices : [];
    var visible = payload.visible !== false;
    if (visible !== state.visible) { state.visible = visible; setVisibility(visible); }
    var sig = state.activeIndex + "|" + state.activeIndices.join(",");
    if (sig !== state.activeSig) {
      state.activeSig = sig;
      renderCurrent(true);
    }
    if (state.playing && state.settings && state.settings.karaoke && state.visible) ensureRaf();
    else { stopRaf(); updateProgress(state.positionMs); }
  }

  if (tauri && tauri.event) {
    tauri.event.listen("desktop-lyric:load", function (event) { setLines(event.payload); });
    tauri.event.listen("desktop-lyric:settings", function (event) { setSettings(event.payload); });
    tauri.event.listen("desktop-lyric:tick", function (event) { tick(event.payload); });
  }
  window.__welkinOverlay = { load: setLines, settings: setSettings, tick: tick };

  function readGeometry() {
    try { return JSON.parse(localStorage.getItem("welkin-desktop-lyric-geometry") || "null"); } catch (error) { return null; }
  }

  if (win) {
    var geometry = readGeometry();
    if (geometry && Number.isFinite(geometry.x) && Number.isFinite(geometry.y)) {
      if (typeof win.setPosition === "function") {
        try { var LogicalPosition = tauri.window.LogicalPosition; win.setPosition(LogicalPosition ? new LogicalPosition(geometry.x, geometry.y) : { x: geometry.x, y: geometry.y }).catch(function () {}); } catch (error) {}
      }
      if (geometry.width && geometry.height && typeof win.setSize === "function") {
        try { var LogicalSize = tauri.window.LogicalSize; win.setSize(LogicalSize ? new LogicalSize(geometry.width, geometry.height) : { width: geometry.width, height: geometry.height }).catch(function () {}); } catch (error) {}
      }
    }
    var saveGeometry = function () {
      Promise.all([
        typeof win.outerPosition === "function" ? win.outerPosition() : null,
        typeof win.outerSize === "function" ? win.outerSize() : null,
        typeof win.scaleFactor === "function" ? win.scaleFactor() : 1,
      ]).then(function (values) {
        var pos = values[0], size = values[1], scale = values[2] || 1;
        if (!pos || !size) return;
        try { localStorage.setItem("welkin-desktop-lyric-geometry", JSON.stringify({ x: pos.x / scale, y: pos.y / scale, width: size.width / scale, height: size.height / scale })); } catch (error) {}
      });
    };
    if (typeof win.onMoved === "function") win.onMoved(saveGeometry);
    if (typeof win.onResized === "function") win.onResized(saveGeometry);
  }

  function startDragging(event) {
    if (!win || typeof win.startDragging !== "function" || (event && event.button !== 0)) return;
    win.startDragging().catch(function () {});
  }

  document.getElementById("drag").addEventListener("pointerdown", startDragging);
  stage.addEventListener("pointerdown", function (event) {
    if (!state.settings || state.settings.locked || event.target.closest("button")) return;
    startDragging(event);
  });
  document.getElementById("close").addEventListener("click", function () { if (win && typeof win.close === "function") win.close().catch(function () {}); });
  document.getElementById("lock").addEventListener("click", function () { if (state.settings) { state.settings.locked = !state.settings.locked; applyLocked(state.settings.locked); } });

  function nudgeFont(delta) {
    if (!state.settings) return;
    state.settings.fontSize = Math.max(14, Math.min(96, state.settings.fontSize + delta));
    document.documentElement.style.setProperty("--font-size", state.settings.fontSize + "px");
  }
  document.getElementById("fontUp").addEventListener("click", function () { nudgeFont(2); });
  document.getElementById("fontDown").addEventListener("click", function () { nudgeFont(-2); });

  window.addEventListener("wheel", function (event) {
    if (!state.settings) return;
    event.preventDefault();
    if (state.settings.wheelAction === "fontSize") nudgeFont(event.deltaY < 0 ? 2 : -2);
    else if (state.settings.wheelAction === "opacity") {
      state.settings.opacity = Math.max(10, Math.min(100, state.settings.opacity + (event.deltaY < 0 ? 5 : -5)));
      document.documentElement.style.setProperty("--opacity", state.settings.opacity / 100);
    }
  }, { passive: false });
})();
