# Changelog

本项目遵循语义化版本号。

## 未发布（蓝条根治 + 主窗原生入场动画）

### 窗口“蓝条”根治
- **关键权衡修正**：`DWMWA_NCRENDERING_POLICY=DWMNCRP_DISABLED` 虽能消边框，
  但会连带关闭 DWM 的最小化/最大化/还原动画（实测无动画）。现改为
  **保留非客户区渲染（DWMNCRP_ENABLED）** 以保证原生窗口动画，并改用
  `DWMWA_BORDER_COLOR = DWM_COLOR_NONE`（attr 34）把边框改为透明来消除
  “蓝条”；同时 `DWMWA_TRANSITIONS_FORCEDISABLED=FALSE`（attr 3）确认动画
  不被误禁用。从而兼顾“无蓝条”与“动画流畅”。

### 主窗原生入场动画（不再“瞬间弹出”）
- **弃用 AnimateWindow**：曾用 `AnimateWindow(AW_ACTIVATE|AW_BLEND)` 做整窗
  淡入——实测会致 WebView2 内容不重绘（内容消失）且动画不触发，已回退
  `ShowWindow`，内容消失问题随之消失（前端 boot/win-reveal 负责内容渐进）。
- **主窗改为“创建即可见”**（`hidden=False` + 创建时居中定位）：触发 Windows
  原生窗口入场动画（scale+fade），观感与普通窗口一致；创建时按屏幕尺寸居中，
  避免 loaded 后再 SetWindowPos 造成跳动。
- 阅读窗仍常驻隐藏/复用时 `SW_SHOW`（无原生动画），靠前端 `win-reveal`
  补内容淡入。

### 阅读窗底色修正（色差/光晕）
- 阅读窗页面为深底 `#171310`，但窗口背景刷/`background_color` 此前误用主窗
  暖米 `#f6f1e9` → 圆角处露出浅色光晕。已按窗口分别填充：主窗暖米、阅读窗
  深底，`_fill_window_background` 支持按窗口传色。

### 实现与验证
- 改动文件：`gui/desktop.py` / `CHANGELOG.md`
- `python -m py_compile` 通过；需实机验证：内容正常显示、主窗启动有原生
  入场动画、顶部蓝条消失。

## 未发布（窗口动画与 UI 动画全面优化 + 进度条显示修复）

### 窗口动画（更流畅）
- **Win11 保系统动画的边框处理**：`_apply_dwm` 优先用
  `DWMWA_BORDER_COLOR=DWMWA_COLOR_NONE`（attr 34）只隐藏系统边框描边——
  DWM 非客户区渲染管线保持运行，最小化/还原/最大化的系统过渡动画完整保留；
  Win10（attr 34 不支持）回退旧的 `DWMNCRP_DISABLED`。根治 1px 细边的同时
  不再削弱窗口动画。
- **阅读窗呼出渐显**：阅读窗「关闭=隐藏 → 打开=显示」没有系统过渡动画
  （瞬间弹出）；`_show_reader_window` 显示后经 `evaluate_js` 调用前端
  `window.__readerReveal()`，补一段 220ms 渐显（win-reveal）。
- **窗口底色统一**：两处 `create_window(background_color)` 从 `#f7f4ee`
  校准为页面 `--bg-app #f6f1e9`，与 class 背景刷一笔画齐——启动与尺寸
  调整期不再出现色差闪变/透明块。

### UI 动画（更丰富）
- **启动入场**：顶栏下落 → 侧栏左滑 → 内容上浮，stagger 递进（只动
  transform/opacity，合成器动画）。
- **结果网格 stagger 入场**：新搜索卡片波浪淡入（`--st` 递增 delay）；
  「加载更多」仅新增部分播放入场，旧卡 `no-anim` 不重播。
- **任务卡入场**：新建下载任务卡淡入（`is-new` 类 400ms 后移除，轮询
  重排不重播）。
- **进度条升级**：修复引用未定义 `var(--accent)` 导致渐变失效、进度条
  不可见的显示 BUG；`width` 过渡改为 `transform: scaleX()` 合成器动画
  （不再触发重排）；运行态加流光扫过效果（progSweep）。
- **滚动性能**：`.pv-reader-scroll` 移除常驻 `will-change: transform`
  （保留 `translateZ(0)` 合成层提升，省显存）。

### 实现与验证
- 改动文件：`gui/desktop.py` / `static/js/app.js` / `static/css/app.css` / `CHANGELOG.md`
- `python -m py_compile` 与 `node --check` 均通过；需打包实机验证动画观感。

## 未发布（窗口动画平滑 + 透明区填充 + 阅读器 Ctrl 滚轮缩放）

### 窗口动画流畅度（主窗 + 阅读器）
- **最小化/最大化/还原改同步切换**：`gui/server.py` 主窗、`gui/desktop.py` 阅读窗
  （`minimize_reader` / `toggle_reader_maximize`）改用 `ShowWindow(SW_MINIMIZE/SW_MAXIMIZE/
  SW_RESTORE)` 同步调用，替代原先 `PostMessage(WM_SYSCOMMAND)` 异步消息——后者在本壳下
  偶发被 GUI 队列吞掉（既无动作也无动画），同步切换可稳定触发系统标准补间动画。
- **退出全屏还原最大化**：`toggle_fullscreen` 退出分支对原为最大化的窗口用
  `ShowWindow(SW_MAXIMIZE)` 还原，动画保持一致。
- **缩放/拖动抗闪烁**：所有 `SetWindowPos` 尺寸调整追加 `SWP_NOCOPYBITS`，复用上次绘制
  缓冲，减少拖动/缩放期的残影闪烁。

