/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Render the Markdown subset used by the release notes (see
 * `scripts/prepare-release.mjs`) to HTML for the update dialog.
 *
 * The notes come from our own `latest.json`, but they still end up in `v-html`,
 * so the whole document is HTML-escaped before any formatting is applied. Only
 * headings, unordered/ordered lists, bold and inline code are recognised; the
 * output is a flat list of block elements (no nesting) which keeps the renderer
 * small and predictable.
 */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Apply the supported inline spans to an already-escaped line. */
function inline(text: string): string {
  return text.replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
}

export function renderReleaseNotes(markdown: string): string {
  const lines = escapeHtml(markdown).split(/\r?\n/);
  const html: string[] = [];
  let list: "ul" | "ol" | null = null;

  const closeList = () => {
    if (list) {
      html.push(`</${list}>`);
      list = null;
    }
  };

  for (const raw of lines) {
    const line = raw.trim();
    if (!line) {
      closeList();
      continue;
    }

    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    if (heading) {
      closeList();
      const level = Math.min(heading[1].length + 2, 6);
      html.push(`<h${level}>${inline(heading[2].trim())}</h${level}>`);
      continue;
    }

    const unordered = /^[-*]\s+(.+)$/.exec(line);
    const ordered = /^\d+[.)]\s+(.+)$/.exec(line);
    const item = unordered?.[1] ?? ordered?.[1];
    if (item !== undefined) {
      const tag = unordered ? "ul" : "ol";
      if (list !== tag) {
        closeList();
        html.push(`<${tag}>`);
        list = tag;
      }
      html.push(`<li>${inline(item.trim())}</li>`);
      continue;
    }

    closeList();
    html.push(`<p>${inline(line)}</p>`);
  }

  closeList();
  return html.join("");
}
