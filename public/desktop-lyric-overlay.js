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

  /** Fraction of the viewport the active block is aligned to. */
  var ANCHOR = 0.5;
  /** Extra lines kept above/below the active block to animate without a rebuild. */
  var WINDOW_BUFFER = 10;

  var state = {
    settings: null,
    lines: [],
    activeIndex: -1,
    activeIndices: [],
    activeSig: "",
    positionMs: 0,
    playing: false,
    visible: true,
    lastTickAt: 0,
    windowFrom: -1,
    windowTo: -1,
    lineEls: {},
    raf: 0,
  };

  var stage = document.getElementById("stage");
  var track = document.getElementById("track");
  var body = document.body;

  function applySettings(settings) {
    if (!settings) return;
    state.settings = settings;
    var root = document.documentElement.style;
    root.setProperty("--line-spacing", settings.lineSpacing + "px");
    root.setProperty("--pad-x", settings.paddingX + "px");
    root.setProperty("--pad-y", settings.paddingY + "px");
    root.setProperty("--opacity", Math.max(0, Math.min(1, settings.opacity / 100)));
    root.setProperty("--font-size", settings.fontSize + "px");
    root.setProperty("--translation-size", settings.translationSize + "px");
    root.setProperty("--font-weight", String(settings.fontWeight));
    root.setProperty("--text-color", settings.textColor);
    root.setProperty("--active-color", settings.activeColor);
    root.setProperty("--translation-color", settings.translationColor);
    // The stroke is centred on the glyph and the fill layer hides its inner
    // half, so 2px yields a 1px outline *outside* the text.
    root.setProperty("--stroke-width", settings.stroke ? "2px" : "0px");
    root.setProperty("--align", settings.align);
    var stack = cssFontFamily(settings.fontFamilies);
    body.style.fontFamily = stack || "";
    body.classList.toggle("karaoke", !!settings.karaoke);
    applyLocked(!!settings.locked);
    if (win) {
      if (typeof win.setAlwaysOnTop === "function") win.setAlwaysOnTop(!!settings.alwaysOnTop).catch(function () {});
      if (typeof win.setSkipTaskbar === "function") win.setSkipTaskbar(!!settings.skipTaskbar).catch(function () {});
    }
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

  function setVisibility(visible) {
    body.classList.toggle("is-hidden", !visible);
    if (!visible) stopRaf();
    if (win && typeof win.show === "function") {
      if (visible) win.show().catch(function () {});
      else win.hide().catch(function () {});
    }
    if (window.AndroidDesktopLyric && window.AndroidDesktopLyric.setVisible) {
      try { window.AndroidDesktopLyric.setVisible(visible); } catch (error) {}
    }
  }

  /** Bounds of the simultaneously-active lines, or null. */
  function activeBlock() {
    var indices = state.activeIndices && state.activeIndices.length
      ? state.activeIndices
      : state.activeIndex >= 0
        ? [state.activeIndex]
        : [];
    if (!indices.length) return null;
    var min = indices[0];
    var max = indices[0];
    for (var i = 1; i < indices.length; i++) {
      if (indices[i] < min) min = indices[i];
      if (indices[i] > max) max = indices[i];
    }
    return { min: min, max: max };
  }

  function anchorIndex() {
    var block = activeBlock();
    if (block) return Math.round((block.min + block.max) / 2);
    return 0;
  }

  function buildLine(line) {
    var wrapper = document.createElement("div");
    wrapper.className = "line";
    if (line.isBG) wrapper.classList.add("is-bg");
    if (line.isDuet) wrapper.classList.add("is-duet");

    var primary = document.createElement("p");
    primary.className = "primary";
    var words = Array.isArray(line.words) && line.words.length ? line.words : [{ text: line.text || "", start: 0, end: 0 }];

    // Outline layer (behind) and fill layer (front) share the exact same word
    // spans, so they wrap identically and the fill hides the stroke's inner half.
    var outline = document.createElement("span");
    outline.className = "outline";
    var fill = document.createElement("span");
    fill.className = "fill";
    var wordEls = [];
    for (var w = 0; w < words.length; w++) {
      var outlineWord = document.createElement("span");
      outlineWord.className = "word";
      outlineWord.textContent = words[w].text;
      outline.appendChild(outlineWord);

      var span = document.createElement("span");
      span.className = "word";
      span.textContent = words[w].text;
      fill.appendChild(span);
      wordEls.push(span);
    }
    primary.appendChild(outline);
    primary.appendChild(fill);
    wrapper.appendChild(primary);

    var node = { el: wrapper, words: wordEls, wordData: words, active: false };

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
    return node;
  }

  function resetWindow() {
    state.windowFrom = -1;
    state.windowTo = -1;
    state.lineEls = {};
    track.textContent = "";
  }

  function buildWindow(center) {
    var total = state.lines.length;
    var ctx = state.settings && Number.isFinite(state.settings.contextLines) ? state.settings.contextLines : 1;
    var block = activeBlock();
    var span = block ? block.max - block.min : 0;
    var radius = Math.max(ctx + WINDOW_BUFFER, Math.ceil(span / 2) + ctx + 4);
    var from = Math.max(0, center - radius);
    var to = Math.min(total - 1, center + radius);

    var frag = document.createDocumentFragment();
    var els = {};
    for (var i = from; i <= to; i++) {
      var node = buildLine(state.lines[i]);
      frag.appendChild(node.el);
      els[i] = node;
    }
    track.textContent = "";
    track.appendChild(frag);
    state.windowFrom = from;
    state.windowTo = to;
    state.lineEls = els;
  }

  /** Rebuild the DOM window only when the active block nears its edge. */
  function ensureWindow() {
    var total = state.lines.length;
    if (!total) {
      if (state.windowFrom !== -1) resetWindow();
      return false;
    }
    if (state.windowFrom < 0) {
      buildWindow(anchorIndex());
      return true;
    }
    var ctx = state.settings && Number.isFinite(state.settings.contextLines) ? state.settings.contextLines : 1;
    var margin = ctx + 3;
    var center = anchorIndex();
    if (center < state.windowFrom + margin || center > state.windowTo - margin) {
      buildWindow(center);
      return true;
    }
    return false;
  }

  function applyActive() {
    var set = {};
    var indices = state.activeIndices || [];
    for (var i = 0; i < indices.length; i++) set[indices[i]] = true;
    if (state.activeIndex >= 0) set[state.activeIndex] = true;
    for (var index in state.lineEls) {
      var node = state.lineEls[index];
      var active = !!set[index];
      if (node.active !== active) {
        node.active = active;
        node.el.classList.toggle("is-active", active);
      }
    }
  }

  function activeEls() {
    var indices = state.activeIndices && state.activeIndices.length
      ? state.activeIndices
      : state.activeIndex >= 0
        ? [state.activeIndex]
        : [];
    var els = [];
    for (var i = 0; i < indices.length; i++) {
      var node = state.lineEls[indices[i]];
      if (node) els.push(node.el);
    }
    return els;
  }

  function layout(animate) {
    var els = activeEls();
    if (!els.length) return;
    var top = Infinity;
    var bottom = -Infinity;
    for (var i = 0; i < els.length; i++) {
      top = Math.min(top, els[i].offsetTop);
      bottom = Math.max(bottom, els[i].offsetTop + els[i].offsetHeight);
    }
    var center = (top + bottom) / 2;
    var viewHeight = stage.clientHeight;
    var maxOffset = Math.max(0, track.offsetHeight - viewHeight);
    var target = Math.min(maxOffset, Math.max(0, center - viewHeight * ANCHOR));

    if (animate) {
      track.style.transition = "";
      track.style.transform = "translate3d(0," + -target + "px,0)";
    } else {
      track.style.transition = "none";
      track.style.transform = "translate3d(0," + -target + "px,0)";
      // Re-enable the transition on the next frame without animating this jump.
      void track.offsetHeight;
      requestAnimationFrame(function () {
        track.style.transition = "";
      });
    }
  }

  function refreshActive(animate) {
    var rebuilt = ensureWindow();
    applyActive();
    layout(animate && !rebuilt);
  }

  function nowMs() {
    if (!state.playing) return state.positionMs;
    return state.positionMs + (performance.now() - state.lastTickAt);
  }

  function updateProgress(position) {
    if (!state.settings || !state.settings.karaoke) return;
    var indices = state.activeIndices && state.activeIndices.length
      ? state.activeIndices
      : state.activeIndex >= 0
        ? [state.activeIndex]
        : [];
    for (var k = 0; k < indices.length; k++) {
      var node = state.lineEls[indices[k]];
      if (!node) continue;
      var words = node.wordData;
      for (var w = 0; w < node.words.length; w++) {
        var word = words[w];
        var progress = 0;
        if (word) {
          var duration = Math.max(1, word.end - word.start);
          progress = Math.max(0, Math.min(1, (position - word.start) / duration));
        }
        node.words[w].style.setProperty("--p", progress * 100 + "%");
      }
    }
  }

  function frame() {
    state.raf = 0;
    if (!state.visible || !state.settings || !state.settings.karaoke || !state.playing) return;
    updateProgress(nowMs());
    state.raf = requestAnimationFrame(frame);
  }

  function ensureRaf() {
    if (!state.raf) state.raf = requestAnimationFrame(frame);
  }

  function stopRaf() {
    if (state.raf) {
      cancelAnimationFrame(state.raf);
      state.raf = 0;
    }
  }

  function setLines(payload) {
    if (!payload) return;
    state.lines = Array.isArray(payload.lines) ? payload.lines : [];
    if (payload.settings) applySettings(payload.settings);
    resetWindow();
    refreshActive(false);
  }

  function setSettings(settings) {
    applySettings(settings);
    resetWindow();
    refreshActive(false);
  }

  function tick(payload) {
    if (!payload) return;
    state.positionMs = payload.positionMs || 0;
    state.lastTickAt = performance.now();
    state.playing = !!payload.playing;
    state.activeIndex = Number.isFinite(payload.activeIndex) ? payload.activeIndex : -1;
    state.activeIndices = Array.isArray(payload.activeIndices) ? payload.activeIndices : [];

    var visible = payload.visible !== false;
    if (visible !== state.visible) {
      state.visible = visible;
      setVisibility(visible);
    }

    var sig = state.activeIndex + "|" + state.activeIndices.join(",");
    if (sig !== state.activeSig) {
      state.activeSig = sig;
      refreshActive(true);
    } else {
      ensureWindow();
    }

    if (state.playing && state.settings && state.settings.karaoke && state.visible) ensureRaf();
    else {
      stopRaf();
      updateProgress(state.positionMs);
    }
  }

  // Desktop: state arrives as Tauri events.
  if (tauri && tauri.event) {
    tauri.event.listen("desktop-lyric:load", function (event) { setLines(event.payload); });
    tauri.event.listen("desktop-lyric:settings", function (event) { setSettings(event.payload); });
    tauri.event.listen("desktop-lyric:tick", function (event) { tick(event.payload); });
  }

  // Android: Kotlin (`evaluateJavascript`) calls these directly.
  window.__welkinOverlay = { load: setLines, settings: setSettings, tick: tick };

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
    resetWindow();
    refreshActive(false);
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
