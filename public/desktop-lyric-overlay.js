(function () {
  "use strict";

  var tauri = window.__TAURI__ || null;
  var win = null;
  if (tauri && tauri.window && typeof tauri.window.getCurrentWindow === "function") {
    try {
      win = tauri.window.getCurrentWindow();
    } catch (error) {
      win = null;
    }
  }

  var GENERIC = {
    "serif": 1, "sans-serif": 1, "monospace": 1, "cursive": 1, "fantasy": 1,
    "system-ui": 1, "ui-serif": 1, "ui-sans-serif": 1, "ui-monospace": 1,
    "ui-rounded": 1, "emoji": 1, "math": 1, "fangsong": 1,
  };

  function cssFontFamily(families) {
    if (!families || !families.length) return "";
    return families
      .map(function (family) { return String(family).trim(); })
      .filter(Boolean)
      .map(function (family) {
        if (GENERIC[family.toLowerCase()]) return family;
        return '"' + family.replace(/\\/g, "\\\\").replace(/"/g, '\\"') + '"';
      })
      .join(", ");
  }

  function hexToRgba(color, opacity) {
    var hex = String(color || "").trim();
    var match = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex);
    if (!match) return hex;
    var value = match[1];
    if (value.length === 3) {
      value = value[0] + value[0] + value[1] + value[1] + value[2] + value[2];
    }
    var r = parseInt(value.slice(0, 2), 16);
    var g = parseInt(value.slice(2, 4), 16);
    var b = parseInt(value.slice(4, 6), 16);
    return "rgba(" + r + ", " + g + ", " + b + ", " + Math.max(0, Math.min(1, opacity)) + ")";
  }

  var state = {
    settings: null,
    lines: [],
    activeIndex: -1,
    activeIndices: [],
    positionMs: 0,
    playing: false,
    visible: true,
    renderedRange: "",
    wordEls: [],
  };

  var stage = document.getElementById("stage");
  var body = document.body;

  function applySettings(settings) {
    if (!settings) return;
    state.settings = settings;
    var root = document.documentElement.style;
    root.setProperty("--line-spacing", settings.lineSpacing + "px");
    root.setProperty("--pad-x", settings.paddingX + "px");
    root.setProperty("--pad-y", settings.paddingY + "px");
    root.setProperty("--radius", settings.borderRadius + "px");
    root.setProperty("--opacity", Math.max(0, Math.min(1, settings.opacity / 100)));
    root.setProperty("--font-size", settings.fontSize + "px");
    root.setProperty("--translation-size", settings.translationSize + "px");
    root.setProperty("--font-weight", String(settings.fontWeight));
    root.setProperty("--text-color", settings.textColor);
    root.setProperty("--active-color", settings.activeColor);
    root.setProperty("--translation-color", settings.translationColor);
    root.setProperty("--stroke-width", settings.strokeWidth + "px");
    root.setProperty("--stroke-color", settings.strokeColor);
    root.setProperty("--align", settings.align);
    root.setProperty(
      "--bg-color",
      settings.background ? hexToRgba(settings.backgroundColor, settings.backgroundOpacity / 100) : "transparent"
    );
    var stack = cssFontFamily(settings.fontFamilies);
    body.style.fontFamily = stack || "";
    body.classList.toggle("shadow", !!settings.shadow);
    body.classList.toggle("karaoke", !!settings.karaoke);
    applyLocked(!!settings.locked);
    if (win) {
      if (typeof win.setAlwaysOnTop === "function") win.setAlwaysOnTop(!!settings.alwaysOnTop).catch(function () {});
      if (typeof win.setSkipTaskbar === "function") win.setSkipTaskbar(!!settings.skipTaskbar).catch(function () {});
    }
    state.renderedRange = "";
    render();
  }

  function applyLocked(locked) {
    body.classList.toggle("locked", locked);
    var lockButton = document.getElementById("lock");
    if (lockButton) lockButton.textContent = locked ? "🔒" : "🔓";
    if (win && typeof win.setIgnoreCursorEvents === "function") {
      win.setIgnoreCursorEvents(locked).catch(function () {});
    }
    if (window.AndroidDesktopLyric && window.AndroidDesktopLyric.setLocked) {
      try { window.AndroidDesktopLyric.setLocked(locked); } catch (error) {}
    }
  }

  function setLines(payload) {
    if (!payload) return;
    state.lines = Array.isArray(payload.lines) ? payload.lines : [];
    if (payload.settings) applySettings(payload.settings);
    state.renderedRange = "";
    render();
  }

  function rangeFor() {
    var total = state.lines.length;
    if (!total) return null;
    var ctx = state.settings && Number.isFinite(state.settings.contextLines) ? state.settings.contextLines : 1;
    var anchor = state.activeIndices && state.activeIndices.length
      ? state.activeIndices
      : state.activeIndex >= 0
        ? [state.activeIndex]
        : [0];
    var min = anchor[0];
    var max = anchor[anchor.length - 1];
    for (var i = 0; i < anchor.length; i++) {
      if (anchor[i] < min) min = anchor[i];
      if (anchor[i] > max) max = anchor[i];
    }
    return {
      from: Math.max(0, min - ctx),
      to: Math.min(total - 1, max + ctx),
    };
  }

  function buildLine(line, index, activeSet) {
    if (!line && line !== "") return null;
    var wrapper = document.createElement("div");
    wrapper.className = "line";
    if (line.isBG) wrapper.classList.add("is-bg");
    if (line.isDuet) wrapper.classList.add("is-duet");
    if (activeSet.indexOf(index) >= 0) wrapper.classList.add("is-active");
    wrapper.dataset.index = String(index);

    var primary = document.createElement("p");
    primary.className = "primary";
    var wordEls = [];
    var words = Array.isArray(line.words) && line.words.length ? line.words : [{ text: line.text || "", start: 0, end: 0 }];
    for (var w = 0; w < words.length; w++) {
      var span = document.createElement("span");
      span.className = "word";
      span.textContent = words[w].text;
      primary.appendChild(span);
      wordEls.push(span);
    }
    wrapper.appendChild(primary);
    state.wordEls[index] = wordEls;

    if (state.settings && state.settings.translation && line.translation) {
      var translation = document.createElement("p");
      translation.className = "translation";
      translation.textContent = line.translation;
      wrapper.appendChild(translation);
    }
    if (line.roman) {
      var roman = document.createElement("p");
      roman.className = "roman";
      roman.textContent = line.roman;
      wrapper.appendChild(roman);
    }
    return wrapper;
  }

  function render() {
    var range = rangeFor();
    if (!range) {
      if (state.renderedRange !== "") {
        state.renderedRange = "";
        stage.textContent = "";
        state.wordEls = [];
      }
      return;
    }
    var key = range.from + ":" + range.to + ":" + state.activeIndices.join(",") + ":" + state.activeIndex;
    if (key === state.renderedRange) return;
    state.renderedRange = key;

    var frag = document.createDocumentFragment();
    state.wordEls = [];
    for (var i = range.from; i <= range.to; i++) {
      var el = buildLine(state.lines[i], i, state.activeIndices);
      if (el) frag.appendChild(el);
    }
    stage.textContent = "";
    stage.appendChild(frag);
  }

  function updateProgress() {
    if (!state.settings || !state.settings.karaoke) return;
    var pos = state.positionMs;
    var indices = state.activeIndices && state.activeIndices.length
      ? state.activeIndices
      : state.activeIndex >= 0
        ? [state.activeIndex]
        : [];
    for (var k = 0; k < indices.length; k++) {
      var index = indices[k];
      var line = state.lines[index];
      var els = state.wordEls[index];
      if (!line || !els) continue;
      var words = Array.isArray(line.words) && line.words.length ? line.words : [];
      for (var w = 0; w < els.length; w++) {
        var word = words[w];
        var progress = 0;
        if (word) {
          var duration = Math.max(1, word.end - word.start);
          progress = Math.max(0, Math.min(1, (pos - word.start) / duration));
        }
        els[w].style.setProperty("--p", progress * 100 + "%");
      }
    }
  }

  function setVisibility(visible) {
    body.classList.toggle("is-hidden", !visible);
    if (win && typeof win.show === "function") {
      if (visible) win.show().catch(function () {});
      else win.hide().catch(function () {});
    }
    if (window.AndroidDesktopLyric && window.AndroidDesktopLyric.setVisible) {
      try { window.AndroidDesktopLyric.setVisible(visible); } catch (error) {}
    }
  }

  function tick(payload) {
    if (!payload) return;
    state.positionMs = payload.positionMs || 0;
    state.playing = !!payload.playing;
    state.activeIndex = Number.isFinite(payload.activeIndex) ? payload.activeIndex : -1;
    state.activeIndices = Array.isArray(payload.activeIndices) ? payload.activeIndices : [];
    var visible = payload.visible !== false;
    if (visible !== state.visible) {
      state.visible = visible;
      setVisibility(visible);
    }
    render();
    updateProgress();
  }

  // Desktop: state arrives as Tauri events.
  if (tauri && tauri.event) {
    tauri.event.listen("desktop-lyric:load", function (event) { setLines(event.payload); });
    tauri.event.listen("desktop-lyric:settings", function (event) { applySettings(event.payload); });
    tauri.event.listen("desktop-lyric:tick", function (event) { tick(event.payload); });
  }

  // Android: Kotlin (`evaluateJavascript`) calls these directly.
  window.__welkinOverlay = { load: setLines, settings: applySettings, tick: tick };

  // Restore the last desktop geometry (shared origin with the main window).
  function readGeometry() {
    try {
      return JSON.parse(localStorage.getItem("welkin-desktop-lyric-geometry") || "null");
    } catch (error) {
      return null;
    }
  }

  if (win) {
    var geometry = readGeometry();
    if (geometry && Number.isFinite(geometry.x) && Number.isFinite(geometry.y)) {
      if (typeof win.setPosition === "function") {
        try {
          var LogicalPosition = tauri.window.LogicalPosition;
          win.setPosition(LogicalPosition ? new LogicalPosition(geometry.x, geometry.y) : { x: geometry.x, y: geometry.y }).catch(function () {});
        } catch (error) {}
      }
      if (geometry.width && geometry.height && typeof win.setSize === "function") {
        try {
          var LogicalSize = tauri.window.LogicalSize;
          win.setSize(LogicalSize ? new LogicalSize(geometry.width, geometry.height) : { width: geometry.width, height: geometry.height }).catch(function () {});
        } catch (error) {}
      }
    }
    var saveGeometry = function () {
      Promise.all([
        typeof win.outerPosition === "function" ? win.outerPosition() : null,
        typeof win.outerSize === "function" ? win.outerSize() : null,
        typeof win.scaleFactor === "function" ? win.scaleFactor() : 1,
      ]).then(function (values) {
        var pos = values[0];
        var size = values[1];
        var scale = values[2] || 1;
        if (!pos || !size) return;
        try {
          localStorage.setItem(
            "welkin-desktop-lyric-geometry",
            JSON.stringify({
              x: pos.x / scale,
              y: pos.y / scale,
              width: size.width / scale,
              height: size.height / scale,
            })
          );
        } catch (error) {}
      });
    };
    if (typeof win.onMoved === "function") win.onMoved(saveGeometry);
    if (typeof win.onResized === "function") win.onResized(saveGeometry);
  }

  // Toolbar wiring.
  var dragButton = document.getElementById("drag");
  if (dragButton) {
    dragButton.addEventListener("pointerdown", function (event) {
      if (!win || typeof win.startDragging !== "function") return;
      if (event.button !== 0) return;
      win.startDragging().catch(function () {});
    });
  }
  document.getElementById("close").addEventListener("click", function () {
    if (win && typeof win.close === "function") win.close().catch(function () {});
  });
  document.getElementById("lock").addEventListener("click", function () {
    if (!state.settings) return;
    state.settings.locked = !state.settings.locked;
    applyLocked(state.settings.locked);
  });
  function nudgeFont(delta) {
    if (!state.settings) return;
    state.settings.fontSize = Math.max(14, Math.min(96, state.settings.fontSize + delta));
    document.documentElement.style.setProperty("--font-size", state.settings.fontSize + "px");
  }
  document.getElementById("fontUp").addEventListener("click", function () { nudgeFont(2); });
  document.getElementById("fontDown").addEventListener("click", function () { nudgeFont(-2); });

  // Mouse-wheel adjusts font size or opacity (desktop).
  window.addEventListener(
    "wheel",
    function (event) {
      if (!state.settings) return;
      event.preventDefault();
      var action = state.settings.wheelAction;
      if (action === "fontSize") nudgeFont(event.deltaY < 0 ? 2 : -2);
      else if (action === "opacity") {
        state.settings.opacity = Math.max(10, Math.min(100, state.settings.opacity + (event.deltaY < 0 ? 5 : -5)));
        document.documentElement.style.setProperty("--opacity", state.settings.opacity / 100);
      }
    },
    { passive: false }
  );
})();
