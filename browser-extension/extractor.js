(function exposeJmTabExtractor(global) {
  "use strict";

  const PAGE_TYPES = new Set(["album", "photo"]);

  function numericId(value) {
    const match = String(value == null ? "" : value).trim().match(/^\d{1,12}$/);
    return match ? match[0].replace(/^0+(?=\d)/, "") : null;
  }

  function parseJmUrl(rawUrl) {
    let url;
    try {
      url = new URL(rawUrl);
    } catch {
      return null;
    }

    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return null;
    }

    const segments = url.pathname.split("/").filter(Boolean);
    let type = null;
    let id = null;

    const isComicDetail =
      segments.length >= 2 &&
      segments[0].toLowerCase() === "comic" &&
      segments[1].toLowerCase() === "detail";

    if (isComicDetail) {
      type = "album";
      const candidates = [
        segments[2],
        url.searchParams.get("id"),
        url.searchParams.get("comic_id"),
        url.searchParams.get("album_id"),
      ];
      id = candidates.map(numericId).find(Boolean) || null;
    }

    for (let index = 0; !type && index < segments.length; index += 1) {
      const candidateType = segments[index].toLowerCase();
      if (!PAGE_TYPES.has(candidateType)) {
        continue;
      }

      type = candidateType;
      const queryKey = type === "album" ? "album_id" : "photo_id";
      const candidates = [
        segments[index + 1],
        url.searchParams.get("id"),
        url.searchParams.get(queryKey),
      ];
      id = candidates.map(numericId).find(Boolean) || null;
      break;
    }

    if (!type || !id) {
      return null;
    }

    return {
      type: type,
      id: id,
      key: type + ":" + id,
      url: url.href,
    };
  }

  function collectJmTabs(tabs) {
    const seen = new Set();
    const result = [];
    const orderedTabs = Array.isArray(tabs) ? tabs.slice() : [];

    orderedTabs.sort(function compareTabOrder(left, right) {
      return Number(left.index || 0) - Number(right.index || 0);
    });

    for (const tab of orderedTabs) {
      const parsed = parseJmUrl(tab && tab.url);
      if (!parsed || seen.has(parsed.key)) {
        continue;
      }

      seen.add(parsed.key);
      result.push({
        type: parsed.type,
        id: parsed.id,
        key: parsed.key,
        url: parsed.url,
        title: String((tab && tab.title) || "").trim(),
        tabId: tab && tab.id,
      });
    }

    return result;
  }

  global.JmTabExtractor = Object.freeze({
    parseJmUrl: parseJmUrl,
    collectJmTabs: collectJmTabs,
  });
})(globalThis);