### 窗口色填充（透明区块修复，主窗 + 阅读器）
- **窗口 class 背景刷**：新增 `_fill_window_background(hwnd)`，把窗口 class 背景刷设为
  与页面同底的暖米 `#f6f1e9`（`--bg-app`），frameless 圆角边角、尺寸调整期由窗口擦背景
  时显示一致色调，不再露出黑色或透明块（此前颜色与 `--bg-app` 略有偏差，改为一笔画齐）。

### 阅读器 Ctrl（Cmd）滚轮缩放
- `static/js/app.js` 新增 `bindReaderZoomWheel`：阅读区拦截 `Ctrl`（macOS `Cmd`）+滚轮，
  `preventDefault` 阻止整页缩放，按 `1.1×` 局部缩放并锚定视口中心不跳视线
  （复用 `applyReaderZoom(next, keepCenter=true)`）。
- 阅读区底栏提示新增「Ctrl+滚轮局部缩放」说明。

### UI 动画平滑
- `static/css/app.css`：`.pv-reader-scroll` 提升为独立合成层
  （`transform: translateZ(0)` + `will-change: transform`），滚动与缩放更流畅；
  顶栏/侧栏悬浮卡、content 平板、折叠动画统一 `--ease-out` 缓动，仅动 transform/opacity。

### 实现与验证
- 改动文件：`gui/desktop.py` / `gui/server.py` / `static/js/app.js` / `static/css/app.css` / `CHANGELOG.md`
- `python -m py_compile` 与 `node --check` 均通过；需打包后实机验证动画流畅度与透明区。

## 未发布（在线预览搜索历史 + 布局留白收敛）

### 预览页搜索历史

- **移除示例文本**：搜索框 `placeholder` 去掉「如 中文、全彩、MANA、1455254」示例；
  首页空态的示例标签（中文/全彩/汉化/…）整体下线。
- **空态改为搜索历史**：原来示例标签的位置改为「搜索历史」列表（`#pv-history`，
  `static/index.html` / `static/js/app.js` / `static/css/app.css`）。
- **搜索一次记录一条**：每次发起新搜索（非「加载更多」）都会把关键词计入历史，
  `localStorage["jm.searchHistory"]` 持久化，去重、最新在前、上限 50 条。
- **点击回填搜索**：左键点某条历史 → 填入搜索框并立即搜索。
- **右键可删除**：对某条历史右键（或悬停时点 ×）可删除该条记录；无历史时显示占位提示。

### 布局收敛（只有顶栏/侧栏悬浮卡片）
- 反馈上一轮「内容区也做成卡片 + 四周留白」看起来怪怪的，改为：
  **只有顶栏与侧栏保留悬浮卡片**（圆角、阴影、浅色），**内容区改为平板**——
  `background: var(--bg-app)`（与窗口同底）、去掉边框/圆角/阴影，成为一整块主体。
- `.app` 四周留白与 `.body` 间隙调整到 `10~12px`，让两块浮卡悬浮感更明显；
  因 content 与窗口同底色，四周衬底不再显现「白圈」。

### 实现与验证
- 改动文件：`static/index.html` / `static/js/app.js` / `static/css/app.css`
- `node --check static/js/app.js` 通过；无 `pv-examples / pv-chip` 残留引用。

## 未发布（下载进度条）

下载任务卡新增**实时进度条**，区分「图片下载」与「PDF 合并」两个阶段，整体进度 0~100：
图片下载占 0~90，PDF 合并占 90~100，任务完成后归位 100%。

### 实现（`jmcomic_gui.py` / `gui/worker.py` / `gui/jobs.py` / `static/js/app.js` / `static/css/app.css`）

- **后端进度回调**：新增 `emit_progress(phase, done, total, note)` 以一行 `@@PROGRESS@@`+JSON
  写入日志文件（当前 `sys.stdout`，worker 中即 `--log-file`）；整体进度按下述权重折算：
  下载阶段 `done/total×90`，PDF 阶段 `90 + done/total×10`。
- **进度下载器子类**：`_progress_downloader_class()` 继承 `JmDownloader`，在
  `before_album/before_photo` 确立图片总数（本子=总页数、章节=页数），`after_image` 每完成
  一张上报一次；通过 `download_album/download_photo` 的 `downloader` 参数注入，批量下载时
  `download_batch` 自动透传同一类到各子线程。
- **PDF 阶段进度**：`export_result_pdfs` 重构为遍历「章节计划」，按已处理章节数上报
  `pdf` 阶段进度。
- **事件通道**：`gui/worker.py` 的 tail 线程新增 `_forward_line`，识别 `@@PROGRESS@@` 前缀
  并解析为 `{"type":"progress","data":{...}}` 事件（其余仍按 log 转发）；`gui/jobs.py` 的
  `JobRecord` 新增 `progress` 字段并在 `to_summary`/历史回填中包含，供列表与重连读取。
- **前端**：任务卡新增进度条（轨 + 填充 + 文案“xx% / done/total”），`dlRenderCard` 渲染最近
  进度、SSE 新增 `progress` 事件监听实时刷新；终态 `done` 定格 100%，`failed/cancelled` 隐藏。

### 验证

- `python -B` 实测 `emit_progress` 输出符合 `@@PROGRESS@@{...}` 结构；目标 `phase/done/total/
  fraction/overall/note` 字段齐全。
