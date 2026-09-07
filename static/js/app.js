/* ==========================================================================
   JMComic Downloader GUI · 前端逻辑
   结构：视图切换 → Toast → 窗口控制 → 健康检查 → 在线预览（搜索/详情/阅读器）
   ========================================================================== */
(function () {
  "use strict";

  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));

  /* ---------------- 外观偏好（localStorage：主题 / 界面缩放 / 动效） ----------------
     尽早 apply：脚本在 body 尾部执行，此时尚未首帧绘制，可避免主题闪烁。 */
  const UI_PREF_KEY = "jm.uiPrefs";
  const UIPrefs = {
    load() {
      try { return JSON.parse(localStorage.getItem(UI_PREF_KEY)) || {}; }
      catch (err) { return {}; }
    },
    save(p) { try { localStorage.setItem(UI_PREF_KEY, JSON.stringify(p)); } catch (err) { /* 隐私模式静默 */ } },
    apply() {
      const p = this.load();
      document.documentElement.dataset.theme = p.theme === "dark" ? "dark" : "light";
      const z = Number(p.zoom) || 1;
      document.body.style.zoom = z > 1 ? String(z) : "";
      // .app 的高度/最小宽度按 1/zoom 补偿（CSS calc 消费），避免缩放后溢出视口
      document.documentElement.style.setProperty("--ui-zoom", String(z > 0 ? z : 1));
      document.body.classList.toggle("no-motion", p.motion === "off");
      return p;
    },
    set(patch) {
      const p = Object.assign(this.load(), patch);
      this.save(p);
      this.apply();
      return p;
    },
  };
  UIPrefs.apply();

  /* ---------------- 视图切换（侧栏导航） ---------------- */

  const navItems = $$("#side-nav .nav-item");
  const views = $$(".view");

  function switchView(name) {
    navItems.forEach((item) => item.classList.toggle("is-active", item.dataset.view === name));
    views.forEach((view) => {
      const active = view.dataset.view === name;
      view.classList.toggle("is-active", active);
      if (active) {
        const heading = view.querySelector("h1");
        if (heading) document.title = "JMComic 下载器 · " + heading.textContent;
      }
    });
    // 阅读器占满内容区时，恢复/进入滚动模式互斥
    $(".content").classList.toggle("reader-mode", name === "preview" && PV.isReaderActive());
    // 切入下载任务视图时立即拉一次任务列表（平时由定时器兜底刷新）
    if (name === "download") dlRefresh();
    // 切入下载历史视图时立即拉一次历史记录
    if (name === "history") histRefresh();
  }
  navItems.forEach((item) => item.addEventListener("click", () => switchView(item.dataset.view)));

  /* ---------------- Toast（自制气泡，多级可扩展） ---------------- */

  window.jmToast = function (text, ms, kind) {
    const host = $("#toast-host");
    const el = document.createElement("div");
    el.className = "toast" + (kind ? " toast-" + kind : "");
    el.textContent = text;
    host.appendChild(el);
    requestAnimationFrame(() => el.classList.add("show"));
    setTimeout(() => {
      el.classList.remove("show");
      setTimeout(() => el.remove(), 220);
    }, ms || 2800);
  };

  /* 自绘确认框（替代浏览器原生 confirm）：返回 Promise<boolean>
     - role=dialog + aria-modal；Esc 关闭（阻止冒泡，避免连带触发全屏/阅读器的 Esc 逻辑）
     - Tab 焦点圈定在两个按钮间循环，不逃逸到背后页面
     - danger 场景默认聚焦「取消」，防止回车误确认 */
  window.jmConfirm = function (title, text, opts) {
    opts = opts || {};
    return new Promise((resolve) => {
      const mask = document.createElement("div");
      mask.className = "jm-modal-mask";
      const box = document.createElement("div");
      box.className = "jm-modal";
      box.setAttribute("role", "dialog");
      box.setAttribute("aria-modal", "true");
      box.setAttribute("aria-label", title);
      const t = document.createElement("p");
      t.className = "jm-modal-title";
      t.textContent = title;
      const d = document.createElement("p");
      d.className = "jm-modal-text";
      d.textContent = text || "";
      const actions = document.createElement("div");
      actions.className = "jm-modal-actions";
      const cancelBtn = document.createElement("button");
      cancelBtn.type = "button";
      cancelBtn.className = "btn btn-ghost";
      cancelBtn.textContent = opts.cancelText || "取消";
      const okBtn = document.createElement("button");
      okBtn.type = "button";
      okBtn.className = "btn " + (opts.danger ? "btn-danger" : "btn-primary");
      okBtn.textContent = opts.okText || "确定";
      const close = (val) => {
        mask.removeEventListener("keydown", onKey);
        mask.classList.remove("show");
        setTimeout(() => mask.remove(), 180);
        resolve(val);
      };
      const onKey = (e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          close(false);
          return;
        }
        if (e.key !== "Tab") return;
        // 焦点圈定：在两个按钮之间循环
        const focusables = [cancelBtn, okBtn];
        const i = focusables.indexOf(document.activeElement);
        e.preventDefault();
        const next = i < 0 ? 0 : (i + (e.shiftKey ? focusables.length - 1 : 1)) % focusables.length;
        focusables[next].focus();
      };
      cancelBtn.addEventListener("click", () => close(false));
      okBtn.addEventListener("click", () => close(true));
      mask.addEventListener("click", (e) => {
        if (e.target === mask) close(false);
      });
      mask.addEventListener("keydown", onKey);
      actions.append(cancelBtn, okBtn);
      box.append(t, d, actions);
      mask.appendChild(box);
      document.body.appendChild(mask);
      requestAnimationFrame(() => mask.classList.add("show"));
      // 危险操作默认聚焦「取消」：回车不再等于确认
      (opts.danger ? cancelBtn : okBtn).focus();
    });
  };

  /* 信息弹窗（单按钮）：快捷键说明等只读提示。lines 支持字符串数组逐行展示。 */
  window.jmAlert = function (title, lines, opts) {
    opts = opts || {};
    return new Promise((resolve) => {
      const mask = document.createElement("div");
      mask.className = "jm-modal-mask";
      const box = document.createElement("div");
      box.className = "jm-modal jm-modal-wide";
      box.setAttribute("role", "dialog");
      box.setAttribute("aria-modal", "true");
      box.setAttribute("aria-label", title);
      const t = document.createElement("p");
      t.className = "jm-modal-title";
      t.textContent = title;
      const d = document.createElement("div");
      d.className = "jm-modal-text jm-modal-lines";
      (Array.isArray(lines) ? lines : [String(lines || "")]).forEach((line) => {
        const p = document.createElement("p");
        p.textContent = line;
        d.appendChild(p);
      });
      const actions = document.createElement("div");
      actions.className = "jm-modal-actions";
      const okBtn = document.createElement("button");
      okBtn.type = "button";
      okBtn.className = "btn btn-primary";
      okBtn.textContent = opts.okText || "知道了";
      const close = () => {
        mask.removeEventListener("keydown", onKey);
        mask.classList.remove("show");
        setTimeout(() => mask.remove(), 180);
        resolve();
      };
      const onKey = (e) => {
        if (e.key === "Escape" || e.key === "Enter") {
          e.preventDefault();
          e.stopPropagation();
          close();
        }
      };
      okBtn.addEventListener("click", close);
      mask.addEventListener("click", (e) => { if (e.target === mask) close(); });
      mask.addEventListener("keydown", onKey);
      actions.appendChild(okBtn);
      box.append(t, d, actions);
      mask.appendChild(box);
      document.body.appendChild(mask);
      requestAnimationFrame(() => mask.classList.add("show"));
      okBtn.focus();
    });
  };

  /* ---------------- 标题栏窗口控制 ---------------- */

  // 独立阅读窗内的窗口按钮必须作用于 reader 窗（主窗/阅读窗共用同一前端）；
  // 直接读 URL（?w=reader）判定，规避对脚本后段 READER_WIN const 的时序依赖。
  function readerWindowActive() {
    return new URLSearchParams(location.search).get("w") === "reader";
  }

  function windowAction(action, params) {
    // 阅读窗内自动补 wid=reader；主窗不传（后端默认 main，兼容 --browser 无壳）
    const merged = Object.assign({}, params || {});
    if (readerWindowActive() && merged.wid === undefined) merged.wid = "reader";
    // params（如边缘缩放 {edges}）拼为 query；无 params 维持原 GET 路径。
    const qs = Object.keys(merged)
      ? "?" + Object.keys(merged)
          .filter((k) => merged[k] !== undefined && merged[k] !== null && merged[k] !== "")
          .map((k) => encodeURIComponent(k) + "=" + encodeURIComponent(merged[k]))
          .join("&")
      : "";
    const call = () =>
      fetch("/api/window/" + action + qs)
        .then((r) => r.json().catch(() => null))
        .catch(() => null);
    // 冷启动头几秒窗口控制器可能还没注册（后端返回 window not ready /
    // reader window not ready），此时不能静默吞掉——带递增间隔补发重试（最多3次，
    // 覆盖 WebView2 冷启动窗口晚建），按钮点了必定生效。
    const READY_ERRS = ["window not ready", "reader window not ready"];
    let tries = 0;
    const attempt = () =>
      call().then((data) => {
        if (data && data.ok === false && READY_ERRS.includes(data.error) && tries < 3) {
          tries += 1;
          return new Promise((resolve) => setTimeout(() => resolve(attempt()), 300 * tries));
        }
        return data;
      });
    return attempt();
  }
  $("#win-min").addEventListener("click", () => windowAction("minimize"));

  // 最大化/还原图标随窗口状态同步（Aero Snap、双击标题栏、按钮点击、F11 全屏都会触发）
  let maxIconTimer = 0;
  // 全屏态标注按钮文案（阅读窗底栏「全屏/退出全屏」等带 [data-fsbtn] 的元素）
  function syncFullscreenChrome() {
    const fsOn = document.body.classList.contains("is-fullscreen");
    document.querySelectorAll("[data-fsbtn]").forEach((b) => {
      b.textContent = fsOn ? "退出全屏" : "全屏";
      b.title = fsOn ? "退出全屏（Esc / F11）" : "全屏（F11）";
    });
  }
  function syncMaxIcon() {
    const qs = readerWindowActive() ? "?wid=reader" : "";
    fetch("/api/window/is-maximized" + qs)
      .then((r) => r.json())
      .then((d) => {
        // 全屏视为“已占满”：图标还原态与边缘缩放逻辑复用 is-maximized
        document.body.classList.toggle(
          "is-maximized", !!(d.maximized || d.fullscreen)
        );
        document.body.classList.toggle("is-fullscreen", !!d.fullscreen);
        syncFullscreenChrome();
      })
      .catch(() => {});
  }
  function syncMaxIconSoon(delay) {
    clearTimeout(maxIconTimer);
    maxIconTimer = setTimeout(syncMaxIcon, delay || 300);
  }
  // 进入/退出自建全屏（主窗 F11 / 阅读窗底栏按钮）。后端按 wid 分别全屏。
  function toggleFullscreenUI() {
    return windowAction("toggle-fullscreen").then((d) => {
      if (!d || d.ok === false) return d;   // 无窗口壳（--browser）时静默忽略
      const fsOn = !!d.fullscreen;
      document.body.classList.toggle("is-fullscreen", fsOn);
      document.body.classList.toggle(
        "is-maximized",
        fsOn || document.body.classList.contains("is-maximized")
      );
      syncFullscreenChrome();
      return d;
    });
  }
  $("#win-max").addEventListener("click", () => {
    // 全屏中再点“最大化/还原”按钮 = 退出全屏
    if (document.body.classList.contains("is-fullscreen")) toggleFullscreenUI();
    else windowAction("toggle-maximize");
    syncMaxIconSoon(250);
  });
  $("#win-close").addEventListener("click", () => windowAction("close"));
  window.addEventListener("resize", () => syncMaxIconSoon(150));
  syncMaxIconSoon(600);

  // 全屏快捷键（窗口级）：F11 切换、Esc 退出。注册在阅读器快捷键之前，
  // 退出全屏时 stopImmediatePropagation，防止“返回/关目录”逻辑被连带触发。
  document.addEventListener("keydown", (e) => {
    const fsOn = document.body.classList.contains("is-fullscreen");
    const zoomLayer = document.querySelector(".pv-zoom");
    const zoomOpen = !!zoomLayer && !zoomLayer.hidden;
    if (e.key === "F11" && !zoomOpen) {
      e.preventDefault();
      toggleFullscreenUI().then(() => syncMaxIconSoon(250));
    } else if (e.key === "Escape" && fsOn) {
      if (zoomOpen) return;   // 放大浮层自己处理 Esc（先关放大）
      e.preventDefault();
      e.stopImmediatePropagation();
      toggleFullscreenUI().then(() => syncMaxIconSoon(250));
    }
  });

  // 自绘标题栏拖动：pywebview 壳 frameless 窗口无系统标题栏可命中 →
  // mousedown 时通知后端 /api/window/drag；WndProc 正常时 WM_NCHITTEST 已把
  // 标题栏判为 HTCAPTION、鼠标消息不进 DOM，此路径仅为 WndProc 未装时的兜底。
  const titlebar = document.querySelector(".titlebar");
  if (titlebar) {
    titlebar.addEventListener("mousedown", (e) => {
      if (e.button !== 0) return;
      if (e.target.closest(".win-btn, #net-badge")) return;
      // 全屏态不提供拖动（窗口已铺满屏幕，后端 NCHITTEST 亦返回 HTCLIENT）
      if (document.body.classList.contains("is-fullscreen")) return;
      e.preventDefault();  // 防止 text_select 选中标题栏文字
      windowAction("drag");
    });
    titlebar.addEventListener("dblclick", (e) => {
      if (e.target.closest(".win-btn, #net-badge")) return;
      if (document.body.classList.contains("is-fullscreen")) {
        toggleFullscreenUI();   // 全屏中双击标题栏 = 退出全屏
      } else {
        windowAction("toggle-maximize");
      }
    });
  }

  /* ---------------- 边缘缩放（pywebview 壳：WndProc WM_NCHITTEST 原生处理边缘，
     此 DOM 监听仅在 WndProc 未装（窗口晚建）时兜底；reader 窗内自动带 wid） ------ */
  const EDGE_PX = 8;
  function edgeZone(cx, cy, w, h) {
    // 最大化/全屏时无边框可拖
    if (document.body.classList.contains("is-maximized")) return "";
    if (document.body.classList.contains("is-fullscreen")) return "";
    const nearL = cx < EDGE_PX, nearR = cx > w - EDGE_PX;
    const nearT = cy < EDGE_PX, nearB = cy > h - EDGE_PX;
    if (nearT && nearL) return "top-left";
    if (nearT && nearR) return "top-right";
    if (nearB && nearL) return "bottom-left";
    if (nearB && nearR) return "bottom-right";
    if (nearL) return "left";
    if (nearR) return "right";
    if (nearT) return "top";
    if (nearB) return "bottom";
    return "";
  }
  window.addEventListener("mousemove", (e) => {
    const zone = edgeZone(e.clientX, e.clientY, window.innerWidth, window.innerHeight);
    const prev = document.body.dataset.edgeZone || "";
    if (prev !== zone) document.body.dataset.edgeZone = zone;
  });
  // capture 阶段拦截：8px 边缘 mousedown → 系统缩放；阻止冒泡到标题栏拖动
  document.addEventListener(
    "mousedown",
    (e) => {
      if (e.button !== 0) return;
      if (e.target.closest && e.target.closest(".win-btn, #net-badge")) return;
      const zone = edgeZone(e.clientX, e.clientY, window.innerWidth, window.innerHeight);
      if (!zone) return;
      e.preventDefault();
      e.stopPropagation();
      windowAction("resize", { edges: zone.replace("-", ",") });
    },
    true
  );

  /* ---------------- 健康检查与版本 ---------------- */

  const badge = $("#net-badge");
  let lastHealth = null;
  /* 站点可达性由预览请求的成败驱动（/api/preview/status 不触发网络请求，无法探测站点，
     故不做后台探测轮询）：最近一次预览请求失败 → site 态；任一预览请求成功 → 恢复 ok */
  let siteDown = false;

  function setSiteReachable(ok) {
    const next = !ok;
    if (siteDown === next) return;
    siteDown = next;
    paintNetBadge();
  }

  function paintNetBadge() {
    if (!lastHealth) {
      badge.dataset.state = "down";
      badge.textContent = "本地服务未连接";
      badge.title = "";
      return;
    }
    if (siteDown) {
      badge.dataset.state = "site";
      badge.textContent = "站点暂不可达";
      badge.title = "本地服务正常，但最近一次预览请求失败（站点波动或代理异常）";
      return;
    }
    badge.dataset.state = "ok";
    badge.textContent = "本地服务已就绪";
  }

  /* 徽章 tooltip 补充当前生效的客户端实现与代理（status 为纯本地读，不触发网络） */
  async function refreshStatusTip() {
    try {
      const res = await fetch("/api/preview/status", { cache: "no-store" });
      const d = await res.json().catch(() => null);
      if (!d || d.ok === false || siteDown) return;
      const implLabel = d.impl === "html" ? "网页端" : "移动端 API";
      badge.title = "客户端：" + implLabel + " · 代理：" + (d.proxy || "跟随系统");
    } catch (err) { /* 静默：tooltip 缺失不影响功能 */ }
  }

  async function health() {
    try {
      const res = await fetch("/api/health", { cache: "no-store" });
      if (!res.ok) throw new Error("bad status");
      const data = await res.json();
      lastHealth = data;
      paintNetBadge();
      refreshStatusTip();
      const ver = $("#about-version");
      if (ver && data.app) ver.textContent = data.app;
      const outPath = $("#output-path");
      if (outPath && data.outputDir) outPath.textContent = data.outputDir;
      const foot = $("#foot-version");
      if (foot) foot.textContent = data.app;
      return data;
    } catch (err) {
      lastHealth = null;
      paintNetBadge();
      return null;
    }
  }

  health();
  setInterval(health, 8000);

  /* ---------------- 设置：打开输出目录 ---------------- */

  /* 打开系统文件管理器中的目录（统一走 /api/open-path，pywebview 壳 / 浏览器
     模式均可用；旧 pywebview 桥 window.pywebview.api.open_path 由 HTTP 端点取代） */
  async function openPath(path) {
    if (!path) {
      jmToast("没有可打开的目录", 2600, "warn");
      return;
    }
    try {
      const res = await fetch("/api/open-path?path=" + encodeURIComponent(path));
      const data = await res.json().catch(() => null);
      if (!data || data.ok !== true) {
        jmToast("无法打开目录：" + ((data && data.error) || "未知错误"), 3200, "warn");
      }
    } catch (err) {
      jmToast("打开目录失败：" + err.message, 3200, "warn");
    }
  }

  $("#open-output").addEventListener("click", async () => {
    const data = await health();
    if (!data) return;
    openPath(data.outputDir);
  });

  /* ---------------- 设置页：外观主题 / 界面缩放 / 动效（localStorage，即时生效） ---------------- */
  (function bindAppearanceSettings() {
    const themeSel = $("#set-theme");
    const zoomSel = $("#set-zoom");
    const motionSel = $("#set-motion");
    if (!themeSel || !zoomSel || !motionSel) return;
    const p = UIPrefs.load();
    themeSel.value = p.theme === "dark" ? "dark" : "light";
    zoomSel.value = String(Number(p.zoom) || 1);
    motionSel.value = p.motion === "off" ? "off" : "on";
    themeSel.addEventListener("change", () => UIPrefs.set({ theme: themeSel.value }));
    zoomSel.addEventListener("change", () => UIPrefs.set({ zoom: Number(zoomSel.value) || 1 }));
    motionSel.addEventListener("change", () => UIPrefs.set({ motion: motionSel.value }));
  })();

  /* ========================================================================
     在线预览（只读，不落盘）
     ======================================================================== */

  const PV = {
    kwInput: $("#preview-keyword"),
    goBtn: $("#preview-go"),
    pages: {
      home: $("#pv-home"),
      results: $("#pv-results"),
      detail: $("#pv-detail"),
      reader: $("#pv-reader"),
    },
    grid: $("#pv-grid"),
    meta: $("#pv-results-meta"),
    detail: $("#pv-detail"),
    moreRow: $("#pv-more-row"),
    moreBtn: $("#pv-load-more"),
    state: {
      search: null,       // {query, page, total, loading, ended}
      album: null,        // 当前详情
      chapters: [],       // 详情章节列表上下文
      readerAlbum: null,  // 阅读器所在本子
      readerChapterIndex: -1,
    },
    reader: { scroll: null, nextToLoad: 0, total: 0, photo: null, timer: null, sentinel: null },
  };

  /* ---- 轻量元素工厂（动态内容一律 textContent，防注入） ---- */
  function el(tag, cls, text) {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  }

  /* ---- 页面切换（内部） ---- */
  function showPage(name) {
    Object.keys(PV.pages).forEach((key) => {
      PV.pages[key].hidden = key !== name;
    });
    $(".content").classList.toggle("reader-mode", name === "reader");
    // 回顶：阅读器滚动容器自己管理，其余滚动 content
    if (name !== "reader") {
      const content = $(".content");
      if (content) content.scrollTop = 0;
    } else {
      PV.reader.scroll && (PV.reader.scroll.scrollTop = 0);
    }
    const activeNav = $('.nav-item[data-view="preview"]');
    if (activeNav) activeNav.classList.add("is-active");
  }
  PV.isReaderActive = () => !PV.pages.reader.hidden;

  /* ---- 搜索历史（localStorage 持久化；搜索一次记录一条；点击回填搜索；右键可删除） ---- */
  const HISTORY_KEY = "jm.searchHistory";
  const HISTORY_MAX = 50;
  const SearchHistory = {
    load() {
      try {
        const raw = localStorage.getItem(HISTORY_KEY);
        const arr = raw ? JSON.parse(raw) : [];
        return Array.isArray(arr) ? arr.filter((x) => typeof x === "string" && x.trim()) : [];
      } catch (err) {
        return [];
      }
    },
    save(list) {
      try { localStorage.setItem(HISTORY_KEY, JSON.stringify(list)); } catch (err) { /* 容量满/隐私模式时静默 */ }
    },
    add(kw) {
      kw = (kw || "").trim();
      if (!kw) return;
      let list = this.load();
      list = [kw].concat(list.filter((x) => x !== kw));
      if (list.length > HISTORY_MAX) list = list.slice(0, HISTORY_MAX);
      this.save(list);
      this.render();
    },
    remove(kw) {
      this.save(this.load().filter((x) => x !== kw));
      this.render();
    },
    render() {
      const host = $("#pv-history");
      if (!host) return;
      host.innerHTML = "";
      host.appendChild(el("div", "pv-history-title", "搜索历史"));
      const list = this.load();
      if (!list.length) {
        host.appendChild(el("div", "pv-history-empty", "暂无搜索历史 · 搜索一次便会记录在这里"));
        return;
      }
      const ul = el("ul", "pv-history-list");
      list.forEach((kw) => {
        const li = el("li", "pv-history-item");
        li.title = kw;
        const kwEl = el("span", "pv-history-kw", kw);
        const del = el("button", "pv-history-del");
        del.type = "button";
        del.title = "删除这条记录";
        del.innerHTML = '<svg viewBox="0 0 12 12" width="10" height="10"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>';
        del.addEventListener("click", (e) => {
          e.stopPropagation();
          this.remove(kw);
          jmToast("已删除搜索记录", 1800);
        });
        li.append(kwEl, del);
        li.addEventListener("click", () => {
          PV.kwInput.value = kw;
          doSearch(kw, 1, false);
        });
        li.addEventListener("contextmenu", (e) => {
          e.preventDefault();
          e.stopPropagation();
          this.remove(kw);
          jmToast("已删除搜索记录", 1800);
        });
        ul.appendChild(li);
      });
      host.appendChild(ul);
    },
  };

  /* ---- 搜索入口 ---- */
  function currentQuery() {
    return PV.kwInput.value.trim();
  }

  async function doSearch(query, page, append) {
    if (!query) {
      jmToast("请输入关键词或车号", 2400, "warn");
      PV.kwInput.focus();
      return;
    }
    if (!append) SearchHistory.add(query);
    if (!append) PV.state.search = { query, page: 1, total: 0, loading: true, ended: false };
    else PV.state.search = Object.assign({}, PV.state.search, { page, loading: true });

    PV.goBtn.disabled = true;
    PV.goBtn.classList.add("is-loading");
    if (!append) {
      PV.grid.innerHTML = "";
      for (let i = 0; i < 8; i += 1) PV.grid.appendChild(el("div", "pv-card pv-skeleton"));
      PV.meta.textContent = "正在搜索…";
      PV.moreRow.hidden = true;
      clearBanner();
      showPage("results");
    }
    const queryText = append ? query : PV.state.search.query;

    try {
      const data = await apiGet("/api/search?q=" + encodeURIComponent(queryText) + "&page=" + page);
      setSiteReachable(true);
      const items = data.items || [];
      PV.state.search = { query: queryText, page, total: data.total || 0, loading: false, ended: false };
      if (append) {
        PV.state.search.items = (PV.state.search.items || []).concat(items);
      } else {
        PV.state.search.items = items;
      }
      PV.state.search.ended = items.length < (data.pageSize || 20);

      // 车号直达：单结果且 id==查询词 → 直接进详情
      if (!append && /^\d+$/.test(queryText) && items.length === 1 && items[0].id === queryText) {
        openAlbum(items[0].id);
        return;
      }
      renderResults();
    } catch (err) {
      setSiteReachable(false);
      PV.state.search.loading = false;
      if (!append) {
        PV.grid.innerHTML = "";
        PV.meta.textContent = "";
        showBanner(err.message || "搜索失败");
      } else {
        jmToast("加载失败：" + err.message, 3200, "warn");
      }
    } finally {
      PV.goBtn.disabled = false;
      PV.goBtn.classList.remove("is-loading");
    }
  }

  function renderResults() {
    const s = PV.state.search;
    if (!s || !s.items) return;
    PV.grid.innerHTML = "";
    if (s.items.length === 0) {
      const empty = el("div", "pv-inline-empty");
      empty.appendChild(el("p", "pv-inline-empty-title", "没有找到相关内容"));
      empty.appendChild(el("p", "pv-inline-empty-text", "试试更短的关键词、作者名，或直接输入车号"));
      PV.grid.appendChild(empty);
      PV.meta.textContent = "关键词「" + s.query + "」没有结果";
      PV.moreRow.hidden = true;
      return;
    }
    PV.meta.textContent = "「" + s.query + "」 · 共 " + s.total + " 本" + (s.page > 1 ? "（第 " + s.page + " 页）" : "");
    // stagger 入场：新搜索全部卡片波浪淡入；「加载更多」时旧卡 no-anim 不重播，
    // 只有新增部分从 0 计数波浪淡入（--st 由 CSS animation-delay 消费）
    const prevCount = s.renderedCount || 0;
    s.items.forEach((item, idx) => {
      const card = albumCard(item);
      if (idx < prevCount) {
        card.classList.add("no-anim");
      } else {
        card.style.setProperty("--st", Math.min(idx - prevCount, 11));
      }
      PV.grid.appendChild(card);
    });
    s.renderedCount = s.items.length;
    PV.moreRow.hidden = s.ended;
  }

  function albumCard(item) {
    const card = el("div", "pv-card");
    const coverBox = el("div", "pv-cover");
    const img = document.createElement("img");
    img.className = "pv-cover-img";
    img.loading = "lazy";
    img.alt = "";
    img.src = "/api/cover?aid=" + encodeURIComponent(item.id) + "&size=_3x4";
    img.addEventListener("error", () => {
      coverBox.classList.add("pv-cover-fallback");
      img.remove();
      coverBox.appendChild(el("span", "pv-cover-id", "JM" + item.id));
    });
    coverBox.appendChild(img);
    card.appendChild(coverBox);
    card.appendChild(el("div", "pv-card-title", item.title || ("JM" + item.id)));
    card.appendChild(el("div", "pv-card-sub", [item.author, item.id].filter(Boolean).join(" · ")));
    card.title = (item.title || "") + "\nJM" + item.id;
    card.addEventListener("click", () => openAlbum(item.id));
    return card;
  }

  /* ---- 详情 ---- */

  // 本页面（窗口）直接提交过的 job_id：跨窗口轮询到新任务时用于区分
  // "别人加的"（阅读窗 / 另一窗口）与"本页自己加的"，避免重复打扰提示。
  const localJobIds = new Set();

  // 预览页发起下载：默认选项（跟随服务端默认输出目录），成功后按钮定格为"已加入任务"
  async function pvSubmitDownload(kind, ids, label, btn) {
    if (btn) btn.disabled = true;
    try {
      const res = await fetch("/api/jobs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ kind, ids }),
      });
      const data = await res.json().catch(() => null);
      if (!res.ok || !data || data.ok === false) {
        throw new Error((data && data.message) || "提交失败（HTTP " + res.status + "）");
      }
      if (data.job && data.job.job_id) localJobIds.add(data.job.job_id);
      jmToast("已加入下载队列：" + label + "，进度可在「下载任务」页查看");
      if (btn) {
        btn.textContent = "已加入任务";
        btn.classList.remove("btn-primary");
        btn.classList.add("btn-ghost");
      }
      return data;
    } catch (err) {
      jmToast(err.message || "提交下载失败", 3200, "warn");
      if (btn) btn.disabled = false;
      return null;
    }
  }

  // 预览详情页「下载此本」→ 不直接入队，跳到「下载任务」页预填车号/章节，便于配置后发起。
  // 独立阅读窗没有「下载任务」配置页 → 回退为直接入队。
  function pvGoDownload(kind, ids, label, btn) {
    const isReaderWin = document.body.classList.contains("is-reader-win");
    if (isReaderWin) {
      return pvSubmitDownload(kind, ids, label, btn);
    }
    const idsEl = $("#dlv-ids");
    const text = ids.join("\n");
    idsEl.value = (idsEl.value || "").trim() ? idsEl.value.trim() + "\n" + text : text;
    // 同步下载类型（本子 / 章节）
    document.querySelectorAll("#dlv-kind .dlv-seg-item").forEach((b) =>
      b.classList.toggle("is-active", b.dataset.kind === (kind === "photo" ? "photo" : "album")));
    if (typeof idsEl.dispatchEvent === "function") {
      idsEl.dispatchEvent(new Event("input", { bubbles: true })); // 触发实时解析/偏好保存
    }
    switchView("download");
    idsEl.focus();
    jmToast("已填入下载任务：" + label + "，配置选项后点「开始下载」", 3600, "ok");
  }

  async function openAlbum(id) {
    PV.detail.innerHTML = "";
    const loading = el("div", "pv-loading");
    loading.appendChild(el("div", "pv-loading-bar"));
    loading.appendChild(el("p", "pv-loading-text", "正在拉取本子详情…"));
    PV.detail.appendChild(loading);
    showPage("detail");
    try {
      const album = await apiGet("/api/album?id=" + encodeURIComponent(id));
      setSiteReachable(true);
      PV.state.album = album;
      PV.state.chapters = album.chapters || [];
      renderAlbum(album);
    } catch (err) {
      setSiteReachable(false);
      PV.detail.innerHTML = "";
      const empty = el("div", "pv-inline-empty");
      empty.appendChild(el("p", "pv-inline-empty-title", "详情加载失败"));
      empty.appendChild(el("p", "pv-inline-empty-text", err.message || "未知错误"));
      empty.appendChild(backButton("返回重试", 1));
      PV.detail.appendChild(empty);
    }
  }

  function renderAlbum(album) {
    PV.detail.innerHTML = "";
    const toolbar = el("div", "pv-toolbar");
    toolbar.appendChild(backButton("返回结果", 1));
    toolbar.appendChild(el("div", "pv-crumb", "JM" + album.id + " · 详情"));
    toolbar.appendChild(el("span", "pv-toolbar-spacer"));
    const dlBtn = el("button", "btn btn-primary btn-sm", "下载此本");
    dlBtn.type = "button";
    dlBtn.title = "跳转到「下载任务」页配置后启动 JM" + album.id + " 整本下载";
    dlBtn.addEventListener("click", () => {
      pvGoDownload("album", [String(album.id)], "JM" + album.id, dlBtn);
    });
    toolbar.appendChild(dlBtn);
    PV.detail.appendChild(toolbar);

    const hero = el("div", "pv-hero");
    const coverBox = el("div", "pv-hero-cover");
    const img = document.createElement("img");
    img.className = "pv-hero-cover-img";
    img.alt = "";
    img.src = "/api/cover?aid=" + encodeURIComponent(album.id) + "&size=";
    img.addEventListener("error", () => {
      coverBox.classList.add("pv-cover-fallback");
      img.remove();
      coverBox.appendChild(el("span", "pv-cover-id", "JM" + album.id));
    });
    coverBox.appendChild(img);
    hero.appendChild(coverBox);

    const info = el("div", "pv-hero-info");
    info.appendChild(el("h2", "pv-hero-title", album.title || ("JM" + album.id)));
    const badges = el("div", "pv-badges");
    renderBadges(badges, album);
    if (badges.childElementCount) info.appendChild(badges);

    const facts = el("dl", "pv-facts");
    factRow(facts, "作者", (album.authors || []).join("、") || "未知");
    factRow(facts, "总页数", album.pageCount + " 页");
    factRow(facts, "章节", album.chapterCount + " 话");
    factRow(facts, "浏览", album.views || "—");
    factRow(facts, "喜欢", album.likes || "—");
    factRow(facts, "更新", album.updateDate || "—");
    info.appendChild(facts);

    if (album.description) {
      const desc = el("p", "pv-hero-desc", album.description);
      info.appendChild(desc);
    }
    hero.appendChild(info);
    PV.detail.appendChild(hero);

    // 章节目录（>24 话折叠，支持正序/倒序）
    PV.detail.appendChild(renderChapterSection(album));
  }

  const TAG_FOLD_LIMIT = 12;
  /* 标签超过 12 个时折叠为「＋N」，点击展开全部；展开态按本子记忆（本次会话内） */
  function renderBadges(host, album) {
    host.innerHTML = "";
    const tags = album.tags || [];
    const expanded = PV.tagsExpanded === String(album.id);
    (expanded ? tags : tags.slice(0, TAG_FOLD_LIMIT)).forEach((tag) =>
      host.appendChild(el("span", "pv-badge", tag)));
    if (tags.length > TAG_FOLD_LIMIT) {
      const more = el("button", "pv-badge pv-badge-more",
        expanded ? "收起标签" : "＋" + (tags.length - TAG_FOLD_LIMIT) + " 个标签");
      more.type = "button";
      more.addEventListener("click", (e) => {
        e.stopPropagation();
        PV.tagsExpanded = expanded ? "" : String(album.id);
        renderBadges(host, album);
      });
      host.appendChild(more);
    }
  }

  const CHAPTER_FOLD_LIMIT = 24;

  /* 章节列表区：超过 24 话默认折叠 + 正序/倒序切换；展开/倒序态按本子记忆（本次会话内） */
  function renderChapterSection(album) {
    const chapters = album.chapters || [];
    const aid = String(album.id);
    const expanded = PV.chExpanded === aid;
    const reversed = PV.chReversed === aid;

    const section = el("div", "pv-chapter-section");
    const head = el("div", "pv-chapter-head");
    head.appendChild(el("h3", "pv-chapter-title", "章节列表"));
    head.appendChild(el("span", "pv-chapter-count", chapters.length + " 话 · 点击在新窗口阅读"));
    head.appendChild(el("span", "pv-toolbar-spacer"));
    if (chapters.length > 1) {
      const sortBtn = el("button", "btn btn-ghost btn-sm", reversed ? "正序" : "倒序");
      sortBtn.type = "button";
      sortBtn.title = reversed ? "按章节号从小到大" : "最新话排最前";
      sortBtn.addEventListener("click", () => {
        PV.chReversed = reversed ? "" : aid;
        refreshChapterSection(album);
      });
      head.appendChild(sortBtn);
    }
    section.appendChild(head);

    const list = el("div", "pv-chapter-list");
    const shown = expanded ? chapters : chapters.slice(0, CHAPTER_FOLD_LIMIT);
    (reversed ? [...shown].reverse() : shown).forEach((ch) => list.appendChild(chapterRow(album, ch)));
    section.appendChild(list);

    if (chapters.length > CHAPTER_FOLD_LIMIT) {
      const fold = el("div", "pv-chapter-fold");
      const btn = el("button", "btn btn-ghost",
        expanded ? "收起章节列表" : "展开全部 " + chapters.length + " 话");
      btn.type = "button";
      btn.addEventListener("click", () => {
        PV.chExpanded = expanded ? "" : aid;
        refreshChapterSection(album);
      });
      fold.appendChild(btn);
      section.appendChild(fold);
    }
    return section;
  }

  function refreshChapterSection(album) {
    const cur = PV.detail.querySelector(".pv-chapter-section");
    if (cur) cur.replaceWith(renderChapterSection(album));
  }

  function chapterRow(album, ch) {
    const row = el("button", "pv-chapter");
    row.type = "button";
    const no = el("span", "pv-chapter-no", "第 " + ch.index + " 话");
    const name = el("span", "pv-chapter-name", ch.title || "未命名");
    const action = el("span", "pv-chapter-go", "在新窗口阅读");
    row.append(no, name, action);
    // 整行点击 → 单开独立阅读窗到该章；Enter/Space 由原生 <button> 语义触发 click
    // （不要重复挂 keydown，否则会与原生 click 双触发开两次窗）
    row.addEventListener("click", () => openReaderWindow(album, ch));
    return row;
  }

  function factRow(dl, label, value) {
    dl.appendChild(el("dt", "pv-fact-key", label));
    dl.appendChild(el("dd", "pv-fact-val", value));
  }

  /* ---- 阅读器 ---- */

  // 独立阅读窗模式（URL 带 ?w=reader）：由 pywebview 壳的独立窗口打开，
  // 复用主窗阅读器 DOM 结构，仅布局/入口/信息层级不同（CSS 按 body 类区分）。
  const READER_WIN = new URLSearchParams(location.search).get("w") === "reader";
  function isReaderWin() { return READER_WIN; }

  async function openReader(album, chapter) {
    const readerPage = PV.pages.reader;
    readerPage.innerHTML = "";
    const loading = el("div", "pv-loading");
    loading.appendChild(el("div", "pv-loading-bar"));
    loading.appendChild(el("p", "pv-loading-text", "正在加载章节图片清单…"));
    readerPage.appendChild(loading);
    showPage("reader");

    PV.state.readerAlbum = album;
    PV.state.readerChapterIndex = (album.chapters || []).findIndex((c) => c.id === chapter.id);
    try {
      const photo = await apiGet("/api/photo?id=" + encodeURIComponent(chapter.id));
      setSiteReachable(true);
      renderReader(photo);
    } catch (err) {
      setSiteReachable(false);
      readerPage.innerHTML = "";
      const empty = el("div", "pv-inline-empty");
      empty.appendChild(el("p", "pv-inline-empty-title", "章节加载失败"));
      empty.appendChild(el("p", "pv-inline-empty-text", err.message || "未知错误"));
      if (isReaderWin()) {
        // 阅读窗没有可返回的上级页面 → 提供重试
        const retryBtn = el("button", "btn btn-ghost pv-back", "重试");
        retryBtn.type = "button";
        retryBtn.addEventListener("click", () => openReader(album, chapter));
        empty.appendChild(retryBtn);
      } else {
        empty.appendChild(backButton("返回详情", 1));
      }
      readerPage.appendChild(empty);
    }
  }

  function renderReader(photo) {
    const readerPage = PV.pages.reader;
    readerPage.innerHTML = "";
    const album = PV.state.readerAlbum;
    const idx = PV.state.readerChapterIndex;
    const chapters = (album && album.chapters) || [];
    const rw = isReaderWin();
    const prevCh = idx > 0 ? chapters[idx - 1] : null;
    const nextCh = idx >= 0 && idx < chapters.length - 1 ? chapters[idx + 1] : null;

    // 顶栏：左 =（阅读窗）目录抽屉开关 /（主窗）返回详情
    const top = el("div", "pv-reader-top");
    if (rw) {
      const tocBtn = el("button", "btn btn-ghost btn-sm pv-toc-toggle", "目录");
      tocBtn.type = "button";
      tocBtn.title = "章节列表（默认收起，点击展开）";
      if (PV.readerWinToc) tocBtn.classList.add("is-active");
      tocBtn.addEventListener("click", () => {
        PV.readerWinToc = !PV.readerWinToc;
        readerPage.classList.toggle("toc-open", PV.readerWinToc);
        tocBtn.classList.toggle("is-active", PV.readerWinToc);
      });
      top.appendChild(tocBtn);
    } else {
      top.appendChild(backButton("返回详情", 1));
    }

    const titleBox = el("div", "pv-reader-titlebox");
    if (rw) {
      // 专辑名已呈现在窗口标题栏 → 这里突出当前章节
      titleBox.appendChild(el("span", "pv-reader-chapter pv-reader-chapter-lg",
        "第 " + photo.index + " 话" + (photo.title ? " · " + photo.title : "")));
    } else {
      titleBox.appendChild(el("span", "pv-reader-album",
        (album ? album.title : "") + " · JM" + (album ? album.id : photo.albumId)));
      titleBox.appendChild(el("span", "pv-reader-chapter",
        "第 " + photo.index + " 话 " + (photo.title || "")));
    }
    top.appendChild(titleBox);

    const nav = el("div", "pv-reader-nav");
    if (rw && album) {
      const dlAlbumBtn = el("button", "btn btn-primary btn-sm", "下载本子");
      dlAlbumBtn.type = "button";
      dlAlbumBtn.title = "将 JM" + album.id + " 整本加入下载队列";
      dlAlbumBtn.addEventListener("click", () => {
        pvSubmitDownload("album", [String(album.id)], "JM" + album.id, dlAlbumBtn);
      });
      nav.appendChild(dlAlbumBtn);
    }
    const dlBtn = el("button", "btn btn-ghost btn-sm", "下载本话");
    dlBtn.type = "button";
    dlBtn.title = "将本章节加入下载队列";
    dlBtn.addEventListener("click", () => {
      pvSubmitDownload("photo", [String(photo.id)],
        "JM" + (photo.albumId || album.id) + " 第 " + photo.index + " 话", dlBtn);
    });
    const prevBtn = el("button", "btn btn-ghost btn-sm" + (prevCh ? "" : " is-disabled"), "上一话");
    prevBtn.type = "button";
    prevBtn.disabled = !prevCh;
    if (prevCh) prevBtn.addEventListener("click", () => openReader(album, prevCh));
    const nextBtn = el("button", "btn btn-primary btn-sm" + (nextCh ? "" : " is-disabled"), "下一话");
    nextBtn.type = "button";
    nextBtn.disabled = !nextCh;
    if (nextCh) nextBtn.addEventListener("click", () => openReader(album, nextCh));
    nav.append(dlBtn, prevBtn, nextBtn);
    top.appendChild(nav);
    readerPage.appendChild(top);

    // 阅读窗：左侧章节抽屉（默认收起；开合状态跨章节保留）
    if (rw) {
      readerPage.appendChild(buildToc(album, chapters, idx));
      if (PV.readerWinToc) readerPage.classList.add("toc-open");
    }

    // 滚动区（阅读沉浸：深色画布）
    const scroll = el("div", "pv-reader-scroll");
    const stage = el("div", "pv-reader-stage");
    scroll.appendChild(stage);
    readerPage.appendChild(scroll);
    // 双击或右键已加载图片 → 自绘放大查看层（左键留给拖动平移，避免误触进浮层）
    const zoomableImg = (target) => {
      const img = target && target.closest ? target.closest(".pv-page-img") : null;
      if (!img || img.classList.contains("is-loading") || img.dataset.state !== "done") return null;
      return img;
    };
    scroll.addEventListener("contextmenu", (e) => {
      const img = zoomableImg(e.target);
      if (!img) return;
      e.preventDefault();
      openZoom(img);
    });
    // 双击进放大：比右键更易发现（功能一致，详见底栏提示与「快捷键」说明）
    scroll.addEventListener("dblclick", (e) => {
      const img = zoomableImg(e.target);
      if (img) { e.preventDefault(); openZoom(img); }
    });
    // 放大后左键拖动平移（未放大不拦截，保留原生滚动）
    bindReaderPan(scroll);
    // Ctrl+滚轮 → 阅读区局部缩放（不触浏览器整页缩放）
    bindReaderZoomWheel(scroll);

    // 底部信息条
    const foot = el("div", "pv-reader-foot");
    // 页码做成按钮：点击 → 内联输入页码直接跳转（P2-7）
    const countLabel = el("button", "pv-zoom-btn pv-reader-count", "第 1 / " + photo.pageCount + " 页");
    countLabel.type = "button";
    countLabel.title = "点击跳转到指定页";
    countLabel.addEventListener("click", () => openPageJump(countLabel));
    foot.appendChild(buildReaderZoombar());   // 缩放：− / 百分比 / ＋ / 适应宽度 / 1:1
    const tip = el("span", "pv-reader-tip",
      rw ? "双击/右键放大 · Ctrl+滚轮缩放 · ←→ 翻章 · 放大后可拖动"
         : "双击/右键放大 · Ctrl+滚轮缩放 · ←→ 翻章 · Esc 返回");
    foot.appendChild(tip);
    foot.appendChild(countLabel);
    // 快捷键说明入口（此前快捷键全靠猜，无可发现入口）
    const helpBtn = el("button", "pv-zoom-btn", "快捷键");
    helpBtn.type = "button";
    helpBtn.title = "查看阅读器快捷键";
    helpBtn.addEventListener("click", () => {
      window.jmAlert("阅读器快捷键", [
        "滚轮：上下翻页",
        "← / →：上一章 / 下一章",
        "Ctrl + 滚轮：阅读区缩放（25% ~ 400%）",
        "双击 / 右键图片：放大查看；放大层内 ← / → 翻页、滚轮/双击缩放、拖拽平移、Esc 关闭",
        "底栏「第 x / N 页」：点击可输入页码直接跳转",
        rw ? "Esc：收起章节目录" : "Esc：返回详情页",
        "F11：全屏 / 退出全屏",
      ]);
    });
    foot.appendChild(helpBtn);
    if (rw) {
      // 阅读窗全屏按钮（独立窗口仍可用 F11，这里给显式入口）
      const fsBtn = el("button", "pv-zoom-btn", "全屏");
      fsBtn.type = "button";
      fsBtn.dataset.fsbtn = "1";
      fsBtn.title = "全屏（F11）";
      if (document.body.classList.contains("is-fullscreen")) fsBtn.textContent = "退出全屏";
      fsBtn.addEventListener("click", () => toggleFullscreenUI().then(() => syncMaxIconSoon(150)));
      foot.appendChild(fsBtn);
    }
    readerPage.appendChild(foot);

    // 旧章节的观察器断开，避免泄漏与重复回调
    if (PV.reader && PV.reader.pageIO) PV.reader.pageIO.disconnect();
    if (PV.reader && PV.reader.sentinelIO) PV.reader.sentinelIO.disconnect();

    PV.reader = { scroll, stage, photo, nextToLoad: 0, total: photo.pageCount, countLabel, currentPage: 1 };
    PV.state.photo = photo;
    // 沿用上一章节的缩放偏好（换章不重置），换章时不做中心锚点
    applyReaderZoom(PV.readerZoom || 1, false);

    /* 当前页码：窄带 IntersectionObserver（视口 45% 处的横带）替代
       每次滚动逐图 getBoundingClientRect —— 长章节滚动零布局读取 */
    PV.reader.pageIO = new IntersectionObserver((entries) => {
      entries.forEach((e) => {
        if (!e.isIntersecting) return;
        const i = Number(e.target.dataset.page);
        if (i && PV.reader.countLabel) {
          PV.reader.currentPage = i;
          PV.reader.countLabel.textContent = "第 " + i + " / " + PV.reader.total + " 页";
        }
      });
    }, { root: scroll, rootMargin: "-45% 0px -54% 0px", threshold: 0 });

    // 惰性批量注入
    appendBatch(6);
    if (PV.reader.total > PV.reader.nextToLoad) {
      PV.reader.sentinel = el("div", "pv-reader-sentinel");
      stage.appendChild(PV.reader.sentinel);
      PV.reader.sentinelIO = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) appendBatch(24);
      }, { root: scroll, rootMargin: "1200px 0px" });
      PV.reader.sentinelIO.observe(PV.reader.sentinel);
    }

    // 贴底兜底：末页未越过 45% 页码横带时（短页），滚到底即钉为最后一页
    scroll.addEventListener("scroll", throttleScroll, { passive: true });
    updateReaderTitlebar();

    // 预取下一章图片清单（服务端 photo 有 LRU 缓存，翻章近乎瞬时）；错开首屏图片加载
    clearTimeout(PV.prefetchTimer);
    PV.prefetchTimer = setTimeout(() => prefetchNextChapter(), 1200);
  }

  /* 预取下一章：服务端对 photo 清单有 LRU 缓存，提前拉取让翻章秒开；失败静默 */
  function prefetchNextChapter() {
    const album = PV.state.readerAlbum;
    const idx = PV.state.readerChapterIndex;
    const chs = (album && album.chapters) || [];
    const next = idx >= 0 ? chs[idx + 1] : null;
    if (!next) return;
    if (PV.prefetchedId === next.id) return;
    PV.prefetchedId = next.id;
    apiGet("/api/photo?id=" + encodeURIComponent(next.id)).catch(() => {});
  }

  /* 页码跳转：点击底栏页码 → 内联输入 → Enter 跳转（未注入的页先批量注入） */
  function openPageJump(label) {
    const r = PV.reader;
    if (!r || r.jumping) return;
    r.jumping = true;
    const input = document.createElement("input");
    input.type = "text";
    input.inputMode = "numeric";
    input.className = "pv-jump-input";
    input.value = String(r.currentPage || 1);
    input.setAttribute("aria-label", "跳转到页码（1 ~ " + r.total + "）");
    label.replaceWith(input);
    input.focus();
    input.select();
    const done = (commit) => {
      if (!input.isConnected) return;
      input.replaceWith(label);
      r.jumping = false;
      if (!commit) return;
      const n = Math.round(Number(input.value));
      if (!Number.isFinite(n) || n < 1 || n > r.total) {
        jmToast("页码超出范围（1 ~ " + r.total + "）", 2400, "warn");
        return;
      }
      jumpToPage(n);
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") { e.preventDefault(); done(true); }
      else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done(false); }
      // 输入框内的 ←/→ 用于移动光标：阻止冒泡到阅读器的翻章快捷键
      else if (e.key === "ArrowLeft" || e.key === "ArrowRight") { e.stopPropagation(); }
    });
    input.addEventListener("blur", () => done(true));
  }

  function jumpToPage(n) {
    const r = PV.reader;
    if (!r || !r.stage) return;
    if (r.nextToLoad < n) appendBatch(n - r.nextToLoad);   // 未注入的页先注入
    const figs = $$(".pv-page", r.stage);
    const target = figs[Math.min(n, figs.length) - 1];
    if (!target) return;
    r.currentPage = n;
    if (r.countLabel) r.countLabel.textContent = "第 " + n + " / " + r.total + " 页";
    target.scrollIntoView({ block: "start", behavior: "smooth" });
  }

  /* ---- 阅读窗：左侧章节抽屉 ---- */
  function buildToc(album, chapters, curIdx) {
    const drawer = el("aside", "pv-toc");
    const head = el("div", "pv-toc-head");
    head.appendChild(el("span", "pv-toc-album",
      (album ? album.title : "") + " · JM" + (album ? album.id : "")));
    head.appendChild(el("span", "pv-toc-count", chapters.length + " 话"));
    drawer.appendChild(head);
    const list = el("div", "pv-toc-list");
    chapters.forEach((ch, i) => {
      const item = el("button", "pv-toc-item" + (i === curIdx ? " is-current" : ""));
      item.type = "button";
      item.appendChild(el("span", "pv-toc-no", "第 " + ch.index + " 话"));
      item.appendChild(el("span", "pv-toc-name", ch.title || "未命名"));
      item.addEventListener("click", () => {
        if (i === curIdx) {
          // 点当前章 = 收起抽屉
          PV.readerWinToc = false;
          const host = PV.pages.reader;
          host.classList.remove("toc-open");
          const t = host.querySelector(".pv-toc-toggle");
          if (t) t.classList.remove("is-active");
          return;
        }
        openReader(album, ch); // 翻章后保留抽屉展开，连续浏览更顺
      });
      list.appendChild(item);
    });
    drawer.appendChild(list);
    return drawer;
  }

  /* ---- 标题栏（阅读窗）：专辑名 · 章节位置同步 ---- */
  function updateReaderTitlebar() {
    if (!isReaderWin()) return;
    const album = PV.state.readerAlbum;
    const photo = PV.state.photo;
    const rd = $("#rd-title");
    if (!rd) return;
    let txt = "";
    if (album) txt = "JM" + album.id + (album.title ? " · " + album.title : "");
    if (photo) txt += " · 第 " + photo.index + " 话";
    rd.textContent = txt;
    rd.title = txt || "";
  }

  /* ---- 阅读区缩放（底栏按钮组控制；放大后左键可拖动平移） ---- */
  const READER_ZOOM_MIN = 0.25;
  const READER_ZOOM_MAX = 4;

  function buildReaderZoombar() {
    const bar = el("div", "pv-zoombar");
    const minus = el("button", "pv-zoom-step", "−");
    minus.type = "button";
    minus.title = "缩小";
    const pct = el("button", "pv-zoom-pct", "100%");
    pct.type = "button";
    pct.title = "恢复适应宽度";
    const plus = el("button", "pv-zoom-step", "＋");
    plus.type = "button";
    plus.title = "放大";
    const fit = el("button", "pv-zoom-btn", "适应宽度");
    fit.type = "button";
    fit.title = "按窗口宽度铺满";
    const raw = el("button", "pv-zoom-btn", "1:1");
    raw.type = "button";
    raw.title = "原始像素大小";
    minus.addEventListener("click", () => applyReaderZoom((PV.readerZoom || 1) * 0.8));
    plus.addEventListener("click", () => applyReaderZoom((PV.readerZoom || 1) * 1.25));
    pct.addEventListener("click", () => applyReaderZoom(1));
    fit.addEventListener("click", () => applyReaderZoom(1));
    raw.addEventListener("click", () => applyReaderZoomActual());
    bar.append(minus, pct, plus, fit, raw);
    PV.readerZoomLabel = pct;
    return bar;
  }

  /* 设置阅读区缩放（1 = 适应宽度）。keepCenter=true 时以视口中心为锚点，缩放不跳视线。 */
  function applyReaderZoom(next, keepCenter) {
    const r = PV.reader;
    if (!r || !r.scroll) return;
    const prev = PV.readerZoom || 1;
    const z = Math.min(READER_ZOOM_MAX, Math.max(READER_ZOOM_MIN, next));
    const sc = r.scroll;
    const cx = keepCenter ? sc.scrollLeft + sc.clientWidth / 2 : null;
    const cy = keepCenter ? sc.scrollTop + sc.clientHeight / 2 : null;
    PV.readerZoom = z;
    sc.style.setProperty("--reader-zoom", String(z));
    const zoomed = z > 1.001;
    $$(".pv-page-img", sc).forEach((img) => img.classList.toggle("is-zoomed", zoomed));
    sc.classList.toggle("is-pannable", zoomed);
    if (PV.readerZoomLabel) PV.readerZoomLabel.textContent = Math.round(z * 100) + "%";
    if (cx !== null && cy !== null && prev !== z) {
      const k = z / prev;
      sc.scrollLeft = Math.max(0, cx * k - sc.clientWidth / 2);
      sc.scrollTop = Math.max(0, cy * k - sc.clientHeight / 2);
    }
    if (!zoomed) sc.scrollLeft = 0;   // 回到适应宽度时横向归位
  }

  /* 1:1 原始像素：用已加载图的自然宽度 ÷ 适应宽度下的显示宽度 */
  function applyReaderZoomActual() {
    const r = PV.reader;
    if (!r || !r.scroll) return;
    const img = $$(".pv-page-img", r.scroll).filter((i) => i.naturalWidth)[0];
    if (!img) return;
    const shown = img.getBoundingClientRect().width;
    const base = shown / (PV.readerZoom || 1);
    if (base > 0 && img.naturalWidth) applyReaderZoom(img.naturalWidth / base);
  }

  /* 放大后（图片宽超出容器）左键按住拖动平移；未放大时完全不拦截，保留原生滚动 */
  function bindReaderPan(sc) {
    sc.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      if ((PV.readerZoom || 1) <= 1.001) return;
      if (e.target.closest && e.target.closest(".pv-page-retry")) return;
      PV.readerPan = { x: e.clientX, y: e.clientY, left: sc.scrollLeft, top: sc.scrollTop };
      sc.classList.add("is-panning");
      try { sc.setPointerCapture(e.pointerId); } catch (_) { /* 捕获失败不影响平移 */ }
      e.preventDefault();   // 避让 text_select 选中与原生拖图
    });
    sc.addEventListener("pointermove", (e) => {
      const p = PV.readerPan;
      if (!p) return;
      sc.scrollLeft = p.left - (e.clientX - p.x);
      sc.scrollTop = p.top - (e.clientY - p.y);
    });
    const endPan = (e) => {
      if (!PV.readerPan) return;
      PV.readerPan = null;
      sc.classList.remove("is-panning");
      try { sc.releasePointerCapture(e.pointerId); } catch (_) { /* 忽略 */ }
    };
    sc.addEventListener("pointerup", endPan);
    sc.addEventListener("pointercancel", endPan);
  }

  /* Ctrl/Cmd + 滚轮 → 阅读区缩放（锚定视口中心不跳视线）。拦截 preventDefault 避免整页缩放 */
  function bindReaderZoomWheel(sc) {
    sc.addEventListener("wheel", (e) => {
      if (!e.ctrlKey && !e.metaKey) return;   // 仅 Ctrl（macOS 兼容 Cmd）
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      applyReaderZoom((PV.readerZoom || 1) * factor, true);
    }, { passive: false });
  }

  /* ---- 图片放大查看（自绘浮层：双击/右键进入、滚轮/双击缩放、拖拽平移、←→ 翻页、Esc/× 关闭） ---- */
  const zoomState = { el: null, img: null, scale: 1, tx: 0, ty: 0, drag: null, index: -1, hint: null };

  function ensureZoomLayer() {
    if (zoomState.el) return;
    const layer = el("div", "pv-zoom");
    layer.hidden = true;
    const stage = el("div", "pv-zoom-stage");
    stage.appendChild(el("div", "pv-zoom-cell"));
    layer.appendChild(stage);
    const close = el("button", "pv-zoom-close", "×");
    close.type = "button";
    close.setAttribute("aria-label", "关闭放大");
    layer.appendChild(close);
    const hint = el("div", "pv-zoom-hint", "滚轮 / 双击 缩放 · 拖拽平移 · Esc 关闭");
    layer.appendChild(hint);
    document.body.appendChild(layer);
    zoomState.el = layer;
    zoomState.stage = stage;
    zoomState.cell = layer.querySelector(".pv-zoom-cell");
    zoomState.hint = hint;

    function apply() {
      if (!zoomState.img) return;
      zoomState.img.style.transform =
        "translate(" + zoomState.tx + "px," + zoomState.ty + "px) scale(" + zoomState.scale + ")";
    }
    function reset() {
      zoomState.scale = 1; zoomState.tx = 0; zoomState.ty = 0; apply();
    }
    function closeZoom() {
      if (zoomState.el.hidden) return;
      zoomState.el.hidden = true;
      if (zoomState.img) { zoomState.img.remove(); zoomState.img = null; }
      reset();
    }

    close.addEventListener("click", closeZoom);
    stage.addEventListener("dblclick", () => {
      // 双击：复位（已是原尺寸则关闭浮层）
      if (Math.abs(zoomState.scale - 1) < 0.01 && !zoomState.tx && !zoomState.ty) closeZoom();
      else reset();
    });
    stage.addEventListener("wheel", (e) => {
      e.preventDefault();
      const delta = e.deltaY < 0 ? 1.25 : 0.8;
      zoomState.scale = Math.min(8, Math.max(1, zoomState.scale * delta));
      apply();
    }, { passive: false });
    stage.addEventListener("mousedown", (e) => {
      if (e.button !== 0) return;
      // 点空白区关闭；点图片拖拽
      const onImg = !!e.target.closest(".pv-zoom-cell img");
      if (!onImg) { closeZoom(); return; }
      zoomState.drag = { x: e.clientX - zoomState.tx, y: e.clientY - zoomState.ty };
      layer.classList.add("is-dragging");
      e.preventDefault();
    });
    window.addEventListener("mousemove", (e) => {
      const d = zoomState.drag;
      if (!d) return;
      zoomState.tx = e.clientX - d.x;
      zoomState.ty = e.clientY - d.y;
      apply();
    });
    window.addEventListener("mouseup", () => {
      zoomState.drag = null;
      layer.classList.remove("is-dragging");
    });
    document.addEventListener("keydown", (e) => {
      if (zoomState.el.hidden) return;
      if (e.key === "Escape") { e.preventDefault(); closeZoom(); }
      // 放大层内 ← / → 翻页（阅读器自身的翻章快捷键在浮层打开时已让路）
      else if (e.key === "ArrowLeft") { e.preventDefault(); zoomGo(-1); }
      else if (e.key === "ArrowRight") { e.preventDefault(); zoomGo(1); }
    });
  }

  function openZoom(img) {
    ensureZoomLayer();
    if (zoomState.img) zoomState.img.remove();
    // 记录当前页序号：供浮层内 ←→ 翻页定位
    zoomState.index = -1;
    const r = PV.reader;
    if (r && r.stage) {
      const figs = $$(".pv-page", r.stage);
      for (let i = 0; i < figs.length; i += 1) {
        if (figs[i].contains(img)) { zoomState.index = i; break; }
      }
    }
    const copy = img.cloneNode(false); // 复用已加载的高清原图，无闪烁
    // 去掉阅读区缩放态：否则 .pv-page-img.is-zoomed 的优先级会盖过浮层尺寸规则
    copy.classList.remove("is-zoomed");
    copy.style.margin = "0";
    copy.style.opacity = "0";
    copy.removeAttribute("data-src");
    zoomState.cell.appendChild(copy);
    zoomState.img = copy;
    zoomState.el.hidden = false;
    zoomState.scale = 1; zoomState.tx = 0; zoomState.ty = 0;
    updateZoomHint();
    const layer = zoomState.el;
    // 进入即适度放大（针对「极大居中 + 放大查看」的需求）
    zoomState.scale = 1.6;
    requestAnimationFrame(() => {
      // 一次性的平滑入场过渡；结束后立即移除，滚轮/拖拽零粘滞即时响应
      copy.style.transition =
        "transform 200ms cubic-bezier(0.2,0.7,0.25,1), opacity 180ms ease";
      copy.style.transform = "scale(" + zoomState.scale + ")";
      copy.style.opacity = "1";
      setTimeout(() => {
        if (copy.isConnected) copy.style.transition = "none";
      }, 240);
    });
    layer.querySelector(".pv-zoom-stage").scrollTop = 0;
  }

  /* 放大层内翻页：目标页在阅读区已加载则复用位图，否则直接取图片清单 URL；
     换页回到适配屏幕（缩放/平移状态不跨页残留） */
  function zoomGo(delta) {
    if (!zoomState.el || zoomState.el.hidden) return;
    const r = PV.reader;
    if (!r || !r.photo || !r.photo.images) return;
    const next = zoomState.index + delta;
    if (zoomState.index < 0 || next < 0 || next >= r.total) return;
    zoomState.index = next;
    const figs = $$(".pv-page", r.stage);
    const loaded = figs[next] ? figs[next].querySelector(".pv-page-img[data-state='done']") : null;
    const src = loaded ? loaded.src : r.photo.images[next].url;
    if (zoomState.img) zoomState.img.remove();
    const img = document.createElement("img");
    img.src = src;
    img.alt = "第 " + (next + 1) + " 页";
    zoomState.cell.appendChild(img);
    zoomState.img = img;
    zoomState.scale = 1; zoomState.tx = 0; zoomState.ty = 0;
    updateZoomHint();
  }

  function updateZoomHint() {
    if (!zoomState.hint) return;
    const r = PV.reader;
    const total = (r && r.total) || 0;
    const pageTxt = zoomState.index >= 0 && total
      ? "第 " + (zoomState.index + 1) + " / " + total + " 页 · "
      : "";
    zoomState.hint.textContent = pageTxt + "←→ 翻页 · 滚轮/双击 缩放 · 拖拽平移 · Esc 关闭";
  }

  function closeZoomLayer() {
    ensureZoomLayer();
    if (zoomState.el && !zoomState.el.hidden) {
      zoomState.el.hidden = true;
      if (zoomState.img) { zoomState.img.remove(); zoomState.img = null; }
      zoomState.scale = 1; zoomState.tx = 0; zoomState.ty = 0;
    }
  }

  function throttleScroll() {
    if (PV.reader.timer) return;
    PV.reader.timer = setTimeout(() => {
      PV.reader.timer = null;
      onReaderScrollBottom();
    }, 150);
  }

  function appendBatch(count) {
    const r = PV.reader;
    if (!r || !r.photo) return;
    const start = r.nextToLoad;
    const end = Math.min(start + count, r.total);
    for (let i = start; i < end; i += 1) {
      const page = r.photo.images[i];
      if (!page) { r.nextToLoad = i + 1; continue; }   // 清单比 total 短时容错跳过
      const wrap = el("figure", "pv-page");
      wrap.dataset.page = String(i + 1);   // 页码横带 IO 读取
      const img = document.createElement("img");
      // 懒加载的后批图片须继承当前缩放态，否则新页会以 100% 混入放大视图
      img.className = "pv-page-img" + ((PV.readerZoom || 1) > 1.001 ? " is-zoomed" : "");
      img.alt = "第 " + (i + 1) + " 页";
      img.dataset.src = page.url;
      img.addEventListener("error", () => onPageError(wrap, i, img));
      wrap.appendChild(img);
      r.stage.insertBefore(wrap, r.sentinel);
      if (r.pageIO) r.pageIO.observe(wrap);
      requestPageImage(img);
    }
    r.nextToLoad = end;
  }

  function requestPageImage(img) {
    if (!img.dataset.src || img.dataset.state) return;
    img.dataset.state = "loading";
    img.classList.add("is-loading");
    img.src = img.dataset.src;
    img.addEventListener("load", () => {
      img.classList.remove("is-loading");
      img.dataset.state = "done";
    }, { once: true });
  }

  function onPageError(wrap, index, img) {
    img.remove();
    wrap.classList.add("is-error");
    wrap.appendChild(el("span", "pv-page-err-title", "第 " + (index + 1) + " 页加载失败"));
    const retry = el("button", "btn btn-ghost btn-sm pv-page-retry", "重试");
    retry.type = "button";
    retry.addEventListener("click", () => {
      wrap.classList.remove("is-error");
      wrap.querySelectorAll(".pv-page-err-title, .pv-page-retry").forEach((n) => n.remove());
      const img2 = document.createElement("img");
      img2.className = "pv-page-img" + ((PV.readerZoom || 1) > 1.001 ? " is-zoomed" : "");
      img2.alt = "第 " + (index + 1) + " 页";
      img2.src = PV.reader.photo.images[index].url + "&retry=" + Date.now();
      wrap.appendChild(img2);
    });
    wrap.appendChild(retry);
  }

  /* 贴底时页码直接钉到最后一页（末页可能较短、未越过 45% 页码横带） */
  function onReaderScrollBottom() {
    const r = PV.reader;
    if (!r || !r.scroll || !r.countLabel) return;
    const sc = r.scroll;
    if (sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 8) {
      r.currentPage = r.total;
      r.countLabel.textContent = "第 " + r.total + " / " + r.total + " 页";
    }
  }

  /* ---- 独立阅读窗：主窗入口 + 阅读窗自举 ---- */

  // 主窗详情页「在新窗口阅读」：优先走 pywebview 壳开真窗口（wid=reader）。
  // 判定是否处于桌面壳：pywebview 注入 window.pywebview（读者壳窗口与主窗都有，
  // 纯浏览器 --browser 模式无）。仅纯浏览器模式（无壳）才降级为新标签页打开同 URL；
  // 壳内若 open-reader 偶发失败，只重试 + 提示，绝不误开系统浏览器。
  function openReaderWindow(album, chapter) {
    if (!album) return;
    const title = album.title || "";
    const ch = (chapter && chapter.id) ? String(chapter.id) : "";
    const inShell = typeof window.pywebview !== "undefined";
    const fallbackUrl = () =>
      location.origin + "/?w=reader&album=" + encodeURIComponent(album.id) +
      (title ? "&title=" + encodeURIComponent(title) : "") +
      (ch ? "&ch=" + encodeURIComponent(ch) : "");

    const attempt = (left) => {
      windowAction("open-reader", { album: album.id, title: title, chapter: ch })
        .then((data) => {
          if (data && data.ok === true) return; // 已交给阅读窗
          if (!inShell) { window.open(fallbackUrl(), "_blank"); return; } // 纯浏览器模式
          if (left > 0) { setTimeout(() => attempt(left - 1), 500); return; }
          window.jmToast && window.jmToast("阅读窗暂未就绪，请稍后再试", 2600, "error");
        });
    };
    attempt(2);
  }

  // 阅读窗页面加载完成即进入该模式：隐藏主窗导航，直接读 album 并打开章节。
  function bootReaderWindow() {
    if (!isReaderWin()) return;
    const q = new URLSearchParams(location.search);
    const aid = (q.get("album") || "").trim();
    document.body.classList.add("is-reader-win");

    // 窗口呼出渐显：desktop.py 在 SW_SHOW 后经 evaluate_js 调用（阅读窗
    // 「关闭=隐藏→打开=显示」无系统过渡动画，用 220ms 渐显补上；重播用
    // 强制 reflow 技巧重启 CSS animation）
    window.__readerReveal = function () {
      document.body.classList.remove("win-reveal");
      void document.body.offsetWidth;
      document.body.classList.add("win-reveal");
      setTimeout(() => document.body.classList.remove("win-reveal"), 320);
    };

    // 标题栏左侧切换为「专辑名 · 章节位置」（内容由 updateReaderTitlebar 同步）
    const rdTitle = el("span", "rd-title");
    rdTitle.id = "rd-title";
    const headLeft = $(".titlebar-left");
    if (headLeft) headLeft.appendChild(rdTitle);
    const badge = $("#net-badge");
    if (badge) badge.hidden = true;
    document.title = "JMComic · 阅读";

    const page = PV.pages.reader;
    if (!aid) {
      // 缺参数兜底：说明而非白屏
      page.innerHTML = "";
      const empty = el("div", "pv-inline-empty");
      empty.appendChild(el("p", "pv-inline-empty-title", "无法开始阅读"));
      empty.appendChild(el("p", "pv-inline-empty-text", "缺少本子编号，请从主窗口的搜索结果重新打开。"));
      page.appendChild(empty);
      showPage("reader");
      return;
    }

    (async () => {
      page.innerHTML = "";
      const loading = el("div", "pv-loading");
      loading.appendChild(el("div", "pv-loading-bar"));
      loading.appendChild(el("p", "pv-loading-text", "正在打开本子 · JM" + aid + " …"));
      page.appendChild(loading);
      showPage("reader");
      try {
        const album = await apiGet("/api/album?id=" + encodeURIComponent(aid));
        setSiteReachable(true);
        PV.state.album = album;
        PV.state.chapters = album.chapters || [];
        const chapters = album.chapters || [];
        if (!chapters.length) {
          page.innerHTML = "";
          const empty = el("div", "pv-inline-empty");
          empty.appendChild(el("p", "pv-inline-empty-title", "这本子暂时没有可读章节"));
          empty.appendChild(el("p", "pv-inline-empty-text", "可回到主窗口详情页检查章节列表，或稍后再试。"));
          page.appendChild(empty);
          return;
        }
        const chIdx = parseInt(q.get("ch") || "0", 10);
        const ch = chapters[Math.max(0, Math.min(chIdx, chapters.length - 1))];
        await openReader(album, ch);
      } catch (err) {
        setSiteReachable(false);
        page.innerHTML = "";
        const empty = el("div", "pv-inline-empty");
        empty.appendChild(el("p", "pv-inline-empty-title", "本子加载失败"));
        empty.appendChild(el("p", "pv-inline-empty-text", err.message || "未知错误"));
        const retryBtn = el("button", "btn btn-ghost pv-back", "重试");
        retryBtn.type = "button";
        retryBtn.addEventListener("click", () => bootReaderWindow());
        empty.appendChild(retryBtn);
        page.appendChild(empty);
      }
    })();
  }

  /* ---- 页内返回按钮（点击统一走 document 委托，避免重复绑定） ---- */
  function backButton(text, depth) {
    const btn = el("button", "pv-back", text);
    btn.type = "button";
    btn.dataset.back = String(depth || 1);
    return btn;
  }

  function goBack(depth) {
    if (PV.isReaderActive()) {
      // 阅读器 → 详情（无详情上下文则回首页）
      if (PV.state.album) {
        showPage("detail");
        renderAlbum(PV.state.album);
      } else {
        showPage("home");
      }
      return;
    }
    if (!PV.pages.detail.hidden) {
      // 详情 → 回结果页（若有结果）或首页
      if (PV.state.search && PV.state.search.items && PV.state.search.items.length) {
        showPage("results");
        renderResults();
      } else {
        showPage("home");
      }
      return;
    }
    showPage("home");
  }

  /* ---- 错误横幅 ---- */
  const bannerHost = $("#pv-results");
  let bannerNode = null;
  function showBanner(message) {
    clearBanner();
    bannerNode = el("div", "pv-banner");
    bannerNode.appendChild(el("span", "pv-banner-icon", "!"));
    bannerNode.appendChild(el("span", "pv-banner-text", message));
    bannerHost.insertBefore(bannerNode, bannerHost.firstChild);
  }
  function clearBanner() {
    if (bannerNode) { bannerNode.remove(); bannerNode = null; }
  }

  /* ---- fetch 封装 ---- */
  class ApiError extends Error {
    constructor(message, code) {
      super(message);
      this.code = code;
    }
  }

  async function apiGet(path) {
    let res;
    try {
      res = await fetch(path, { cache: "no-store" });
    } catch (err) {
      throw new ApiError("无法连接本地服务，请确认应用仍在运行", "offline");
    }
    let data = null;
    try {
      data = await res.json();
    } catch (err) {
      /* 非 JSON（异常） */
    }
    if (!res.ok || !data || data.ok === false) {
      throw new ApiError((data && data.message) || ("请求失败（HTTP " + res.status + "）"), (data && data.code) || "http");
    }
    return data;
  }

  /* ---- 事件绑定 ---- */
  function bindPreviewEvents() {
    const go = () => {
      const q = currentQuery();
      if (q) doSearch(q, 1, false);
    };
    PV.goBtn.addEventListener("click", go);
    PV.kwInput.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        go();
      }
    });
    PV.moreBtn.addEventListener("click", () => {
      const s = PV.state.search;
      if (s && !s.loading) doSearch(s.query, s.page + 1, true);
    });

    // 无限滚动（P2-12）：哨兵进入视口（含 500px 预取边距）自动加载下一页；
    // 「加载更多」按钮仍保留作兜底。结果页隐藏时哨兵不相交，不会误触发。
    const resultsIO = new IntersectionObserver((entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      const s = PV.state.search;
      if (!s || s.loading || s.ended || !s.items || !s.items.length) return;
      doSearch(s.query, s.page + 1, true);
    }, { root: $(".content"), rootMargin: "500px 0px" });
    const pvSentinel = $("#pv-sentinel");
    if (pvSentinel) resultsIO.observe(pvSentinel);

    // 首页空态底部：渲染搜索历史（点击回填搜索，右键可删除）
    SearchHistory.render();

    // 阅读器快捷键
    document.addEventListener("keydown", (e) => {
      if (!PV.isReaderActive()) return;
      // 放大浮层打开时：按键全部交给浮层（Esc 关闭放大等）
      if (zoomState.el && !zoomState.el.hidden) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (isReaderWin()) {
          // 独立阅读窗无上级页面：Esc 只收起目录，避免误关窗口
          if (PV.readerWinToc) {
            PV.readerWinToc = false;
            const host = PV.pages.reader;
            host.classList.remove("toc-open");
            const t = host.querySelector(".pv-toc-toggle");
            if (t) t.classList.remove("is-active");
          }
        } else {
          goBack(1);
        }
      } else if (e.key === "ArrowLeft") {
        const chs = (PV.state.readerAlbum && PV.state.readerAlbum.chapters) || [];
        const prev = chs[PV.state.readerChapterIndex - 1];
        if (prev) { e.preventDefault(); openReader(PV.state.readerAlbum, prev); }
      } else if (e.key === "ArrowRight") {
        const chs = (PV.state.readerAlbum && PV.state.readerAlbum.chapters) || [];
        const next = chs[PV.state.readerChapterIndex + 1];
        if (next) { e.preventDefault(); openReader(PV.state.readerAlbum, next); }
      }
    });
  }
  bindPreviewEvents();

  // 返回按钮全局（静态 back 元素走 data-back）
  document.addEventListener("click", (e) => {
    const btn = e.target.closest("[data-back]");
    if (btn) {
      e.preventDefault();
      goBack(Number(btn.dataset.back) || 1);
    }
  });

  /* ========================================================================
     下载任务（提交表单 → POST /api/jobs → 轮询列表 + SSE 实时日志/状态）
     ======================================================================== */

  const DL = {
    form: $("#dlv-form"),
    idsEl: $("#dlv-ids"),
    kindSeg: $("#dlv-kind"),
    kindBtns: $$("#dlv-kind .dlv-seg-item"),
    client: $("#dlv-client"),
    imageFormat: $("#dlv-image-format"),
    imageThreads: $("#dlv-image-threads"),
    photoThreads: $("#dlv-photo-threads"),
    proxy: $("#dlv-proxy"),
    submit: $("#dlv-submit"),
    hint: $("#dlv-hint"),
    listBox: $("#dlv-list"),
    meta: $("#dlv-list-meta"),
    tasks: $("#dlv-tasks"),
    refresh: $("#dlv-refresh"),
    paste: $("#dlv-paste"),
    parse: $("#dlv-parse"),
    filters: $("#dlv-filters"),
    clearDone: $("#dlv-clear-done"),
    emptyHint: $("#dlv-empty"),
    kind: "album",
    filter: "all",          // 状态筛选：all | active | done | bad
    timer: null,
    byId: new Map(),        // job_id -> {summary, open, ring[], dom{...}}
    sources: new Map(),     // job_id -> EventSource
    stats: null,            // GET /api/jobs 附带：{maxConcurrent, running, queued, total}
  };

  const KIND_LABEL = { album: "本子", photo: "章节" };
  const STATE_LABEL = {
    queued: "排队中", running: "下载中",
    done: "已完成", failed: "失败", cancelled: "已取消",
  };
  const DL_PLACEHOLDER = {
    album: "支持空格、逗号、换行分隔，例如：1455254, 148227, https://devapp.18comic.cc/comic/detail?id=1455254",
    photo: "粘贴章节 ID（可多个），例如：221233, 246810, https://18comic.vip/photo/221233/",
  };

  function dlSetKind(kind) {
    DL.kind = kind === "photo" ? "photo" : "album";
    DL.kindBtns.forEach((b) => b.classList.toggle("is-active", b.dataset.kind === DL.kind));
    DL.idsEl.placeholder = DL_PLACEHOLDER[DL.kind];
  }

  /* ---- 解析输入：URL 提取 id / 纯数字 token，去重保序 ---- */
  function dlTokenToId(tok) {
    if (/^https?:\/\//i.test(tok)) {
      const m = tok.match(/[?&]id=(\d+)/);
      return m ? m[1] : (tok.match(/(\d+)\/?$/) || [])[1] || null;
    }
    if (/^\d+$/.test(tok)) return tok;
    return null;
  }

  function dlParseIds(raw) {
    const out = [];
    const tokens = String(raw || "").split(/[\s,，、;；]+/).filter(Boolean);
    tokens.forEach((tok) => {
      const id = dlTokenToId(tok);
      if (id && out.indexOf(id) === -1) out.push(id);
    });
    return out;
  }

  /* 输入实时解析反馈（P1-5）：已识别 N 个 ID（去重前后）、无法识别的项数 */
  function dlRenderParseInfo() {
    if (!DL.parse) return;
    const tokens = String(DL.idsEl.value || "").split(/[\s,，、;；]+/).filter(Boolean);
    if (!tokens.length) { DL.parse.hidden = true; DL.parse.textContent = ""; return; }
    let ok = 0;
    let bad = 0;
    tokens.forEach((tok) => { if (dlTokenToId(tok)) ok += 1; else bad += 1; });
    const unique = dlParseIds(DL.idsEl.value).length;
    DL.parse.hidden = false;
    DL.parse.textContent = "";
    DL.parse.appendChild(el("span", null, "已识别 " + unique + " 个 ID"));
    if (ok !== unique) DL.parse.appendChild(el("span", null, "（去重前 " + ok + " 个）"));
    if (bad) DL.parse.appendChild(el("span", "is-bad", " · " + bad + " 项无法识别，将被忽略"));
  }

  /* 粘贴剪贴板（P1-5）：追加到输入框末尾 */
  async function dlPasteClipboard() {
    let text = "";
    try { text = await navigator.clipboard.readText(); }
    catch (err) { text = ""; }
    if (!text || !text.trim()) {
      jmToast("无法读取剪贴板，请手动粘贴（Ctrl+V）", 3000, "warn");
      DL.idsEl.focus();
      return;
    }
    const cur = DL.idsEl.value.trim();
    DL.idsEl.value = cur ? cur + "\n" + text.trim() : text.trim();
    dlRenderParseInfo();
    DL.idsEl.focus();
  }

  async function dlSubmit() {
    const ids = dlParseIds(DL.idsEl.value);
    if (!ids.length) {
      jmToast("请先粘贴车号 / 章节 ID 或链接", 2600, "warn");
      DL.idsEl.focus();
      return;
    }
    DL.hint.textContent = "";
    DL.submit.disabled = true;
    DL.submit.classList.add("is-loading");
    try {
      const body = {
        kind: DL.kind,
        ids,
        options: {
          client: DL.client.value,
          imageFormat: DL.imageFormat.value,
          imageThreads: Math.max(1, Number(DL.imageThreads.value) || 20),
          photoThreads: Math.max(1, Number(DL.photoThreads.value) || 4),
          proxy: DL.proxy.value.trim(),
        },
      };
      let res;
      try {
        res = await fetch("/api/jobs", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });
      } catch (err) {
        throw new ApiError("无法连接本地服务，请确认应用仍在运行", "offline");
      }
      const data = await res.json().catch(() => null);
      if (!res.ok || !data || data.ok === false) {
        throw new ApiError((data && data.message) || "提交失败（HTTP " + res.status + "）", (data && data.code) || "http");
      }
      const job = data.job;
      if (job && job.job_id) localJobIds.add(job.job_id);
      DL.idsEl.value = "";
      DL.listBox.hidden = false;
      jmToast("已提交" + (KIND_LABEL[job.kind] || "下载") + "任务：" + (job.ids || []).join("、"));
      dlRefresh(); // 列表马上包含新任务（SSE 亦由轮询触发挂接）
    } catch (err) {
      DL.hint.textContent = err.message || "提交失败";
      jmToast(err.message || "提交失败", 3400, "warn");
    } finally {
      DL.submit.disabled = false;
      DL.submit.classList.remove("is-loading");
    }
  }

  /* ---- 卡片 DOM（一次创建，之后仅更新文字/类名，保留日志面板状态） ---- */
  function dlCardDom(st) {
    const li = el("li", "dlv-task");
    // 新卡入场动画：400ms 后摘类，之后轮询的 appendChild 节点重排不会重播
    li.classList.add("is-new");
    setTimeout(() => li.classList.remove("is-new"), 400);
    const row = el("div", "dlv-task-row");
    row.title = "点击展开 / 收起日志";

    const main = el("div", "dlv-task-main");
    const title = el("div", "dlv-task-title");
    const badge = el("span", "dlv-badge");
    badge.appendChild(el("span", "dlv-dot"));
    const stateEl = el("span", "dlv-state", "…");
    badge.appendChild(stateEl);
    const idsEl = el("span", "dlv-task-ids", "…");
    title.append(badge, idsEl);
    const sub = el("div", "dlv-task-sub");
    const prog = el("div", "dlv-progress");
    prog.hidden = true;
    const progTrack = el("div", "dlv-progress-track");
    const progBar = el("div", "dlv-progress-bar");
    progTrack.appendChild(progBar);
    prog.appendChild(progTrack);
    const progText = el("span", "dlv-progress-text", "");
    prog.appendChild(progText);
    main.append(title, sub, prog);

    const actions = el("div", "dlv-task-actions");
    const logBtn = el("button", "dlv-btn-mini", "日志");
    logBtn.type = "button";
    const copyBtn = el("button", "dlv-btn-mini", "复制");
    copyBtn.type = "button";
    copyBtn.title = "复制该任务的日志到剪贴板";
    const openBtn = el("button", "dlv-btn-mini", "打开目录");
    openBtn.type = "button";
    const cancelBtn = el("button", "dlv-btn-mini is-danger", "取消");
    cancelBtn.type = "button";
    const retryBtn = el("button", "dlv-btn-mini", "重试");
    retryBtn.type = "button";
    const clearBtn = el("button", "dlv-btn-mini", "清除");
    clearBtn.type = "button";
    actions.append(logBtn, copyBtn, openBtn, cancelBtn, retryBtn, clearBtn);

    row.append(main, actions);
    const logEl = el("div", "dlv-task-log");
    logEl.appendChild(el("span", "dlv-log-empty", "暂无日志"));
    li.append(row, logEl);

    row.addEventListener("click", () => dlToggleLog(st.summary.job_id));
    copyBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      dlCopyLog(st.summary.job_id);
    });
    openBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      dlOpenOutputDir(st.summary);
    });
    cancelBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      dlCancel(st.summary.job_id);
    });
    retryBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      dlRetry(st.summary);
    });
    clearBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      dlClear(st.summary.job_id);
    });
    st.dom = { li, badge, stateEl, idsEl, sub, prog, progBar, progText, logBtn, copyBtn, openBtn, cancelBtn, retryBtn, clearBtn, logEl };
    return li;
  }

  function dlRenderCard(st) {
    const s = st.summary;
    st.dom.idsEl.textContent = (KIND_LABEL[s.kind] || s.kind) + " · " + (s.ids || []).join("、");
    st.dom.idsEl.title = (s.ids || []).map((x) => "JM" + x).join("\n");
    st.dom.sub.innerHTML = "";
    const parts = [
      (s.ids || []).length + " 个" + (KIND_LABEL[s.kind] || ""),
      s.output || "",
    ];
    if (s.returncode != null) parts.push("退出码 " + s.returncode);
    if (s.state === "queued" && s.queuePos != null) parts.push("队列第 " + s.queuePos + " 位");
    parts.forEach((txt) => st.dom.sub.appendChild(el("span", null, txt)));
    dlPaintProgress(st, s.progress);
    dlPaintState(s.job_id);
  }

  /* ---- 进度条：下载为 0~90 整体段，PDF 合并为 90~100；终态归位 ----
     修复：进度必须走 transform: scaleX（CSS 初始 scaleX(0)、过渡挂在 transform 上）。
     旧实现运行中只改 width，scaleX(0) 把条压成零宽 → 下载全程进度条不可见。 */
  function dlPaintProgress(st, p) {
    if (!st || !st.dom) return;
    const state = st.summary.state;
    if (state === "done") {
      st.dom.prog.hidden = false;
      st.dom.progBar.style.transform = "scaleX(1)";
      st.dom.progText.textContent = "完成 100%";
      return;
    }
    if (state === "failed" || state === "cancelled" || state === "queued" || !p) {
      st.dom.prog.hidden = true;
      return;
    }
    const overall = Math.max(0, Math.min(100, Number(p.overall) || 0));
    st.dom.prog.hidden = false;
    st.dom.progBar.style.transform = "scaleX(" + overall / 100 + ")";
    const phase = p.phase === "pdf" ? "PDF 合并" : "下载图片";
    let txt = phase + " " + Math.round(overall) + "%";
    if (p.done != null && p.total != null) txt += "（" + p.done + "/" + p.total + "）";
    if (p.note) txt = p.note + " · " + txt;
    st.dom.progText.textContent = txt;
  }

  function dlPaintState(id) {
    const st = DL.byId.get(id);
    if (!st || !st.dom) return;
    const state = st.summary.state;
    const active = state === "queued" || state === "running";
    const terminal = state === "done" || state === "failed" || state === "cancelled";
    st.dom.badge.className = "dlv-badge is-" + state;
    st.dom.stateEl.textContent = STATE_LABEL[state] || state;
    // 运行态流光（CSS progSweep）随状态开关
    if (st.dom.progBar) st.dom.progBar.classList.toggle("is-running", state === "running");
    st.dom.cancelBtn.hidden = !active;
    st.dom.retryBtn.hidden = !(state === "failed" || state === "cancelled");
    st.dom.clearBtn.hidden = !terminal;
    dlApplyFilter();   // 状态变化可能改变筛选分组归属
  }

  /* ---- 状态筛选（P1-4）：chips 切换 DL.filter，卡片按状态显隐 ---- */
  function dlFilterMatch(state) {
    if (DL.filter === "active") return state === "queued" || state === "running";
    if (DL.filter === "done") return state === "done";
    if (DL.filter === "bad") return state === "failed" || state === "cancelled";
    return true;
  }

  function dlApplyFilter() {
    let visible = 0;
    DL.byId.forEach((st) => {
      if (!st.dom) return;
      const show = dlFilterMatch(st.summary.state);
      st.dom.li.hidden = !show;
      if (show) visible += 1;
    });
    if (DL.emptyHint) {
      DL.emptyHint.hidden = DL.byId.size === 0 || visible > 0;
      if (DL.byId.size > 0 && visible === 0) DL.emptyHint.textContent = "当前筛选下没有任务";
    }
  }

  /* 批量清除已结束任务（P1-4）：历史记录仍保留在「下载历史」 */
  async function dlClearFinished() {
    const targets = [...DL.byId.values()].filter((st) =>
      st.summary.state === "done" || st.summary.state === "failed" || st.summary.state === "cancelled");
    if (!targets.length) { jmToast("没有可清除的已结束任务", 2200, "warn"); return; }
    const ok = await jmConfirm("清除已结束任务",
      "将从任务列表移除 " + targets.length + " 个已完成 / 失败 / 已取消的任务（历史记录保留）。",
      { danger: true, okText: "清除" });
    if (!ok) return;
    let n = 0;
    for (const st of targets) {
      try {
        const res = await fetch("/api/jobs/" + encodeURIComponent(st.summary.job_id), { method: "DELETE" });
        if (res.ok) n += 1;
      } catch (err) { /* 单个失败不打断其余清除 */ }
    }
    jmToast(n ? "已清除 " + n + " 个任务" : "清除失败，请稍后再试", 2400, n ? "ok" : "warn");
    dlRefresh();
  }

  function dlLogNode(line, isError) {
    const n = el("span", "dlv-log-line" + (isError ? " is-error" : ""), line == null ? "" : String(line));
    return n;
  }

  function dlAppendLogLine(st, line, isError) {
    st.ring = st.ring || [];
    st.ring.push({ line: line, error: !!isError });
    if (st.ring.length > 400) st.ring.splice(0, st.ring.length - 400);
    if (st.dom && st.open) {
      const empty = st.dom.logEl.querySelector(".dlv-log-empty");
      if (empty) empty.remove();
      const last = st.dom.logEl.lastElementChild;
      if (!last || last.textContent !== line) st.dom.logEl.appendChild(dlLogNode(line, isError));
      st.dom.logEl.scrollTop = st.dom.logEl.scrollHeight;
    }
  }

  function dlRenderRing(st) {
    st.dom.logEl.innerHTML = "";
    if (st.ring && st.ring.length) {
      st.ring.forEach((l) => st.dom.logEl.appendChild(dlLogNode(l.line, l.error)));
      st.dom.logEl.scrollTop = st.dom.logEl.scrollHeight;
    } else {
      st.dom.logEl.appendChild(el("span", "dlv-log-empty", "暂无日志"));
    }
  }

  async function dlToggleLog(id, force) {
    const st = DL.byId.get(id);
    if (!st || !st.dom) return;
    const willOpen = force != null ? !!force : !st.open;
    st.open = willOpen;
    st.dom.li.classList.toggle("is-open", willOpen);
    st.dom.logBtn.textContent = willOpen ? "收起" : "日志";
    if (!willOpen) return;
    dlRenderRing(st);
    // 终态任务补齐完整日志（文件侧兜底，实时任务靠 SSE/ring）
    const s = st.summary.state;
    if (s === "done" || s === "failed" || s === "cancelled") dlFetchFullLog(id);
  }

  async function dlFetchFullLog(id) {
    const st = DL.byId.get(id);
    if (!st || !st.dom || !st.open) return;
    try {
      const res = await fetch("/api/jobs/" + encodeURIComponent(id) + "/log", { cache: "no-store" });
      if (!res.ok) return;
      const text = await res.text();
      const lines = text.split(/\r?\n/).filter((l) => l.length);
      if (!lines.length) return;
      st.dom.logEl.innerHTML = "";
      lines.forEach((l) => st.dom.logEl.appendChild(dlLogNode(l, /error|失败|exception/i.test(l))));
      st.dom.logEl.scrollTop = st.dom.logEl.scrollHeight;
    } catch (err) {
      /* 日志文件读取失败不打断 UI */
    }
  }

  /* 复制任务日志（P1-1）：优先实时 ring；终态任务 ring 为空时回源完整日志文件 */
  async function dlCopyLog(id) {
    const st = DL.byId.get(id);
    if (!st) return;
    let text = (st.ring || []).map((l) => l.line).join("\n").trim();
    const s = st.summary.state;
    if (!text && (s === "done" || s === "failed" || s === "cancelled")) {
      try {
        const res = await fetch("/api/jobs/" + encodeURIComponent(id) + "/log", { cache: "no-store" });
        if (res.ok) text = (await res.text()).trim();
      } catch (err) { /* 下面统一提示 */ }
    }
    if (!text) { jmToast("暂无日志可复制", 2200, "warn"); return; }
    try {
      await navigator.clipboard.writeText(text);
    } catch (err) {
      // WebView2 剪贴板权限异常时的兜底：临时 textarea + execCommand
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      ta.style.userSelect = "text";
      document.body.appendChild(ta);
      ta.select();
      let ok = false;
      try { ok = document.execCommand("copy"); } catch (_) { ok = false; }
      ta.remove();
      if (!ok) { jmToast("复制失败：请展开日志后手动选中复制", 3200, "warn"); return; }
    }
    jmToast("已复制日志到剪贴板", 1800, "ok");
  }

  /* ---- 服务端同步：全量拉取后 diff 更新 ---- */
  async function dlRefresh() {
    let data;
    try {
      data = await apiGet("/api/jobs");
    } catch (err) {
      return; // 服务未就绪：静默，定时器下轮再试
    }
    DL.stats = data.stats || null;
    dlSync(data.jobs || []);
  }

  function dlSync(list) {
    const seen = new Set(list.map((j) => j.job_id));
    // 1) 更新 / 新建
    list.forEach((job) => {
      seen.add(job.job_id);
      const st = DL.byId.get(job.job_id);
      if (st) {
        dlApply(job, st);
      } else {
        const fresh = { summary: job, open: false, ring: [], dom: null };
        DL.byId.set(job.job_id, fresh);
        DL.tasks.appendChild(dlCardDom(fresh));
        dlApply(job, fresh);
        // 跨窗口新增任务（阅读窗等其它窗口提交；本窗未提交过）→ 轻提示。
        // 仅主窗提示（阅读窗无任务面板）；首轮同步（DL.booted=false）只建卡不提示。
        if (!isReaderWin() && DL.booted && !localJobIds.has(job.job_id)) {
          jmToast("新增下载任务：" + (KIND_LABEL[job.kind] || "下载") + " " + (job.ids || []).join("、"),
            3600, "ok");
        }
      }
    });
    // 2) 移除服务端已不存在的卡片（JobManager 重启清空）
    DL.byId.forEach((st, id) => {
      if (seen.has(id)) return;
      if (st.dom) st.dom.li.remove();
      dlCloseSource(id);
      DL.byId.delete(id);
    });
    // 3) 最新提交在最上（服务端按提交顺序返回）
    [...list].reverse().forEach((job) => {
      const st = DL.byId.get(job.job_id);
      if (st && st.dom) DL.tasks.appendChild(st.dom.li);
    });
    dlApplyFilter();
    dlRenderMeta();
    DL.booted = true; // 首轮同步完成：此后新任务才触发跨窗提示
  }

  function dlApply(job, st) {
    const prevState = st.summary.state;
    st.summary = job;
    dlRenderCard(st);
    const s = job.state;
    const terminal = s === "done" || s === "failed" || s === "cancelled";
    if (s === "running") {
      dlEnsureStream(job.job_id);   // 轮询捕获到 running 后再挂 SSE（重放历史事件，不丢日志）
    } else {
      dlCloseSource(job.job_id);
    }
    if (prevState !== s) {
      // 终态变化 → 轻提示（后台任务完成提醒）
      if (terminal && (prevState === "queued" || prevState === "running")) {
        const detail = s === "failed"
          ? (job.summary && job.summary.error ? "：" + job.summary.error : "：退出码 " + (job.returncode != null ? job.returncode : "未知"))
          : "";
        jmToast(STATE_LABEL[s] + detail, 3600, s === "failed" ? "warn" : "ok");
      }
      // 终态且日志面板正展开 → 拉完整日志
      if (terminal && st.open) dlFetchFullLog(job.job_id);
    }
  }

  /* ---- SSE：任务进行中实时订阅（服务端终态后自动关闭连接） ---- */
  function dlEnsureStream(id) {
    if (DL.sources.has(id)) return;
    const st = DL.byId.get(id);
    if (!st) return;
    let es;
    try {
      es = new EventSource("/api/jobs/" + encodeURIComponent(id) + "/events");
    } catch (err) {
      return;
    }
    DL.sources.set(id, es);
    es.addEventListener("status", (ev) => {
      let data;
      try { data = JSON.parse(ev.data); } catch (err) { return; }
      const cur = DL.byId.get(id);
      if (!cur) return;
      const s = data.state;
      if (s) {
        const prev = cur.summary.state;
        cur.summary.state = s;
        dlPaintState(id);
        dlRenderMeta();   // SSE 直推状态也要同步 meta 计数（徽章与汇总一致）
        if (s === "done" || s === "failed" || s === "cancelled") {
          dlCloseSource(id);
          if (prev !== s && cur.open) dlFetchFullLog(id);
        }
      }
    });
    es.addEventListener("log", (ev) => {
      let data;
      try { data = JSON.parse(ev.data); } catch (err) { return; }
      const cur = DL.byId.get(id);
      if (!cur) return;
      if (data && typeof data.line === "string") {
        dlAppendLogLine(cur, data.line, data.level === "error");
      }
    });
    es.addEventListener("progress", (ev) => {
      let data;
      try { data = JSON.parse(ev.data); } catch (err) { return; }
      const cur = DL.byId.get(id);
      if (!cur || !data) return;
      if (data && data.type === "progress" && data.data) {
        cur.summary.progress = data.data;   // 同步本卡进度态，dlRenderCard 也读它
        dlPaintProgress(cur, data.data);
      }
    });
    es.onerror = () => {
      // 服务端以 Connection: close 结束（终态），此处兜底关闭避免无限重连
      const cur = DL.byId.get(id);
      if (cur && (cur.summary.state === "done" || cur.summary.state === "failed" || cur.summary.state === "cancelled")) {
        dlCloseSource(id);
      }
    };
  }

  function dlCloseSource(id) {
    const es = DL.sources.get(id);
    if (es) {
      try { es.close(); } catch (err) { /* noop */ }
      DL.sources.delete(id);
    }
  }

  function dlRenderMeta() {
    const all = [...DL.byId.values()];
    DL.listBox.hidden = all.length === 0;
    if (!all.length) {
      DL.meta.textContent = "暂无任务";
      return;
    }
    // 以本地卡片状态为计数来源（SSE 翻状态后立即一致，不依赖轮询快照）；
    // 服务端 stats 仅提供并发上限（maxConcurrent）。
    const running = all.filter((x) => x.summary.state === "running").length;
    const queued = all.filter((x) => x.summary.state === "queued").length;
    const maxC = DL.stats && DL.stats.maxConcurrent != null ? DL.stats.maxConcurrent : "–";
    let text = "共 " + all.length + " 个任务 · 进行中 " + running + "/" + maxC + " 个";
    if (queued > 0) text += " · 排队 " + queued + " 个";
    DL.meta.textContent = text;
  }

  async function dlCancel(id) {
    const st = DL.byId.get(id);
    if (!st) return;
    let res;
    try {
      res = await fetch("/api/jobs/" + encodeURIComponent(id), { method: "DELETE" });
    } catch (err) {
      jmToast("取消失败：无法连接本地服务", 3200, "warn");
      return;
    }
    const data = await res.json().catch(() => null);
    if (!res.ok || !data || data.ok === false) {
      jmToast((data && data.message) || "取消失败", 3200, "warn");
      return;
    }
    jmToast("已请求取消，稍后状态会更新", 2400);
    dlRefresh();
  }

  /* 打开任务输出目录（统一走 /api/open-path；不再依赖 pywebview 桥） */
  async function dlOpenOutputDir(job) {
    const path = job.output || "";
    if (!path) {
      jmToast("该任务没有记录输出目录", 2600, "warn");
      return;
    }
    await openPath(path);
  }

  /* 失败 / 已取消 → 用原参数重新提交 */
  async function dlRetry(job) {
    const ids = job.ids || [];
    if (!ids.length) {
      jmToast("该任务缺少车号，无法重试", 2800, "warn");
      return;
    }
    const body = {
      kind: job.kind || "album",
      ids,
      options: Object.assign({}, job.options || {}),
    };
    try {
      const res = await fetch("/api/jobs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const data = await res.json().catch(() => null);
      if (!res.ok || !data || data.ok === false) {
        throw new Error((data && data.message) || "重试提交失败（HTTP " + res.status + "）");
      }
      if (data.job && data.job.job_id) localJobIds.add(data.job.job_id);
      jmToast("已重新提交任务：" + ids.join("、"), 2600, "ok");
      dlRefresh();
    } catch (err) {
      jmToast("重试失败：" + err.message, 3400, "warn");
    }
  }

  /* 终态任务从列表清除（历史记录保留） */
  async function dlClear(id) {
    const st = DL.byId.get(id);
    if (!st) return;
    let res;
    try {
      res = await fetch("/api/jobs/" + encodeURIComponent(id), { method: "DELETE" });
    } catch (err) {
      jmToast("清除失败：无法连接本地服务", 3000, "warn");
      return;
    }
    const data = await res.json().catch(() => null);
    if (!res.ok || !data || data.ok === false) {
      jmToast((data && data.message) || "清除失败", 3000, "warn");
      return;
    }
    jmToast("已清除任务记录", 2200);
    dlRefresh();
  }

  function bindDownloadEvents() {
    DL.kindSeg.addEventListener("click", (e) => {
      const item = e.target.closest(".dlv-seg-item");
      if (item && item.dataset.kind) dlSetKind(item.dataset.kind);
    });
    DL.form.addEventListener("submit", (e) => {
      e.preventDefault();
      dlSubmit();
    });
    DL.refresh.addEventListener("click", () => dlRefresh());
    // 输入实时解析反馈 + 粘贴剪贴板（P1-5）
    DL.idsEl.addEventListener("input", dlRenderParseInfo);
    if (DL.paste) DL.paste.addEventListener("click", dlPasteClipboard);
    // 状态筛选 chips（P1-4）
    if (DL.filters) {
      DL.filters.addEventListener("click", (e) => {
        const chip = e.target.closest(".dlv-chip");
        if (!chip || !chip.dataset.filter) return;
        DL.filter = chip.dataset.filter;
        $$(".dlv-chip", DL.filters).forEach((c) =>
          c.classList.toggle("is-active", c === chip));
        dlApplyFilter();
      });
    }
    if (DL.clearDone) DL.clearDone.addEventListener("click", dlClearFinished);
  }
  bindDownloadEvents();
  DL.timer = setInterval(dlRefresh, 2500); // 常驻低频轮询：后台任务完成也会更新徽标与提示

  /* ---- 下载偏好持久化（Bug3 修复：此前后端 /api/settings 就绪但前端从未调用） ----
     启动时 GET 回填表单；变更后 400ms 防抖 POST 合并写回。
     client/proxy 同时被在线预览门面读取 → 预览侧跟随同一设置。 */
  function prefsFromForm() {
    return {
      client: DL.client.value,
      imageFormat: DL.imageFormat.value,
      imageThreads: Math.min(64, Math.max(1, Number(DL.imageThreads.value) || 20)),
      photoThreads: Math.min(16, Math.max(1, Number(DL.photoThreads.value) || 4)),
      proxy: DL.proxy.value.trim(),
    };
  }

  /* 旧版 settings.json 的中文标签 → 表单值（与后端 jm_api._IMPL_ALIASES / _fmt_proxies 对齐）；
     旧版蛇形键（image_threads/format 等）一并迁移，新键（camelCase）优先 */
  const LEGACY_CLIENT = { "移动端 API（推荐）": "api", "网页端（不稳定）": "html" };
  const LEGACY_FORMAT = { "保持原格式": "original" };

  function prefsApplyToForm(p) {
    if (!p) return;
    const client = LEGACY_CLIENT[p.client] || p.client;
    if (client === "api" || client === "html") DL.client.value = client;
    const fmt = LEGACY_FORMAT[p.imageFormat] || p.imageFormat || LEGACY_FORMAT[p.format] || p.format;
    if (["original", "jpg", "png", "webp"].includes(fmt)) DL.imageFormat.value = fmt;
    const it = Number(p.imageThreads != null ? p.imageThreads : p.image_threads);
    const pt = Number(p.photoThreads != null ? p.photoThreads : p.photo_threads);
    if (it >= 1 && it <= 64) DL.imageThreads.value = String(Math.round(it));
    if (pt >= 1 && pt <= 16) DL.photoThreads.value = String(Math.round(pt));
    if (typeof p.proxy === "string") DL.proxy.value = (p.proxy === "跟随系统") ? "" : p.proxy;
  }

  let prefsDirty = false;   // 用户已动手改过 → 不再用启动回填覆盖
  let prefsTimer = 0;
  function prefsSaveSoon() {
    prefsDirty = true;
    clearTimeout(prefsTimer);
    prefsTimer = setTimeout(async () => {
      try {
        await fetch("/api/settings", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ settings: prefsFromForm() }),
        });
      } catch (err) { /* 服务未就绪：静默，下次变更再存 */ }
    }, 400);
  }

  async function prefsLoad() {
    try {
      const res = await fetch("/api/settings", { cache: "no-store" });
      const data = await res.json().catch(() => null);
      if (data && data.settings && !prefsDirty) prefsApplyToForm(data.settings);
    } catch (err) { /* 静默：保持表单默认值 */ }
  }
  prefsLoad();
  [DL.client, DL.imageFormat, DL.imageThreads, DL.photoThreads, DL.proxy]
    .forEach((f) => f.addEventListener("change", prefsSaveSoon));

  /* ========================================================================
     下载历史（GET /api/history → 真列表；一键重下 / 单条删除 / 清空）
     ======================================================================== */

  const HIST = {
    listBox: $("#hst-list"),
    empty: $("#hst-empty"),
    meta: $("#hst-meta"),
    items: $("#hst-items"),
    clearBtn: $("#hst-clear"),
  };

  function histFmtTime(ts) {
    if (!ts) return "—";
    const d = new Date(ts * 1000);
    const p = (n) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  function histOptsSummary(entry) {
    const o = entry.options || {};
    const parts = [];
    parts.push(o.client === "html" ? "网页端" : "移动端 API");
    parts.push({ original: "原格式", jpg: "jpg", png: "png", webp: "webp" }[o.imageFormat] || o.imageFormat || "原格式");
    if (o.imageThreads) parts.push("图并发 " + o.imageThreads);
    if (o.photoThreads) parts.push("章并发 " + o.photoThreads);
    if (o.proxy) parts.push("代理");
    return parts.join(" · ");
  }

  function histRowDom(entry) {
    const li = el("li", "dlv-task hst-item");
    const row = el("div", "dlv-task-row hst-row");
    const main = el("div", "dlv-task-main");
    const title = el("div", "dlv-task-title");
    const badge = el("span", "dlv-badge is-" + (entry.state || "done"));
    badge.appendChild(el("span", "dlv-dot"));
    badge.appendChild(el("span", "dlv-state", STATE_LABEL[entry.state] || entry.state));
    const idsEl = el("span", "dlv-task-ids",
      (KIND_LABEL[entry.kind] || entry.kind) + " · " + (entry.ids || []).join("、"));
    idsEl.title = (entry.ids || []).map((x) => "JM" + x).join("\n");
    title.append(badge, idsEl);
    const sub = el("div", "dlv-task-sub");
    const when = el("span", "hst-when", histFmtTime(entry.submitted_at));
    when.title = "提交于 " + histFmtTime(entry.submitted_at);
    sub.append(when, el("span", "hst-opts", histOptsSummary(entry)),
               el("span", "hst-out", entry.output || ""));
    main.append(title, sub);
    const actions = el("div", "dlv-task-actions");
    const redl = el("button", "dlv-btn-mini", "重新下载");
    redl.type = "button";
    const del = el("button", "dlv-btn-mini is-danger", "删除");
    del.type = "button";
    actions.append(redl, del);
    row.append(main, actions);
    li.appendChild(row);
    redl.addEventListener("click", (e) => { e.stopPropagation(); histRedownload(entry); });
    del.addEventListener("click", (e) => { e.stopPropagation(); histRemove(entry.job_id); });
    return li;
  }

  async function histRefresh() {
    let data;
    try {
      data = await apiGet("/api/history");
    } catch (err) {
      return; // 服务未就绪：静默
    }
    const list = data.history || [];
    HIST.items.innerHTML = "";
    list.forEach((entry) => HIST.items.appendChild(histRowDom(entry)));
    const empty = list.length === 0;
    HIST.listBox.hidden = empty;
    HIST.empty.hidden = !empty;
    HIST.meta.textContent = empty ? "" : "共 " + list.length + " 条记录";
  }

  async function histRedownload(entry) {
    const body = {
      kind: entry.kind || "album",
      ids: entry.ids || [],
      options: entry.options || {},
    };
    if (!body.ids.length) { jmToast("该记录缺少车号，无法重下", 3000, "warn"); return; }
    try {
      const res = await fetch("/api/jobs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const data = await res.json().catch(() => null);
      if (!res.ok || !data || data.ok === false) {
        throw new Error((data && data.message) || "提交失败（HTTP " + res.status + "）");
      }
      if (data.job && data.job.job_id) localJobIds.add(data.job.job_id);
      jmToast("已重新提交下载任务，正在下载", 2600, "ok");
      switchView("download");
      dlRefresh();
    } catch (err) {
      jmToast("重新下载失败：" + err.message, 3400, "warn");
    }
  }

  async function histRemove(jobId) {
    let res;
    try {
      res = await fetch("/api/history/" + encodeURIComponent(jobId), { method: "DELETE" });
    } catch (err) {
      jmToast("删除失败：无法连接本地服务", 3000, "warn");
      return;
    }
    const data = await res.json().catch(() => null);
    if (!res.ok || !data || data.ok === false) {
      jmToast((data && data.message) || "删除失败", 3000, "warn");
      return;
    }
    jmToast("已删除该条历史记录", 2200);
    histRefresh();
  }

  HIST.clearBtn.addEventListener("click", async () => {
    const ok = window.jmConfirm
      ? await window.jmConfirm("清空下载历史", "确定清空全部下载历史记录吗？该操作不可恢复。", { danger: true, okText: "清空" })
      : window.confirm("确定清空全部下载历史记录吗？该操作不可恢复。");
    if (!ok) return;
    let res;
    try {
      res = await fetch("/api/history", { method: "DELETE" });
    } catch (err) {
      jmToast("清空失败：无法连接本地服务", 3000, "warn");
      return;
    }
    const data = await res.json().catch(() => null);
    if (!res.ok || !data || data.ok === false) {
      jmToast((data && data.message) || "清空失败", 3000, "warn");
      return;
    }
    jmToast("已清空下载历史", 2200);
    histRefresh();
  });

  /* ---- 启动：独立阅读窗模式（?w=reader&album=…） ---- */
  DL.booted = false; // dlSync 首轮静默，之后才提示跨窗口新增任务
  bootReaderWindow();
})();
