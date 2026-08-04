import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

class FakeElement {
  constructor() {
    this.textContent = "";
    this.value = "";
    this.disabled = false;
    this.handlers = new Map();
    this.classList = { toggle() {} };
  }

  addEventListener(type, handler) {
    this.handlers.set(type, handler);
  }

  click() {
    this.handlers.get("click")?.();
  }

  focus() {}
  select() {}
}

function nextTurn() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

test("popup scans Edge tabs and copies one URL per line", async () => {
  const selectors = new Map(
    ["#summary", "#counts", "#preview", "#status", "#copy-links", "#copy-ids", "#refresh"]
      .map((selector) => [selector, new FakeElement()]),
  );
  const clipboardWrites = [];
  const context = {
    URL,
    Set,
    Object,
    Array,
    Number,
    String,
    Boolean,
    Error,
    Promise,
    navigator: {
      clipboard: {
        async writeText(value) {
          clipboardWrites.push(value);
        },
      },
    },
    document: {
      querySelector(selector) {
        return selectors.get(selector);
      },
      execCommand() {
        return false;
      },
    },
    window: {
      addEventListener() {},
    },
    chrome: {
      runtime: {},
      tabs: {
        query(_query, callback) {
          callback([
            { id: 1, index: 0, url: "https://devapp.18comic.cc/comic/detail?id=123", title: "A" },
            { id: 2, index: 1, url: "https://jm.test/photo?id=456", title: "B" },
            { id: 3, index: 2, url: "https://example.test/", title: "Other" },
          ]);
        },
      },
    },
  };
  context.globalThis = context;

  runInNewContext(readFileSync(new URL("./extractor.js", import.meta.url), "utf8"), context);
  runInNewContext(readFileSync(new URL("./popup.js", import.meta.url), "utf8"), context);
  await nextTurn();

  assert.equal(selectors.get("#summary").textContent, "找到 2 个 JM 页面");
  assert.equal(selectors.get("#counts").textContent, "本子 1 · 章节 1");
  assert.equal(
    selectors.get("#preview").value,
    "https://devapp.18comic.cc/comic/detail?id=123\nhttps://jm.test/photo?id=456",
  );
  assert.equal(selectors.get("#copy-links").disabled, false);

  selectors.get("#copy-links").click();
  await nextTurn();
  assert.deepEqual(clipboardWrites, [
    "https://devapp.18comic.cc/comic/detail?id=123\nhttps://jm.test/photo?id=456",
  ]);
  assert.equal(selectors.get("#status").textContent, "已复制 2 个链接。");
});