- `python -m py_compile`（backend 三文件）+ `node --check`（app.js）全部通过。
- 进度条以独立 `EventSource` `progress` 事件驱动，热重连时 `_stream_job_events` 重放已有
  事件，进度可从最近一次恢复展示。

## 未发布（窗口动画原生化 + 阅读窗交互修正）

用户真机反馈两个大块诉求，本轮一次性修复并实测验证（17/17 PASS）：

### 1. 窗口动画 / 移动 / 全屏 / 边缘缩放（`gui/desktop.py` + `gui/server.py`）

- **最小化/最大化/还原 → 原生补间动画**：根因是上一轮为了「消除系统标题栏 + 自绘栏两层框」**摘掉了 `WS_CAPTION`**，但 DWM 仅对带标题栏样式的窗口播放 min/max 补间动画 & 提供 Aero Snap（StackOverflow melak47 / Office / VS / Steam 的 frameless 同款）。
  - **修复**：`_install_window_logic` 把 `WS_CAPTION` **加回**窗口样式；同时确保
    `DwmSetWindowAttribute(3=FALSE)`)`（`DWMWA_TRANSITIONS_FORCEDISABLED`）确保过渡动画未被禁用。
  - **降级保底**：新增 `_strip_caption()`（WndProc 安装超时/自自失败时摘掉
    `WS_CAPTION`）—— 宁可没有补间动画，也绝不冒「两层框」风险；`wndproc-status`
    接口可在线查询 `captionStripped` 状态。
- **拖动原生感 + Aero Snap**：实测 **WinForms 宿主吞掉 `SC_MOVE`/`SC_SIZE`**（WM_NCLBUTTONDOWN 也不会到达 WndProc——WebView2 子窗口盖满客户区），系统移动循环不可用。
  - **修复**（`gui/desktop.py`）：增强 `_manual_move_drag` 16ms 轮询自绘拖动，
    叠加 **Aero Snap 语义自绘**：
      -  顶部停留 ≥ 0.35s → `ShowWindow(SW_MAXIMIZE)` 自动最大化；带走原生补间动画
         （实测：循环工作线程用 `PostMessageW(WM_SYSCOMMAND, SC_MAXIMIZE)` 在本壳下
         **偶发不生效**（消息消息发到队列但 GUI 不处理，ret=1 但 IsZoomed=False），
 改 `ShowWindow` 后稳定触发，拖动线程跨线程同步生效）。
      -  左/右边缘释放 → 吸附工作半屏（`max(half, min_w)`，受 `WM_GETMINMAXINFO` 
         的最小宽钳制——本应用 UI min-width=1020，1920 屏半屏 960 被钳到 1020，属
         原生对带最小尺寸窗口同样行为；大屏则得标准半屏）。
    **`_NATIVE_DRAG=False`** 作为实测定案，WndProc WM_NCLBUTTONDOWN 死路径
    保留备用；保留旧 `start_manual_drag` / `WM_APP_MOVERESIZE` 兜底链。
- **边缘缩放**：同理由原生 SC_SIZE 不可用，新增 `_manual_resize_loop` 轮询
  SetWindowPos（同样问窗口自身 `WM_GETMINMAXINFO` 取得最小尺寸）。`server.py`
  reader/main 分支同步接 `start_manual_resize` 调用。
- **全屏**：自建实现 `toggle_fullscreen(wid)`，不走 pywebview `toggle_fullscreen`
  （后者会改 FormBorderStyle，与我们的 NCCALCSIZE 的 `rcWork` 修正冲突，导致
  **盖不住任务栏**）。进入：临时去 `WS_CAPTION|WS_THICKFRAME` + SetWindowPos 到
  `rcMonitor`（覆盖任务栏）；`WM_NCCALCSIZE` 全屏态跳过 rcWork 修正；
  `WM_NCHITTEST` 全屏态返回 `HTCLIENT`；退出：恢复样式 + 还原原 rect（原为最大化则
  回最大化）。`WM_CLOSE`/`WM_QUIT`/`WM_DESTROY` 不受影响。
- **新增后端端点**：`/api/window/toggle-fullscreen`、`/api/window/wndproc-status`；
  `/api/window/is-maximized` 响应增加 `fullscreen` 字段供前端同步图标。

### 2. 阅读窗交互修正（`static/index.html`/`static/js/app.js`/`static/css/app.css`）

- **底部提示文案修正**：`app.js` 从「点击图片放大 · 滚轮缩放 / 拖拽平移 / 双击复位 · Esc 关闭」
  改为「滚轮上下滑动 · 右键放大查看 · 放大后可拖动 · 底栏可缩放」—— 符合主阅读区真实行为。
- **缩放独立按钮**：主阅读区增加底栏缩放控件「− / 百分比 / ＋ / 适应宽度 / 1:1」
  （`buildReaderZoombar``），由 CSS 变量 `--reader-zoom` 驱动图片宽度（25%~400%），
  跳过首次事件中心锚点。换章保留缩放偏好。
- **点击图片开浮层 → 右键打开**：主阅读区点击图片不再打开浮层，改为右键打开
  （`contextmenu` 事件）—— 左键留给拖动拖。浮层点击空区关闭、点图片拖拽保留。
