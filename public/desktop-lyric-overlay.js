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
    androidVisibleSignaled: false,
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
    root.setProperty("--stroke-color", settings.stroke ? settings.strokeColor || "#000000" : "transparent");
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
      fragment.appendChild(element);
      elements.push(element);
    }
    return { fragment: fragment, elements: elements, data: words };
  }

  // The second voice of a duet hugs the right edge; everything else is left.
  function isCounterLine(line) {
    return !!(line.isDuet && !line.isBG);
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
      if (state.settings && state.settings.translation && entry.line.translation) wrapper.appendChild(makeSubline(entry.line.translation, "translation bg-translation", false));
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
        oldPrimary.classList.add("is-leaving");
        (function (el) { window.setTimeout(function () { if (el.parentNode) el.parentNode.removeChild(el); }, 580); })(oldPrimary);
      } else if (oldPrimary && oldPrimary.parentNode) {
        oldPrimary.parentNode.removeChild(oldPrimary);
      }
      newPrimary.el.classList.toggle("is-entering", !!animate);
      if (animate) (function (el) { window.setTimeout(function () { el.classList.remove("is-entering"); }, 660); })(newPrimary.el);
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
    var padY = state.settings && state.settings.paddingY != null ? state.settings.paddingY : 18;
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

  if (window.AndroidDesktopLyric) document.documentElement.classList.add("android");

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
    // Persist the user's baseline height (without the temporary extra line), so
    // the auto-grow never leaks into the remembered geometry.
    var saveGeometry = function () {
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
    if (typeof win.onMoved === "function") win.onMoved(saveGeometry);
    if (typeof win.onResized === "function") win.onResized(function () {
      requestAnimationFrame(function () {
        var logical = window.innerHeight;
        state.baseHeight = state.extraHeight ? logical - state.extraHeight : logical;
        applyStageOffset();
        saveGeometry();
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
    var direction = resizeDirectionAt(event);
    stage.style.cursor = direction ? RESIZE_CURSORS[direction] : "";
  });
  stage.addEventListener("pointerleave", function () { stage.style.cursor = ""; });

  document.getElementById("drag").addEventListener("pointerdown", startDragging);
  document.getElementById("resize").addEventListener("pointerdown", function (event) {
    if (!win || typeof win.startResizeDragging !== "function" || event.button !== 0) return;
    event.preventDefault();
    userResizing = true;
    markResizing();
    win.startResizeDragging("SouthEast").catch(function () {
      body.classList.remove("resizing");
      userResizing = false;
    });
  });
  stage.addEventListener("pointerdown", function (event) {
    if (!state.settings || state.settings.locked || event.target.closest("button")) return;
    if (event.button === 0) {
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
    }
    startDragging(event);
  });
  document.getElementById("close").addEventListener("click", function () { if (win && typeof win.close === "function") win.close().catch(function () {}); });
  document.getElementById("lock").addEventListener("click", function () { if (state.settings) { state.settings.locked = !state.settings.locked; applyLocked(state.settings.locked); } });

})();
