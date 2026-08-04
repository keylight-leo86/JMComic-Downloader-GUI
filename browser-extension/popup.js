(function initializePopup() {
  "use strict";

  const summary = document.querySelector("#summary");
  const counts = document.querySelector("#counts");
  const preview = document.querySelector("#preview");
  const status = document.querySelector("#status");
  const copyLinksButton = document.querySelector("#copy-links");
  const copyIdsButton = document.querySelector("#copy-ids");
  const refreshButton = document.querySelector("#refresh");
  let items = [];

  function errorMessage(error) {
    if (error && typeof error.message === "string") {
      return error.message;
    }
    return String(error || "未知错误");
  }

  function setStatus(message, isError) {
    status.textContent = message;
    status.classList.toggle("error", Boolean(isError));
  }

  function showUnexpectedError(error) {
    summary.textContent = "扩展运行出错";
    setStatus("错误：" + errorMessage(error), true);
  }

  window.addEventListener("error", function handleWindowError(event) {
    showUnexpectedError(event.error || event.message);
  });

  window.addEventListener("unhandledrejection", function handleRejectedPromise(event) {
    showUnexpectedError(event.reason);
  });

  function linksText() {
    return items.map(function getUrl(item) {
      return item.url;
    }).join("\n");
  }

  function idsText() {
    return items.map(function getId(item) {
      return item.id;
    }).join("\n");
  }

  function queryCurrentWindowTabs() {
    return new Promise(function queryTabs(resolve, reject) {
      if (!globalThis.chrome || !chrome.tabs || typeof chrome.tabs.query !== "function") {
        reject(new Error("无法调用 Edge 标签页接口，请在 edge://extensions/ 重新加载此扩展"));
        return;
      }

      try {
        chrome.tabs.query({ currentWindow: true }, function onTabs(tabs) {
          const lastError = chrome.runtime && chrome.runtime.lastError;
          if (lastError) {
            reject(new Error(lastError.message));
            return;
          }
          resolve(Array.isArray(tabs) ? tabs : []);
        });
      } catch (error) {
        reject(error);
      }
    });
  }

  async function copyText(text, label) {
    if (!text) {
      setStatus("没有可复制的内容。", true);
      return;
    }

    let clipboardError = null;
    try {
      if (!navigator.clipboard || typeof navigator.clipboard.writeText !== "function") {
        throw new Error("Clipboard API 不可用");
      }
      await navigator.clipboard.writeText(text);
      setStatus("已复制 " + items.length + " 个" + label + "。", false);
      return;
    } catch (error) {
      clipboardError = error;
    }

    preview.value = text;
    preview.focus();
    preview.select();

    let copied = false;
    try {
      copied = document.execCommand("copy");
    } catch {
      copied = false;
    }

    if (copied) {
      setStatus("已复制 " + items.length + " 个" + label + "。", false);
    } else {
      setStatus("自动复制失败：" + errorMessage(clipboardError) + "。内容已选中，可按 Ctrl+C。", true);
    }
  }

  async function scanTabs() {
    setStatus("", false);
    summary.textContent = "正在扫描当前窗口……";
    counts.textContent = "";
    copyLinksButton.disabled = true;
    copyIdsButton.disabled = true;

    try {
      if (!globalThis.JmTabExtractor || typeof JmTabExtractor.collectJmTabs !== "function") {
        throw new Error("网址解析脚本未加载，请在 edge://extensions/ 重新加载此扩展");
      }

      const tabs = await queryCurrentWindowTabs();
      items = JmTabExtractor.collectJmTabs(tabs);
      const albumCount = items.filter(function isAlbum(item) {
        return item.type === "album";
      }).length;
      const photoCount = items.length - albumCount;

      preview.value = linksText();
      summary.textContent = items.length
        ? "找到 " + items.length + " 个 JM 页面"
        : "没有识别到 JM 本子或章节页面";
      counts.textContent = items.length ? "本子 " + albumCount + " · 章节 " + photoCount : "";
      copyLinksButton.disabled = items.length === 0;
      copyIdsButton.disabled = items.length === 0;

      if (!items.length) {
        setStatus("已检查当前窗口的 " + tabs.length + " 个标签页。", false);
      }
    } catch (error) {
      items = [];
      preview.value = "";
      counts.textContent = "";
      copyLinksButton.disabled = true;
      copyIdsButton.disabled = true;
      showUnexpectedError(error);
    }
  }

  copyLinksButton.addEventListener("click", function copyLinks() {
    copyText(linksText(), "链接");
  });
  copyIdsButton.addEventListener("click", function copyIds() {
    copyText(idsText(), "车号");
  });
  refreshButton.addEventListener("click", scanTabs);

  scanTabs();
})();