- **图片居中**：`.content.reader-mode .pv-page` 从 `width:100%` 改为 `width:auto`，
  `.`p-page-img` 加 `margin-inline:auto`，`max-width` 从 `96vw` 改相对容器，
  `pv-reader-scroll` 加 `overflow-x-auto` 与 `scroll-padding-inline:24px`。
- **拖动平移**：`bindReaderPan` 在 `pv-reader-scroll` 上监听 pointer events，
  图片超出容器（`zoom>1`）时左键拖动平移：`pointerdown/move/up` + `setPointerCapture`，
  `preventDefault` 避让 `text_select=True` 选中与原生拖图；未放大不拦截保留原生滚动；
  容器加 `is-panning` 类改光标。
- **窗口按钮贴右侧**：浮层克隆图移除 `is-zoomed` 类与 `margin:0`，避免阅读区缩放态
  影响浮层；懒加载后续批次与重试图片继承当前缩放。

### 3. 阅读窗全屏开关 + 主窗全屏快捷键（`static/js/app.js`/`static/css/app.css`）

- **主窗 F11 切换全屏**（`document.keydown` 在阅读器快捷键之前注册；`Esc` 退出时
  `stopImmediatePropagation` 防止同时触发返回/关目录逻辑）。
- **阅读窗底栏「全屏」按钮**（独立窗口仍可用 F11，同时给显式入口），全屏时
  按标签动态切换「全屏」↔「退出全屏」、背景图标加重。
- **main win-max 按钮 / 标题栏双击**在全屏态：点击 = 退出全屏；标题栏拖动/边缘缩放
  在全屏态全部禁用。
- **Esc 优先级**：放大浮层打开时 > 全屏退出 > 阅读器返回/关目录。

### 4. 附带修复（`gui/jobs.py`）

`_drain_stream` 终态事件路径增加 `self._remember_terminal(rec)`（幂等）——
让历史落盘与状态可见**同步**（先前修：`SSE 读线程先看到 state=done，但 `_read` 主线程稍后才 ``_remember_terminal` 落盘，测试与真实应用间存在「历史可见较状态晚」小小窗口，在极端情况下可能丢失历史）。该修复顺带让 flake 的
`test_terminal_persisted_and_reseeded` 稳定通过。

### 5. 验证矩阵

- **GUI 桌面实测**（工具会话内）：17/17 PASS（`build/probe_final_a.py`）
  - A1 WS_CAPTION 恢复 / WndProc installed / nativeDrag 实测 False
  - A3 全屏进出（普通态 & 最大化态）—— 全屏覆盖 1920x1080 含任务栏区
  - A2 拖动跟随光标（16ms 轮询）、顶部停留 +0.40s 自动最大化、
    左/右边缘吸附 min 钳制
  - A4 右缘缩放跟手
- **回归测试**：39/39 unittest 全绿（`test_jm_api` 17 + `test_jobs_api` 22）
- **阅读窗视觉验证**（headless Edge 截图）：图居中、缩放按钮组、新文案、
  「全屏」按钮全部如设计呈现。

### 2. 构建链修复：tkinter 可选化 + venv 重建（`jmcomic_gui.py`）

- **背景**：`gui.worker`（PyInstaller `hiddenimports` 之一）模块级 `import jmcomic_gui`，
  而 `jmcomic_gui` 顶层无条件 `import tkinter`。managed Python 3.13.12（构建解释器）
  不带 tkinter → 源码 `import gui.worker` 崩、PyInstaller 分析也绕不开 tkinter 依赖；
  此前 frozen exe 只冒烟了主进程，`--jm-worker` 下载子进程从未在无 tk 环境验证，
  该隐患一直潜伏。
- **修复**：tkinter 改为**可选导入**（`try/except ImportError`，置 `tk=None`）。
  `JMComicApp` 类体仅方法定义 + 惰性注解（`from __future__ import annotations`），
  tkinter 只在 `main()` 的 Tk GUI 分支真正实例化——该分支无 tk 时优雅报错退出
  （return 69）。下载核心（`run_worker` / `export_result_pdfs` 等）零 tk 依赖，
  worker 子进程不再被旧 Tk GUI 绑架。
- **连带修复（构建环境）**：default venv 的 site-packages 曾大面积半损坏
  （cryptography / pycryptodome / click / fastapi 等目录源码缺失、只剩空壳，
  PyInstaller hook `get_module_file_attribute` 返回 None 中断构建）。重建 venv +
  按 requirements-release.txt 重装（含 pywebview 6.2.1 / pythonnet 3.1.0 /
  clr-loader 0.3.1 官方配对——CLR 引导实测 OK，此前注释中的
  「clr-loader 0.3 移除 get_netfx」问题仅影响 pythonnet 3.0.x）。
- **验证**：frozen exe 重建后冒烟——主服务 HTTP 200；`--jm-worker` 真实下载
  链路（含 jmcomic API 域名获取/查询）跑通，无效 ID 走业务错误路径干净退出。

### 3. 详情章节预览合并为单开窗阅读（`static/js/app.js` + `gui/server.py` + `gui/desktop.py`）

- **诉求**：详情页"章节行点击预览"与工具栏"在新窗口阅读"是两个独立入口——
  前者走主窗内嵌阅读（`openReader` + `showPage('reader')`），后者才真单开 WebView2
  阅读窗。两条路径并存造成功能割裂（同一在线观看两种体验、章节行点的"预览"
  与工具栏的"在新窗口阅读"行为不一致），且内嵌路径在主窗内挤占在线预览 tab
  空间。**统一为单开窗阅读**。
- **改动**：
  - `chapterRow(album, ch)` 整行点击 → 调 `openReaderWindow(album, ch)`
    （原调内嵌 `openReader(album, ch)`）；行尾 action 标签"预览"→"在新窗口阅读"；
    同步加键盘可达（Enter/Space）。
  - 详情工具栏"在新窗口阅读"按钮**删除**（与章节行同质入口合并）。
  - `openReaderWindow(album, chapter?)` 接受可选 chapter：拼 `?w=reader&album=...&ch=...`，
    浏览器模式降级路径同步带 `ch`；`windowAction('open-reader', ...)` payload
    增加 `chapter` 字段。
  - 后端 `desktop.open_reader(album, title, chapter='')`：URL 拼 `&ch=chapter`；
    幂等键由 `_reader_album=album` 升级为 `_reader_target=(album, chapter)`——避免
    同专辑换章节被误判为"已打开"而复用旧窗。
  - `server.py` `/api/window/open-reader` 读 `qs['chapter']`（兼容 `qs['ch']`）→ 透传。
- **体验**：现在章节行就是"点击该话 → 在独立阅读窗打开该话"；主窗在线预览
  tab 不再有内嵌阅读面板（panel 结构与 CSS 仍保留以服务独立窗）。独立窗内
  抽屉翻章、上一话/下一话、缩放、拖动、全屏等所有 d67bac4 能力**完全保留**。
- **验证**：node 语法 + py import + unittest 47/47 全绿；frozen 冒烟
  `/api/window/open-reader` 端点解析正确（缺参 → `missing album`；带 chapter
  → 后端接收成功，因 browser 模式无 reader 窗返回 `reader window not ready`
  是预期——真机双窗下 reader 窗由 pywebview 壳创建，端到端弹窗验证留真机）。

### 4. 恢复 frameless：去掉 d67bac4 加回的 WS_CAPTION（`gui/desktop.py`）

- **根因**：d67bac4 为了拿原生 min/max 补间动画 + Aero Snap 在 `_install_window_logic`
  主动 OR 上 `WS_CAPTION` 位——但**该位本身就会让系统绘制标准标题栏**（"JMComic
  Downloader GUI"蓝条 + min/max/close 按钮）。`WM_NCCALCSIZE return 0` 仅把
  客户区撑到全窗，**不会**阻止 WS_CAPTION 触发的标准栏绘制——真机视觉上"系统栏
  + 自绘栏"两层框始终在。
- **用户反馈**：「还是有顶部横幅」（红框标注）—— 接受失去原生补间动画的代价，
  优先视觉干净的单层自绘栏。
- **改动**（3 处）：
  - `_install_window_logic`：style 仍 OR `WS_THICKFRAME | WS_MINIMIZEBOX |
    WS_MAXIMIZEBOX | WS_SYSMENU`（保留 thickframe 供边缘 resize 手柄视觉、
    min/max/sysmenu 供自绘按钮 / Alt+Space 菜单），**去掉** `WS_CAPTION`。
  - `_frame_style`：fullscreen=True 摘 `WS_THICKFRAME`；fullscreen=False 恢复
    `WS_THICKFRAME | MIN/MAX/SYSMENU`（同样去 caption）—— 全屏态逻辑简化（基
    样式本就没 caption）。
  - `_strip_caption` 改文档说明现为 no-op（主路径 frameless 永远不命中），保留
    函数以备未来回退加 caption 时可立即清理。
  - 文件头 docstring 与 WndProc 路径注释同步更新。
- **保留不动**：拖动 / Aero Snap 全部走 d67bac4 的自绘版（`_NATIVE_DRAG=False`
  + `_manual_move_drag` 轮询 + 顶部停留 0.35s 触发 ShowWindow(SW_MAXIMIZE)
  + 左/右半屏吸附），不依赖 WS_CAPTION；
  边缘 resize 走 `WM_NCHITTEST` + 自绘轮询；
  全屏由前端 F11 / 底栏按钮触发，行为与 d67bac4 一致（仅样式位改了）；
  min/max 按钮调 `PostMessage(WM_SYSCOMMAND, SC_MAXIMIZE/MINIMIZE)`，
  frameless 仍可工作（sysmenu / 标题栏不存在不影响 SC_ 命令）。
- **验证**：py import + unittest 47/47 + frozen exe 启动 HTTP 200 全部 ✓；
  视觉确认真机无系统栏留真机。

### 5. 顶部 1px 细框根治 + 悬浮卡片化布局

- **顶部 1px 蓝/白细边线根治（`gui/desktop.py`）**：上轮摘掉 `WS_CAPTION`/`WS_BORDER`
  后，窗口为保留边缘缩放手柄仍带 `WS_THICKFRAME`；Win11 的 DWM 会对这类窗口绘制
  非客户区（accent 色细框 + 圆角），真机视觉上就是顶部那条 1px 蓝/白线。
  - **修复**：新增 `_apply_dwm(hwnd, rounded=…)`，置
    `DWMWA_NCRENDERING_POLICY=DWMNCRP_DISABLED`（DWM 不再绘制任何非客户区）+ 显式
    `DWMWA_WINDOW_CORNER_PREFERENCE` 声明圆角/直角；在 `_install_window_logic`、
    `_frame_style`、`_guard_frameless` 三处统一应用（全屏态传 `rounded=False` 还原直角）。
  - 纯增量：不改变边框样式位，不影响拖动/边缘缩放/全屏既有逻辑；dwmapi 缺失或旧版
    系统不支持时静默降级，不抛异常。
- **悬浮卡片化布局（`static/css/app.css`）**：让左侧边栏与内容区也获得和顶部标题栏
  一致的「自己的区域」——四周留白、圆角、柔和阴影。
  - `.app` 统一 `padding:7px 9px 9px` + `gap:11px`，三块区域作为嵌套卡片悬浮其中；
    `.titlebar` 去掉外 margin、`.body` 增加 `gap:10px`。
  - `.sidebar` 从「贴边通高长条」改为圆角卡片（`border-left` 换为四边柔边 + `radius` +
    `shadow-card`）；`.content` 同步补 `bg-card` + 四边柔边 + 圆角 + 阴影。
  - 全屏态（`body.is-fullscreen`）回归 edge-to-edge：去留白、直角，保持一致沉浸。
  - DOM 边缘缩小/拖动兜底逻辑不受影响（监听在 `window`/`document` 捕获层，事件仍能
    命中留白区）；阅读窗（`is-reader-win`）隐藏侧栏后内容卡自然横贯。

## 未发布（真机问题修复：搜索 NOMODEL + 系统标题栏回归）

真机双击 `dist/gui_pw` exe 验证暴露两个问题，均已在本次修复并重建：

- **搜索报 `No module named ...`（NOMODEL）** —— 根因是 **exe 打包缺 jmcomic**：
  构建环境 sys.path 不含 `src/`（未 `pip install -e .`），spec 中
  `collect_all('jmcomic')` 静默空转（PyInstaller 对找不到的包不报错），PYZ 内
  jmcomic 模块数为 0；而 `gui/jm_api.py` 对 jmcomic 是**延迟 import**（首次搜索/
  下载才 `import jmcomic`），故启动冒烟正常、一搜索即炸。
  **修复**：`build/gui_pw/spec/JMComic-Downloader-GUI.spec` 顶部把 `src/` 挂进
  `sys.path` 且 Analysis `pathex=[src]`（双保险），重建后 exe 内 jmcomic 17 个
  模块齐备（exe 24.9MB → 41.8MB）；`build_gui_exe.ps1` 同步给 main/worker 参数
  增加 `--paths src`（防 `pip install -e .` 步骤意外失败时 collect 静默漏收）。
- **系统标题栏 + 自绘标题栏两层框** —— 根因是 frameless 成了"单保险"：
  `_install_window_logic` 主动把 `WS_CAPTION`(0x00C00000) OR 回窗口样式，隐藏
  完全依赖 `WM_NCCALCSIZE` 子类化；一旦 WndProc 失效/未装，系统栏立即回归。
  **修复**（`gui/desktop.py`）：①窗口样式**永不设置 WS_CAPTION|WS_BORDER|
  WS_DLGFRAME**（仅保留 THICKFRAME/MIN/MAX/SYSMENU 提供系统 resize/最大化/
  Alt+Space 能力）——即使 WndProc 完全失效，系统标题栏也不可能出现；
  ②SetWindowLongPtrW 装 proc 检查返回值，失败不置 installed（允许轮询重试）
  并落日志；③WndProc 回调异常过频还原原 proc 后清 installed 标记并**自愈重装**；
  ④新增 `logs/desktop.log`（WndProc 安装成功/失败/自愈留痕）。
- **真机验证记录（工具会话内冒烟）**：重建 exe 双窗句柄正常、两窗样式
  `WS_CAPTION=False / THICKFRAME+MAX+MIN+SYSMENU=True`（系统框不可能出现）；
  `/api/window/*` 控制链 OK；`/api/search?q=ONE+PIECE` 返回 total=442、80 条
  （NOMODEL 消除）；源码模式 desktop.log 记录两窗 WndProc 安装成功。
  **待真机复核**：桌面双击 exe 后搜索、拖动、双击最大化、阅读窗打开是否全部正常；
  已知问题「阅读窗非显示态 ~30s WebView2 进程组回收」仍在架构决策中（见下节）。

## 未发布（回退 pywebview(WebView2) 双窗口壳 / WebView2 隔离）

桌面壳从 PySide6/QtWebEngine **整体回退 pywebview(WebView2)**（实测窗口效果不佳）；
独立阅读窗、UI 精修、单 exe 内嵌 worker 等既有能力全部保留（历史见下方各节）。

- **双窗口保留（主窗 + 独立阅读窗）**：pywebview 6 运行期禁止新建窗口，故在
  `webview.start()` 前**预创建**两个 `hidden=True` 窗口；阅读窗常驻隐藏，打开即
  show + 导航，`WM_CLOSE` 拦截为 `SW_HIDE`（关闭=隐藏复用，重开同一专辑保留翻页进度）。
- **无边框自绘标题栏（ctypes WndProc 子类化，每窗口独立安装）**：
  - `WM_NCCALCSIZE` 隐藏系统标题栏；`WM_NCHITTEST` 区分标题栏(HTCAPTION)/按钮区
    (HTCLIENT)/8px 边缘缩放(HTxxx)；`WM_GETMINMAXINFO` 限定最小尺寸；
    `WM_NCLBUTTONDOWN`/`DBLCLK` 接管拖动与双击最大化；`WM_APP_MOVERESIZE` 承载
    前端拖动兜底（后台轮询手动拖动）；
  - 主窗关闭 → 统一退出（停 worker → 停服务 → 退出，1.5s 兜底）；阅读窗关闭 → 隐藏。
- **WebView2 数据隔离**：`webview.start(storage_path=exe同目录/.webview2-cache)`
  即 WebView2 UserDataFolder——与系统其它 WebView2 实例（Edge 浏览器等）完全隔离，
  不共用 `%APPDATA%\pywebview`，互不影响。
- **窗口控制改 ctypes 直控 HWND**：删除 PySide6 的 WindowController 注册表与
  GUI 线程 marshal；server.py 新增 `_main_hwnd/_reader_hwnd`（窗口出现后注册），
  `/api/window/*` 按 `wid`(main/reader) 路由，经 PostMessage/ShowWindow/
  SetWindowPos 跨线程直控；前端 `readerWindowActive()` 自动补 `wid=reader`，
  阅读窗内按钮不再打到主窗。
- **依赖/打包回退**：`requirements.txt` 以 `pywebview>=6.2` 替换 `PySide6`，并钉死
  `clr-loader<0.3`（**pythonnet 3.1.0 调 `clr_loader.get_netfx()`，clr-loader 0.3.x 移除
  该 API，会导致 CLR 双重引导失败、pywebview 完全起不来**；实测 0.2.7.post0 + .NET
  Framework 4.8 正常）；`requirements-release.txt` 锁定 pywebview 6.2.1 +
  pythonnet 3.1.0 + clr-loader 0.2.7.post0；`build_gui_exe.ps1` 改
  `--collect-all webview/clr_loader/pythonnet`；单 exe 内嵌 worker（`--jm-worker`
  自我拉起）与 `-IncludeWorker` 双 exe 开关保持不变。
- 验证（冻结 exe，升级权限实测）：py_compile 100%、unittest 39/39、模块导入冒烟
  通过；`.webview2-cache/EBWebView` 落 exe 同目录（与系统其它 WebView2 隔离）；
  双窗句柄/窗口对象常驻、`/api/window/*` 窗口控制链与阅读窗打开显示正常。
  **已知问题（真机验证中）**：独立阅读窗处于非显示态（关闭隐藏/最小化/尚未显示）
  约 30 秒后，其 WebView2 进程组被回收、控件进入不可恢复的 crashed 状态，再次
  打开阅读窗报 `CoreWebView2 is no longer valid`（诊断：GPU/Utility 进程被外部
  Terminated；`--disable-backgrounding-occluded-windows`/`--disable-gpu` 等参数
  均无效；pywebview 无法重建控件）。桌面真实环境行为待用户双击 exe 复测确认。

## 未发布（独立阅读窗 / UI 精修 / 单 exe 分发）

- **单 exe 分发（内嵌 worker）**：
  - `run_gui.py` 支持首参 `--jm-worker` 分派：在加载 Qt/HTTP 服务前拦截并转交
    `gui.worker.main()`；`gui/jobs.py` 打包模式改为优先用同目录
    `JMComic-Downloader-Worker.exe`（双 exe 布局，兼容旧产物），找不到时以
    `本exe --jm-worker <worker参数>` 自我拉起（单 exe 布局）；
  - `build_gui_exe.ps1` 默认只产出一个主程序 exe（`dist\gui\JMComic-Downloader-GUI.exe`，
    `--hidden-import gui.worker` 确保 worker 依赖随包）；可选 `-IncludeWorker`
    开关恢复双 exe 布局（下载子进程独立 console，启动更快、内存占用更小）；
  - 单 exe 已实测：windowed onefile 自我拉起时 stdout 管道事件通道可用
    （spike 4/4 事件收到），下载任务无需同目录 Worker.exe。
- **独立阅读窗口**（详情页「在新窗口阅读」入口；壳层实现已换代，见顶部「回退
  pywebview」节——由 PySide6 `QMainWindow` 重做为 pywebview 预创建第二窗）：
  - 真双窗：阅读沉浸、画布更大，关闭不退出应用；后端按 `wid` 路由
    （`main`/`reader`）；无壳/壳失败时浏览器模式降级为新标签页同 URL；
  - 章节信息同步：标题栏左侧显示「`JM{id} · 专辑名 · 第 N 话`」，阅读区顶部
    突出当前章节；
  - 章节抽屉（默认收起 → 点击「目录」按钮左侧滑入）：专辑名 + 章节徽章 +
    当前章高亮；切换章节时抽屉保持展开，支持连续翻阅；
  - 阅读窗下载入口：顶部条新增「下载本子」「下载本话」「上一话/下一话」，
    主窗下载任务面板自动出现（共享 `/api/jobs` 队列）。
- **跨窗任务轻提示**：阅读窗 / 主窗提交的任务共享同一队列，本窗未提交过的
  新任务出现时只在主窗（`is-reader-win=false`）弹 toast；首轮同步静默，避免启
  动即刷屏历史任务。
- **图片放大查看（自绘浮层）**：阅读页点击已加载图片进入深色全屏放大层，
  默认 1.6 倍入场；滚轮缩放 1–8 倍，鼠标拖拽平移，双击复位/Esc 关闭，右上
  圆形 ×。入场过渡结束后移除内联 transition，滚轮/拖拽零粘滞即时响应。
- **UI 视觉与动效精修（暖橙奶油精修版）**：
  - 统一缓动 `cubic-bezier(.2,.7,.25,1)`，过渡时长 140–260ms；只动
    `transform`/`opacity` 保合成层流畅；尊重 `prefers-reduced-motion`；
  - 去除全应用 0.5px 发丝边框（标题栏、侧栏、卡片、列表、按钮）→ 1px 低透明
    柔边 / 阴影 / 背景过渡；
  - 标题栏升级为**悬浮圆角毛玻璃卡**（`margin 7 9 0 / border-radius 12 /
    backdrop-filter: blur(10px)`），所有按钮 hover 上浮、active `scale(.96)`；
  - 全局 `:focus-visible` 橙色柔光环（替代 0.5px 发虚边框）；
  - 视图切换淡入 + 8px 上移；阅读卡片 hover 上浮 `-3px`；btn-primary 渐变橙
    + hover 上浮 + 加深阴影；分类型（`.dlv-seg-item.is-active`）凸起阴影；
  - 骨架 shimmer / toast 滑入 / 自绘确认框淡入+scale / 章节抽屉滑入 / 放大
    层 fade-in + scale-pop / 任务日志展开展开；
  - 深色阅读画布加暖橙 radial 微光（`#191512` 起步）；图片 `.pv-page-img`
    `max-width: min(96vw, 1240px)` 极大居中；占位 shimmer；`.pv-zoom-cell
    img` 覆盖以适配放大容器。
- **错误兜底差异化**：主窗阅读失败 → 「返回详情」；阅读窗阅读失败 → 「重试」
    （无主窗上下文）；阅读窗缺 `album` 参数 → 「无法开始阅读 / 缺少本子编号」。
- 验证：源码语法 100% 通过；无头浏览器 mock 数据 9/9 主链路通过（搜索→详情
  → 内嵌阅读 / 阅读窗加载 → 放大层 1.6× → 滚轮 2× → Esc 关闭 → 目录切换第 2 话
  → 标题栏同步 / 抽屉保持展开）；桌面壳目检关键界面截图 6/6 通过。

## 未发布（Qt 桌面壳迁移 / 四问题修复）

- **桌面壳迁移 PySide6 + QtWebEngine**：自带 Chromium 渲染内核，不再依赖系统
  WebView2 Runtime 与 pywebview（WinForms）宿主；解决旧壳「右上角点 X 无反应、
  窗口无法拖动」的架构根因——HTML 标题栏鼠标事件正常进入 DOM，无 WM_NCHITTEST
  吞消息问题。
  - 拖动：标题栏 mousedown → Qt `startSystemMove()`（系统移动循环）；
  - 边缘缩放：前端 8px 边缘检测（缩放光标）→ Qt `startSystemResize()`；
  - 右上角最小化/最大化/关闭与标题栏双击 → Qt 原生窗口方法；
  - 无控制器/`--browser` 模式：`/api/window/*` 返回 `window not ready`，前端
    400ms 自动补发（冷启动不丢操作）；
  - 退出统一走 `closeEvent`：先 `JOBS.shutdown_all()` 停 worker → 停本地服务 →
    退出；1500ms 兜底强制结束，修复 frozen 场景 `_MEI` 临时目录残留告警。
- 依赖/打包：`requirements*.txt` 以 `PySide6` 替换 `pywebview`；
  `build_gui_exe.ps1` 改 `--collect-all PySide6 + shiboken6`（Qt 插件/translations/
  resources/`QtWebEngineProcess.exe` 全部随包），产物仍为 `dist\gui\` 双 EXE。
- 环境变量：`JM_WEBENGINE_FLAGS` 可追加 Chromium 启动参数（无 GPU/虚拟化会话可用
  `--no-sandbox --disable-gpu`）；`JM_QTWEBENGINE_DEBUG_PORT` 开启远程调试端口。
- 「打开输出目录」桥接：旧 pywebview 壳经 `window.pywebview.api.open_path` 调用，
  Qt 壳无该桥；统一改走本地 HTTP `/api/open-path`（`os.startfile`，浏览器调试
  模式同样可用），前端设置页与任务行入口同步更新，不再依赖 pywebview 全局对象。
- 沿用上一版标题栏修复：最大化/还原双图标随窗口状态同步（`body.is-maximized`）、
  `request_close` 语义改为 Qt closeEvent 统一退出。
- 验证：unittest 47/47、生产 Qt 壳自动化 9/9（真实页面 + 控制器 + 拖动/缩放/
  三按钮/状态轮询/干净退出）、真实入口 `run_gui.py` 冒烟通过；真人鼠标拖动与
  边缘缩放需实机复测。

## 未发布（新版桌面界面 / 在线预览）

- 新增 `run_gui.py` 桌面入口：本地 HTTP 服务 + pywebview（WebView2）无边框窗口，自绘标题栏与橙色主题前端（对齐桌面「小说管家」风格）。
- 新增在线预览：关键词文字搜索 → 本子/章节详情 → 章节内看图，全程只读不写入下载目录。
- 下载任务视图：输入车号/章节 ID 批量提交，实时进度与日志（SSE + 轮询双通道），支持取消、失败/取消后重试、清除记录。
- 下载历史：终态任务持久化到系统数据目录（`LOCALAPPDATA\JMComic-Downloader-GUI\jobs_history.json`，可用 `JM_HISTORY_PATH` 覆盖），重启后仍展示；支持一键重新下载、单条删除、清空（自绘确认框）。
- 并发队列可视化：任务列表汇总显示进行中/排队数、并发上限，排队任务标注队列位次。
- 构建脚本 `build_gui_exe.ps1`：打包新版界面为双 EXE（主程序 + 下载 Worker 子进程）。

## 1.2.0 - 2026-08-04

- 建立独立项目名称 `JMComic-Downloader-GUI`。
- 提供 Windows Tkinter 图形界面和批量输入。
- 下载后按章节生成 PDF。
- PDF 转换不缩小图片、不主动降低 JPEG 质量。
- PDF 页数校验成功后自动删除对应中间图片。
- PDF 失败时保留原图片。
- 提供 Edge/Chrome 标签页复制扩展 1.0.2。
- 增加代理、并发和图片格式设置。
- 默认下载目录跟随当前 EXE 所在文件夹，并迁移旧版默认路径设置。
- 在说明文档中增加 `gpt5.6-sol` 开发辅助工具声明。
- 增加 GUI/PDF 离线测试和浏览器扩展测试。
- 保留上游 MIT License 和原作者版权声明。
- 移除上游 PyPI 发布、云端内容下载和收藏夹导出工作流。
