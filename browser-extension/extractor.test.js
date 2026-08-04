import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const context = { URL, Set, Object };
context.globalThis = context;
runInNewContext(
  readFileSync(new URL("./extractor.js", import.meta.url), "utf8"),
  context,
);
const { collectJmTabs, parseJmUrl } = context.JmTabExtractor;

test("parses album and photo URL variants", () => {
  assert.deepEqual({ ...parseJmUrl("https://example.test/album/001234/") }, {
    type: "album",
    id: "1234",
    key: "album:1234",
    url: "https://example.test/album/001234/",
  });
  assert.equal(parseJmUrl("https://example.test/photo/?id=5678").id, "5678");
});

test("rejects unrelated tabs", () => {
  assert.equal(parseJmUrl("https://example.test/search?id=1234"), null);
  assert.equal(parseJmUrl("chrome://extensions/"), null);
});

test("keeps tab order and removes duplicate work pages", () => {
  const result = collectJmTabs([
    { id: 3, index: 2, url: "https://site.test/album/22", title: "B" },
    { id: 1, index: 0, url: "https://site.test/album/11", title: "A" },
    { id: 2, index: 1, url: "https://mirror.test/album/11", title: "A mirror" },
    { id: 4, index: 3, url: "https://site.test/photo/11", title: "chapter" },
  ]);

  assert.deepEqual(
    [...result].map((item) => item.key),
    ["album:11", "album:22", "photo:11"],
  );
});

test("supports query parameter URL variants", () => {
  assert.equal(parseJmUrl("https://example.test/album?id=0088").id, "88");
  assert.equal(parseJmUrl("https://example.test/album?album_id=99").id, "99");
  assert.equal(parseJmUrl("https://example.test/photo?photo_id=100").id, "100");
});

test("supports the current 18comic detail page format", () => {
  assert.deepEqual(
    { ...parseJmUrl("https://devapp.18comic.cc/comic/detail?id=1455254") },
    {
      type: "album",
      id: "1455254",
      key: "album:1455254",
      url: "https://devapp.18comic.cc/comic/detail?id=1455254",
    },
  );
  assert.equal(
    parseJmUrl("https://devapp.18comic.cc/comic/detail/001455254").id,
    "1455254",
  );
});
