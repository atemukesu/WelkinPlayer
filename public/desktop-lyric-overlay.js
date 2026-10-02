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
    currentWordSets: [],
    currentBaseElement: null,
    currentExtraElement: null,
    currentPrimaryIndex: null,
    primaryWordSet: null,
    extraItems: {},
    baseHeight: null,
    extraHeight: 0,
    lastBlockCount: 0,
    androidVisibleSignaled: false,
    raf: 0,
    // Synchronously-readable window geometry, so a touch drag never has to wait
    // on an IPC round-trip before it can start moving.
    winPos: { x: 0, y: 0, width: 0, height: 0, scale: 1, Position: null, Size: null, valid: false },
  };

  var stage = document.getElementById("stage");
  var current = document.getElementById("current");
  var body = document.body;

  // Hybrid devices fire both mouse and touch. Track which one is in use so the
  // CSS hover surface only applies to a real mouse, never to a sticky touch
  // `:hover`. The touch surface is driven separately by `revealTouchControls`.
  function trackPointerType(event) {
    if (event.pointerType === "mouse") {
      body.classList.add("mouse");
      body.classList.remove("touching");
    } else {
      body.classList.add("touching");
      body.classList.remove("mouse");
    }
  }
  // Detect the input type on move as well as down: the hover surface keys off
  // `body.mouse`, so without this the very first hover (before any click) found
  // no `.mouse` class and painted no background.
  document.addEventListener("pointerdown", trackPointerType, true);
  document.addEventListener("pointermove", trackPointerType, true);

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
    root.setProperty("--opacity", Math.max(0, Math.min(1, settings.opacity / 100)));
    root.setProperty("--font-size", settings.fontSize + "px");
    root.setProperty("--translation-size", settings.translationSize + "px");
    root.setProperty("--font-weight", String(settings.fontWeight));
    root.setProperty("--text-color", settings.textColor);
    root.setProperty("--active-color", settings.activeColor);
    root.setProperty("--translation-color", settings.translationColor);
    root.setProperty("--stroke-width", settings.stroke ? "1px" : "0px");
    root.setProperty("--stroke-scale", settings.stroke ? "1" : "0");
    root.setProperty("--stroke-color", settings.stroke ? settings.strokeColor || "#000000" : "transparent");
    body.style.fontFamily = cssFontFamily(settings.fontFamilies) || "";
    body.classList.toggle("karaoke", !!settings.karaoke);
    body.classList.toggle("has-stroke", !!settings.stroke);
    applyLocked(!!settings.locked);
    if (win) {
      // Always-on-top is a fixed property of the floating layer now.
      if (typeof win.setAlwaysOnTop === "function") win.setAlwaysOnTop(true).catch(function () {});
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
    // Locked = click-through: the window swallows no input, so the layer behind
    // it stays usable. An in-window unlock affordance is impossible in that
    // state; unlock from the settings toggle (desktop) / notification (Android).
    if (win && typeof win.setIgnoreCursorEvents === "function") win.setIgnoreCursorEvents(locked).catch(function () {});
    if (window.AndroidDesktopLyric && window.AndroidDesktopLyric.setLocked) {
      try { window.AndroidDesktopLyric.setLocked(locked); } catch (error) {}
    }
  }

  function updateTrack(payload) {
    var element = document.getElementById("track");
    if (!element) return;
    var title = payload && payload.title ? payload.title : "";
    var artist = payload && payload.artist ? payload.artist : "";
    element.textContent = title && artist ? title + " - " + artist : title || artist;
  }

  function updatePlayButton() {
    var button = document.getElementById("play");
    if (!button) return;
    var playing = !!state.playing;
    button.title = playing ? "暂停" : "播放";
    button.setAttribute("aria-label", button.title);
    button.innerHTML = playing
      ? '<svg viewBox="0 0 24 24"><path d="M8 5h3v14H8z"/><path d="M13 5h3v14h-3z"/></svg>'
      : '<svg viewBox="0 0 24 24"><path d="M7 5v14l12-7z"/></svg>';
  }

  function sendControl(action) {
    if (isAndroid && window.AndroidDesktopLyric && typeof window.AndroidDesktopLyric.control === "function") {
      try { window.AndroidDesktopLyric.control(action); } catch (error) {}
      return;
    }
    if (tauri && tauri.event && typeof tauri.event.emit === "function") {
      try { tauri.event.emit("desktop-lyric:control", action); } catch (error) {}
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

  // The Android overlay WebView starts hidden, and because `visible` defaults to
  // true the first tick never triggers a change. Signal it visible exactly once,
  // after the first frame of content is actually on screen.
  function signalAndroidVisible() {
    if (state.androidVisibleSignaled) return;
    if (!window.AndroidDesktopLyric || typeof window.AndroidDesktopLyric.setVisible !== "function") return;
    if (!state.visible) return;
    state.androidVisibleSignaled = true;
    requestAnimationFrame(function () {
      try { window.AndroidDesktopLyric.setVisible(true); } catch (error) {}
    });
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

  function makeWords(line) {
    var words = Array.isArray(line.words) && line.words.length ? line.words : [{ text: line.text || "", start: 0, end: 0 }];
    var fragment = document.createDocumentFragment();
    var elements = [];
    for (var i = 0; i < words.length; i++) {
      var element = document.createElement("span");
      element.className = "word";
      element.textContent = words[i].text;
      element.setAttribute("data-text", words[i].text == null ? "" : words[i].text);
      fragment.appendChild(element);
      elements.push(element);
    }
    return { fragment: fragment, elements: elements, data: words };
  }

  // The second voice of a duet hugs the right edge. A background line follows
  // the voice it belongs to, so the duet's backing vocals also sit on the right
  // instead of being forced to the left like every other background line.
  function isCounterLine(line) {
    return !!line.isDuet;
  }

  // Display order: left voice on top, right voice below, background last.
  function lineRank(line) {
    if (line.isBG) return 2;
    return line.isDuet ? 1 : 0;
  }

  // `kind` is "primary" for the lead / duet lines and "bg" for background
  // vocals. Both participate in the karaoke sweep; only the size differs.
  function makeLine(line, kind) {
    var paragraph = document.createElement("div");
    paragraph.className = "lyric-line " + kind + (isCounterLine(line) ? " duet" : "");
    var words = makeWords(line);
    paragraph.appendChild(words.fragment);
    return { el: paragraph, words: words.elements, wordData: words.data };
  }

  function makeSubline(text, className, counter) {
    var paragraph = document.createElement("p");
    paragraph.className = className + (counter ? " duet" : "");
    paragraph.textContent = text;
    return paragraph;
  }

  function buildPrimaryGroup(entry) {
    var group = document.createElement("div");
    group.className = "primary-group";
    var line = makeLine(entry.line, entry.line.isBG ? "bg" : "primary");
    group.appendChild(line.el);
    var counter = isCounterLine(entry.line);
    if (state.settings && state.settings.translation && entry.line.translation) group.appendChild(makeSubline(entry.line.translation, "translation", counter));
    if (entry.line.roman) group.appendChild(makeSubline(entry.line.roman, "roman", counter));
    return { el: group, wordSets: [{ elements: line.words, data: line.wordData }] };
  }

  function buildExtraItem(entry) {
    var wrapper = document.createElement("div");
    wrapper.className = "extra-item";
    wrapper.setAttribute("data-rank", String(lineRank(entry.line)));
    var wordSets = [];
    if (entry.line.isBG) {
      var bg = makeLine(entry.line, "bg");
      wrapper.appendChild(bg.el);
      wordSets.push({ elements: bg.words, data: bg.wordData });
      var bgCounter = isCounterLine(entry.line);
      if (state.settings && state.settings.translation && entry.line.translation) wrapper.appendChild(makeSubline(entry.line.translation, "translation bg-translation", bgCounter));
      if (entry.line.roman) wrapper.appendChild(makeSubline(entry.line.roman, "roman", bgCounter));
    } else {
      var duet = makeLine(entry.line, "primary");
      wrapper.appendChild(duet.el);
      wordSets.push({ elements: duet.words, data: duet.wordData });
      var counter = isCounterLine(entry.line);
      if (state.settings && state.settings.translation && entry.line.translation) wrapper.appendChild(makeSubline(entry.line.translation, "translation", counter));
      if (entry.line.roman) wrapper.appendChild(makeSubline(entry.line.roman, "roman", counter));
    }
    return { el: wrapper, wordSets: wordSets };
  }

  function resetBlock() {
    if (state.currentNode && state.currentNode.parentNode) state.currentNode.parentNode.removeChild(state.currentNode);
    state.currentNode = null;
    state.currentBaseElement = null;
    state.currentExtraElement = null;
    state.currentPrimaryIndex = null;
    state.primaryWordSet = null;
    state.extraItems = {};
    state.currentWordSets = [];
  }

  function renderCurrent(animate) {
    var block = activeBlock();
    block.sort(function (a, b) { return lineRank(a.line) - lineRank(b.line); });
    var primaryEntry = block[0] || null;

    // The blur transition is reserved for a whole-screen swap: exactly one line
    // is active before and after. Whenever several lines are on screen together
    // (or the block size is changing), use the smooth opacity transition instead.
    var multi = block.length > 1 || state.lastBlockCount > 1;
    var enterClass = multi ? "is-entering-soft" : "is-entering";
    var leaveClass = multi ? "is-leaving-soft" : "is-leaving";

    if (!primaryEntry) {
      var leaving = state.currentNode;
      if (leaving && animate) {
        leaving.classList.add("is-leaving");
        window.setTimeout(function () { if (leaving.parentNode === current) leaving.parentNode.removeChild(leaving); }, 580);
      } else if (leaving && leaving.parentNode === current) {
        current.removeChild(leaving);
      }
      state.currentNode = null;
      state.currentBaseElement = null;
      state.currentExtraElement = null;
      state.currentPrimaryIndex = null;
      state.primaryWordSet = null;
      state.extraItems = {};
      state.currentWordSets = [];
      state.lastBlockCount = 0;
      return;
    }

    if (!state.currentNode) {
      var blockElement = document.createElement("div");
      blockElement.className = "lyric-block";
      var extraGroup = document.createElement("div");
      extraGroup.className = "extra-group";
      blockElement.appendChild(extraGroup);
      current.appendChild(blockElement);
      state.currentNode = blockElement;
      state.currentExtraElement = extraGroup;
    }

    // The lead line is only rebuilt (and blurred) when the lead line itself
    // changes; extra lines fade in / out without disturbing it.
    if (state.currentPrimaryIndex !== primaryEntry.index || !state.currentBaseElement) {
      var newPrimary = buildPrimaryGroup(primaryEntry);
      var oldPrimary = state.currentBaseElement;
      if (oldPrimary && animate) {
        oldPrimary.classList.add(leaveClass);
        (function (el) { window.setTimeout(function () { if (el.parentNode) el.parentNode.removeChild(el); }, 580); })(oldPrimary);
      } else if (oldPrimary && oldPrimary.parentNode) {
        oldPrimary.parentNode.removeChild(oldPrimary);
      }
      newPrimary.el.classList.toggle(enterClass, !!animate);
      if (animate) (function (el) { window.setTimeout(function () { el.classList.remove(enterClass); }, 660); })(newPrimary.el);
      state.currentNode.insertBefore(newPrimary.el, state.currentExtraElement);
      state.currentBaseElement = newPrimary.el;
      state.currentPrimaryIndex = primaryEntry.index;
      state.primaryWordSet = newPrimary.wordSets[0];
    }

    var desired = {};
    for (var j = 0; j < block.length; j++) {
      if (block[j].index !== primaryEntry.index) desired[block[j].index] = block[j];
    }

    var index;
    for (index in state.extraItems) {
      if (!desired[index]) {
        var gone = state.extraItems[index];
        delete state.extraItems[index];
        if (animate) {
          gone.el.classList.add("is-leaving");
          (function (el) {
            window.setTimeout(function () {
              if (el.parentNode) el.parentNode.removeChild(el);
              if (state.currentExtraElement && state.currentExtraElement.children.length === 0) state.currentExtraElement.hidden = true;
              remeasure();
            }, 260);
          })(gone.el);
        } else if (gone.el.parentNode) {
          gone.el.parentNode.removeChild(gone.el);
        }
      }
    }
    for (var m = 0; m < block.length; m++) {
      var entry = block[m];
      if (entry.index === primaryEntry.index || state.extraItems[entry.index]) continue;
      var item = buildExtraItem(entry);
      state.extraItems[entry.index] = item;
      var rank = lineRank(entry.line);
      var children = state.currentExtraElement.children;
      var placed = false;
      for (var c = 0; c < children.length; c++) {
        var sibling = children[c];
        if (sibling.classList.contains("is-leaving")) continue;
        if (Number(sibling.getAttribute("data-rank")) > rank) {
          state.currentExtraElement.insertBefore(item.el, sibling);
          placed = true;
          break;
        }
      }
      if (!placed) state.currentExtraElement.appendChild(item.el);
      if (animate) {
        item.el.classList.add("is-entering");
        (function (el) { window.setTimeout(function () { el.classList.remove("is-entering"); }, 300); })(item.el);
      }
    }
    state.currentExtraElement.hidden = state.currentExtraElement.children.length === 0;

    state.currentWordSets = state.primaryWordSet ? [state.primaryWordSet] : [];
    for (index in state.extraItems) {
      var sets = state.extraItems[index].wordSets;
      for (var k = 0; k < sets.length; k++) state.currentWordSets.push(sets[k]);
    }

    state.lastBlockCount = block.length;
    updateProgress(state.positionMs);
    remeasure();
    signalAndroidVisible();
  }

  function remeasure() {
    if (!state.currentNode) return;
    updateWindowHeight(state.currentNode);
  }

  // The lead line is pinned below `--stage-top`: a single-line block is centred
  // in the baseline window, and the inset stays fixed while the window grows for
  // extra lines, so the text never drifts.
  function applyStageOffset() {
    if (!win) return;
    if (state.baseHeight == null && window.innerHeight) state.baseHeight = window.innerHeight;
    if (state.baseHeight == null) return;
    var base = state.currentBaseElement ? state.currentBaseElement.getBoundingClientRect().height : 0;
    if (base <= 0) return;
    var padY = 18;
    var offset = Math.max(padY, Math.round((state.baseHeight - base) / 2));
    document.documentElement.style.setProperty("--stage-top", offset + "px");
  }

  // Size the floating window so extra lines fit below the lead line. The extra
  // height is measured from the extra group alone, so a previous line that is
  // still fading out cannot inflate it (which made the next line resize/jump).
  function updateWindowHeight(blockElement) {
    var extra = 0;
    if (state.currentExtraElement && !state.currentExtraElement.hidden) {
      var rowGap = parseFloat(getComputedStyle(blockElement).rowGap) || 0;
      extra = Math.round(state.currentExtraElement.getBoundingClientRect().height + rowGap);
    }
    if (state.baseHeight == null && window.innerHeight) state.baseHeight = window.innerHeight;
    applyStageOffset();
    state.extraHeight = extra;
    if (!win || typeof win.setSize !== "function" || state.baseHeight == null) return;
    var desired = state.baseHeight + extra;
    if (Math.abs(desired - window.innerHeight) <= 1) return;
    var width = window.innerWidth;
    var LogicalSize = tauri && tauri.window && tauri.window.LogicalSize;
    try {
      var size = LogicalSize ? new LogicalSize(width, desired) : { width: width, height: desired };
      win.setSize(size).catch(function () {});
    } catch (error) {}
  }

  function nowMs() {
    return state.playing ? state.positionMs + (performance.now() - state.lastTickAt) : state.positionMs;
  }

  function updateProgress(position) {
    if (!state.currentNode || !state.settings) return;
    var sets = state.currentWordSets;
    for (var s = 0; s < sets.length; s++) {
      var elements = sets[s].elements;
      var data = sets[s].data;
      for (var i = 0; i < elements.length; i++) {
        var word = data[i];
        var progress = 0;
        if (word) progress = Math.max(0, Math.min(1, (position - word.start) / Math.max(1, word.end - word.start)));
        elements[i].style.setProperty("--p", progress * 100 + "%");
        elements[i].classList.toggle("is-unplayed", progress <= 0);
      }
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
    updateTrack(payload);
    if (payload.settings) applySettings(payload.settings);
    resetBlock();
    renderCurrent(false);
  }

  function setSettings(settings) {
    applySettings(settings);
    // Style changes can alter the block structure (translation toggle, ...), so
    // rebuild it silently; the karaoke playhead is re-applied right after.
    resetBlock();
    renderCurrent(false);
  }

  function tick(payload) {
    if (!payload) return;
    state.positionMs = payload.positionMs || 0;
    state.lastTickAt = performance.now();
    var playing = !!payload.playing;
    if (playing !== state.playing) {
      state.playing = playing;
      updatePlayButton();
    }
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
    Promise.all([
      tauri.event.listen("desktop-lyric:load", function (event) { setLines(event.payload); }),
      tauri.event.listen("desktop-lyric:settings", function (event) { setSettings(event.payload); }),
      tauri.event.listen("desktop-lyric:tick", function (event) { tick(event.payload); }),
    ]).then(function () {
      // Tell the main window we can receive documents now; it answers with a
      // fresh snapshot. Lets a re-opened layer paint the current lyric at once.
      if (typeof tauri.event.emit === "function") {
        try { tauri.event.emit("desktop-lyric:ready"); } catch (error) {}
      }
    });
  }
  window.__welkinOverlay = { load: setLines, settings: setSettings, tick: tick };

  function readGeometry() {
    try { return JSON.parse(localStorage.getItem("welkin-desktop-lyric-geometry") || "null"); } catch (error) { return null; }
  }

  var isAndroid = !!window.AndroidDesktopLyric;
  if (isAndroid) document.documentElement.classList.add("android");

  if (win) {
    body.classList.add("desktop");
    var geometry = readGeometry();
    if (geometry && Number.isFinite(geometry.x) && Number.isFinite(geometry.y)) {
      if (Number.isFinite(geometry.height)) state.baseHeight = geometry.height;
      if (typeof win.setPosition === "function") {
        try { var LogicalPosition = tauri.window.LogicalPosition; win.setPosition(LogicalPosition ? new LogicalPosition(geometry.x, geometry.y) : { x: geometry.x, y: geometry.y }).catch(function () {}); } catch (error) {}
      }
      if (geometry.width && geometry.height && typeof win.setSize === "function") {
        try { var LogicalSize = tauri.window.LogicalSize; win.setSize(LogicalSize ? new LogicalSize(geometry.width, geometry.height) : { width: geometry.width, height: geometry.height }).catch(function () {}); } catch (error) {}
      }
    }
    // Cache the physical window position and scale so a touch drag can read it
    // synchronously. `outerPosition` / `scaleFactor` are async IPC; waiting on
    // them during `pointerdown` drops the first frames of every gesture.
    var refreshWinPos = function () {
      if ((drag && drag.active) || (touchResize && touchResize.active)) return;
      Promise.all([
        typeof win.outerPosition === "function" ? win.outerPosition() : Promise.resolve(null),
        typeof win.outerSize === "function" ? win.outerSize() : Promise.resolve(null),
        typeof win.scaleFactor === "function" ? win.scaleFactor() : Promise.resolve(1),
      ]).then(function (values) {
        var pos = values[0], size = values[1];
        state.winPos.scale = values[2] || 1;
        if (pos) {
          state.winPos.x = pos.x;
          state.winPos.y = pos.y;
          state.winPos.Position = pos.constructor || state.winPos.Position;
          state.winPos.valid = true;
        }
        if (size) {
          state.winPos.width = size.width;
          state.winPos.height = size.height;
          state.winPos.Size = size.constructor || state.winPos.Size;
        }
      }).catch(function () {});
    };

    // Persist the baseline height (without the temporary extra line), so the
    // auto-grow never leaks into the remembered geometry. Debounced: during a
    // drag `onMoved` fires per frame and a synchronous `localStorage` write each
    // time stalls the main thread and visibly worsens the drag.
    var doSaveGeometry = function () {
      Promise.all([
        typeof win.outerPosition === "function" ? win.outerPosition() : null,
        typeof win.outerSize === "function" ? win.outerSize() : null,
        typeof win.scaleFactor === "function" ? win.scaleFactor() : 1,
      ]).then(function (values) {
        var pos = values[0], size = values[1], scale = values[2] || 1;
        if (!pos || !size) return;
        var height = state.baseHeight != null ? state.baseHeight : size.height / scale;
        try { localStorage.setItem("welkin-desktop-lyric-geometry", JSON.stringify({ x: pos.x / scale, y: pos.y / scale, width: size.width / scale, height: height })); } catch (error) {}
      });
    };
    var saveTimer = 0;
    var saveGeometry = function () {
      window.clearTimeout(saveTimer);
      saveTimer = window.setTimeout(doSaveGeometry, 300);
    };
    refreshWinPos();
    if (typeof win.onMoved === "function") win.onMoved(function () { refreshWinPos(); saveGeometry(); });
    if (typeof win.onResized === "function") win.onResized(function () {
      requestAnimationFrame(function () {
        var logical = window.innerHeight;
        state.baseHeight = state.extraHeight ? logical - state.extraHeight : logical;
        applyStageOffset();
        saveGeometry();
        refreshWinPos();
      });
      if (userResizing) markResizing();
    });
  }

  var userResizing = false;
  var resizeClassTimer = 0;

  // Keep the hover background up while resizing: the OS owns the drag and the
  // webview stops receiving hover, so `:hover` alone would blink the panel off.
  function markResizing() {
    body.classList.add("resizing");
    window.clearTimeout(resizeClassTimer);
    resizeClassTimer = window.setTimeout(function () {
      body.classList.remove("resizing");
      userResizing = false;
    }, 400);
  }

  function startDragging(event) {
    if (!win || typeof win.startDragging !== "function" || (event && event.button !== 0)) return;
    win.startDragging().catch(function () {});
  }

  var RESIZE_EDGE = 8;
  var RESIZE_DIRECTIONS = {
    n: "North", s: "South", e: "East", w: "West",
    ne: "NorthEast", nw: "NorthWest", se: "SouthEast", sw: "SouthWest",
  };
  var RESIZE_CURSORS = {
    North: "ns-resize", South: "ns-resize",
    East: "ew-resize", West: "ew-resize",
    NorthWest: "nwse-resize", SouthEast: "nwse-resize",
    NorthEast: "nesw-resize", SouthWest: "nesw-resize",
  };

  function resizeDirectionAt(event) {
    if (!win || (state.settings && state.settings.locked)) return null;
    var width = window.innerWidth;
    var height = window.innerHeight;
    var x = event.clientX;
    var y = event.clientY;
    var vertical = y <= RESIZE_EDGE ? "n" : y >= height - RESIZE_EDGE ? "s" : "";
    var horizontal = x <= RESIZE_EDGE ? "w" : x >= width - RESIZE_EDGE ? "e" : "";
    var key = vertical + horizontal;
    return key ? RESIZE_DIRECTIONS[key] : null;
  }

  stage.addEventListener("pointermove", function (event) {
    if (drag.active) return;
    var direction = resizeDirectionAt(event);
    stage.style.cursor = direction ? RESIZE_CURSORS[direction] : "";
  });
  stage.addEventListener("pointerleave", function () { stage.style.cursor = ""; });

  // Touch resizing: the OS `startResizeDragging` only follows a mouse, so the
  // handle is driven through `setSize`, mirroring the touch-drag model.
  var touchResize = {
    active: false,
    downPointerX: 0, downPointerY: 0,
    startW: 0, startH: 0, scale: 1, Size: null,
  };

  function beginTouchResize(event) {
    if (lockedNow() || !win) return;
    if (!state.winPos.valid || !state.winPos.width || !state.winPos.height) { refreshWinPos(); event.preventDefault(); return; }
    touchResize.active = true;
    touchResize.startW = state.winPos.width;
    touchResize.startH = state.winPos.height;
    touchResize.scale = state.winPos.scale;
    touchResize.Size = state.winPos.Size;
    touchResize.downPointerX = window.screenX + event.clientX;
    touchResize.downPointerY = window.screenY + event.clientY;
    userResizing = true;
    // Keep the handle and surface up for the whole gesture; the shared 1.5s
    // timer resumes only after release.
    window.clearTimeout(controlsTimer);
    body.classList.add("controls-visible", "touching", "resizing");
    try { resizeHandle.setPointerCapture(event.pointerId); } catch (error) {}
    event.preventDefault();
  }

  function moveTouchResize(event) {
    if (!touchResize.active) return;
    var pointerX = window.screenX + event.clientX;
    var pointerY = window.screenY + event.clientY;
    var minWidth = Math.round(240 * touchResize.scale);
    var minHeight = Math.round(72 * touchResize.scale);
    var width = Math.max(minWidth, Math.round(touchResize.startW + (pointerX - touchResize.downPointerX) * touchResize.scale));
    var height = Math.max(minHeight, Math.round(touchResize.startH + (pointerY - touchResize.downPointerY) * touchResize.scale));
    scheduleSize(width, height);
  }

  function endTouchResize(event) {
    if (!touchResize.active) return;
    touchResize.active = false;
    userResizing = false;
    body.classList.remove("resizing");
    try { resizeHandle.releasePointerCapture(event.pointerId); } catch (error) {}
    refreshWinPos();
    saveGeometry();
    // Restart the shared auto-hide so the handle stays reachable for 1.5s.
    revealTouchControls();
  }

  var resizeHandle = document.getElementById("resize");
  resizeHandle.addEventListener("pointerdown", function (event) {
    if (event.pointerType !== "mouse") {
      beginTouchResize(event);
      return;
    }
    if (!win || typeof win.startResizeDragging !== "function" || event.button !== 0) return;
    event.preventDefault();
    userResizing = true;
    markResizing();
    win.startResizeDragging("SouthEast").catch(function () {
      body.classList.remove("resizing");
      userResizing = false;
    });
  });
  resizeHandle.addEventListener("pointermove", moveTouchResize);
  resizeHandle.addEventListener("pointerup", endTouchResize);
  resizeHandle.addEventListener("pointercancel", endTouchResize);
  // Desktop mouse: use the OS-native window drag / edge resize, which only
  // follows a real mouse pointer.
  stage.addEventListener("pointerdown", function (event) {
    if (event.pointerType !== "mouse") return;
    if (!win || !state.settings || state.settings.locked || event.target.closest("button")) return;
    if (event.button !== 0) return;
    var direction = resizeDirectionAt(event);
    if (direction && typeof win.startResizeDragging === "function") {
      event.preventDefault();
      userResizing = true;
      markResizing();
      win.startResizeDragging(direction).catch(function () {
        body.classList.remove("resizing");
        userResizing = false;
      });
      return;
    }
    startDragging(event);
  });

  // Touch / pen handling is shared by every platform: phones, tablets and
  // touch-screen desktops. A tap reveals the controls for a while; a drag moves
  // the layer. The Android overlay is moved natively from raw screen coordinates
  // (the WebView's own `clientX/Y` feed back into themselves once the window
  // follows the finger), while a desktop window is moved through Tauri
  // (`startDragging` only follows a mouse, and the browser cannot move the
  // transparent window by itself).
  var controlsTimer = 0;
  var drag = {
    active: false, moved: false,
    // Android: the native container owns the movement, so only the initial
    // pointer is kept to tell a drag from a tap.
    downX: 0, downY: 0,
    // Desktop: `window.screenX + clientX` is the only truly stable screen
    // coordinate. Webview pointer events report a window-relative `screenX` in
    // practice, which oscillates as the window follows the finger.
    downPointerX: 0, downPointerY: 0, lastPointerX: 0, lastPointerY: 0,
    curX: 0, curY: 0, scale: 1, Position: null,
    downAt: 0,
  };
  var pendingPos = null;
  var posRaf = 0;

  // Coalesce `setPosition` to one IPC per frame. Touch samples at up to 120 Hz;
  // issuing one async IPC per sample queues them faster than they drain, so the
  // window falls behind the finger.
  function schedulePosition(x, y) {
    pendingPos = { x: x, y: y };
    if (posRaf) return;
    posRaf = requestAnimationFrame(function () {
      posRaf = 0;
      var next = pendingPos;
      pendingPos = null;
      if (!next || !win) return;
      var position = drag.Position ? new drag.Position(next.x, next.y) : { x: next.x, y: next.y };
      try { win.setPosition(position).catch(function () {}); } catch (error) {}
    });
  }

  // Same coalescing for touch resizing: one `setSize` per frame.
  var pendingSize = null;
  var sizeRaf = 0;
  function scheduleSize(width, height) {
    pendingSize = { width: width, height: height };
    if (sizeRaf) return;
    sizeRaf = requestAnimationFrame(function () {
      sizeRaf = 0;
      var next = pendingSize;
      pendingSize = null;
      if (!next || !win) return;
      var size = touchResize.Size ? new touchResize.Size(next.width, next.height) : { width: next.width, height: next.height };
      try { win.setSize(size).catch(function () {}); } catch (error) {}
    });
  }

  function lockedNow() { return !!(state.settings && state.settings.locked); }

  /** Mouse hover: show while the pointer stays inside the interactive area. */
  function showControls() {
    if (lockedNow()) return;
    body.classList.add("controls-visible");
  }

  /**
   * Touch reveal: keep the buttons *and* the background surface around for a
   * moment after the finger lifts, so the controls are actually reachable.
   */
  function revealTouchControls() {
    if (lockedNow()) return;
    body.classList.add("controls-visible", "touching");
    window.clearTimeout(controlsTimer);
    controlsTimer = window.setTimeout(hideControls, 1500);
  }

  function hideControls() {
    window.clearTimeout(controlsTimer);
    body.classList.remove("controls-visible");
    body.classList.remove("touching");
  }

  function blurActive() {
    var active = document.activeElement;
    if (active && typeof active.blur === "function") active.blur();
    window.setTimeout(function () {
      var focused = document.activeElement;
      if (focused && typeof focused.blur === "function") focused.blur();
    }, 0);
  }

  function controlClick(action) {
    sendControl(action);
    // Keep the layer reachable after a press: touch users get the timeout,
    // mouse users keep it while the pointer is still over the controls.
    if (body.classList.contains("mouse")) showControls();
    else revealTouchControls();
    if (lockedNow()) hideControls();
    blurActive();
  }

  function beginTouchDrag(event) {
    drag.active = true;
    drag.moved = false;
    drag.downX = event.clientX;
    drag.downY = event.clientY;
    drag.downPointerX = window.screenX + event.clientX;
    drag.downPointerY = window.screenY + event.clientY;
    drag.lastPointerX = drag.downPointerX;
    drag.lastPointerY = drag.downPointerY;
    drag.downAt = performance.now();
    // Cancel any pending auto-hide so it cannot fire mid-gesture.
    window.clearTimeout(controlsTimer);
    if (!lockedNow()) {
      if (win && !state.winPos.valid) {
        // Geometry cache not ready yet (cold start): abandon this gesture rather
        // than teleporting the window from an assumed origin.
        drag.active = false;
        body.classList.remove("dragging");
        refreshWinPos();
        event.preventDefault();
        return;
      }
      body.classList.add("dragging");
      try { stage.setPointerCapture(event.pointerId); } catch (error) {}
      if (win) {
        // Synchronous read of the cached geometry: no IPC on the drag path.
        drag.curX = state.winPos.x;
        drag.curY = state.winPos.y;
        drag.scale = state.winPos.scale;
        drag.Position = state.winPos.Position;
        refreshWinPos();
      }
    }
    event.preventDefault();
  }

  function moveTouchDrag(event) {
    if (!drag.active) return;

    if (isAndroid) {
      // The native container moves the window from raw screen coordinates,
      // which do not change as the window follows the finger, so there is no
      // delta to apply here and no feedback to cancel. Only record whether the
      // gesture became a drag so the controls can stay up after release.
      if (Math.abs(event.clientX - drag.downX) > 6 || Math.abs(event.clientY - drag.downY) > 6) drag.moved = true;
      return;
    }

    if (!win) return;
    // `window.screenX + clientX` cancels the window's own movement, giving the
    // true finger position on screen regardless of platform `screenX` semantics.
    var pointerX = window.screenX + event.clientX;
    var pointerY = window.screenY + event.clientY;
    if (Math.abs(pointerX - drag.downPointerX) > 6 || Math.abs(pointerY - drag.downPointerY) > 6) drag.moved = true;
    drag.curX += (pointerX - drag.lastPointerX) * drag.scale;
    drag.curY += (pointerY - drag.lastPointerY) * drag.scale;
    drag.lastPointerX = pointerX;
    drag.lastPointerY = pointerY;
    schedulePosition(Math.round(drag.curX), Math.round(drag.curY));
  }

  function endTouchDrag(event) {
    if (!drag.active) return;
    drag.active = false;
    body.classList.remove("dragging");
    try { stage.releasePointerCapture(event.pointerId); } catch (error) {}
    // The cached position moved with the window; resync it for the next gesture.
    if (win) refreshWinPos();
    // Whether it was a tap or a drag, keep the touch surface and controls up for
    // a moment after release, then let them hide on the shared timer.
    revealTouchControls();
  }

  stage.addEventListener("pointerdown", function (event) {
    if (event.pointerType === "mouse") return;
    if (event.target.closest("button")) return;
    beginTouchDrag(event);
  });
  stage.addEventListener("pointermove", moveTouchDrag);
  stage.addEventListener("pointerup", endTouchDrag);
  stage.addEventListener("pointercancel", function (event) {
    if (!drag.active) return;
    drag.active = false;
    body.classList.remove("dragging");
    hideControls();
    try { stage.releasePointerCapture(event.pointerId); } catch (error) {}
  });

  // Desktop mouse hover reveals the controls while the pointer is inside. The
  // listeners ignore touch so a finger never pins the layer open.
  stage.addEventListener("pointerenter", function (event) { if (event.pointerType === "mouse") showControls(); });
  stage.addEventListener("pointerleave", function (event) { if (event.pointerType === "mouse") hideControls(); });
  document.getElementById("tools").addEventListener("pointerenter", function (event) { if (event.pointerType === "mouse") showControls(); });
  document.getElementById("tools").addEventListener("pointerleave", function (event) { if (event.pointerType === "mouse") hideControls(); });

  document.getElementById("close").addEventListener("click", function () {
    // Tell the main window so its toggle reflects the real state...
    sendControl("close");
    // ...and tear the layer down here right away for immediate feedback.
    if (isAndroid && window.AndroidDesktopLyric && typeof window.AndroidDesktopLyric.close === "function") {
      try { window.AndroidDesktopLyric.close(); return; } catch (error) {}
    }
    if (win && typeof win.close === "function") win.close().catch(function () {});
  });
  document.getElementById("lock").addEventListener("click", function () {
    if (!state.settings) return;
    state.settings.locked = !state.settings.locked;
    applyLocked(state.settings.locked);
    controlClick(state.settings.locked ? "lock" : "unlock");
  });
  document.getElementById("prev").addEventListener("click", function () { controlClick("previous"); });
  document.getElementById("play").addEventListener("click", function () { controlClick("toggle"); });
  document.getElementById("next").addEventListener("click", function () { controlClick("next"); });

  // Chromium pauses rAF for hidden documents. The main window is hidden when
  // minimised to tray, but this overlay is its own always-on-top window, so its
  // rAF keeps running; still, resync it on visibility changes so a temporarily
  // occluded/hidden layer resumes the karaoke sweep immediately instead of
  // waiting for the next tick.
  document.addEventListener("visibilitychange", function () {
    if (document.hidden) {
      stopRaf();
    } else if (state.playing && state.settings && state.settings.karaoke && state.visible) {
      ensureRaf();
    }
  });

})();
