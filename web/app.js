// RemoteControl 前端
// M1：WebSocket 连接 + 密码认证。
// M2：在认证后采集鼠标/键盘事件，归一化坐标后发送到服务端注入到被控端。
// M3：WebCodecs 解码视频流，渲染到 canvas。

const statusEl = document.getElementById('status');
const hintEl = document.getElementById('hint');
const formEl = document.getElementById('auth-form');
const passwordEl = document.getElementById('password');
const controlPanelEl = document.getElementById('control-panel');
const surfaceEl = document.getElementById('control-surface');
const controlHintEl = document.getElementById('control-hint');
const canvasEl = document.getElementById('video-canvas');
const topDrawerEl = document.getElementById('top-drawer');
const btnBackLoginEl = document.getElementById('btn-back-login');
const btnToggleFullscreenEl = document.getElementById('btn-toggle-fullscreen');
const btnSendAltF4El = document.getElementById('btn-send-alt-f4');
const btnSendCtrlAltDelEl = document.getElementById('btn-send-ctrl-alt-del');
const btnSendClipboardEl = document.getElementById('btn-send-clipboard');
const btnReadClipboardEl = document.getElementById('btn-read-clipboard');
const btnAutoClipboardEl = document.getElementById('btn-auto-clipboard');
const clipToastEl = document.getElementById('clip-toast');

let ws = null;
let helloReceived = false;
let inputBound = false;
// 用户主动断开（如点击"返回登录"）时置位，让 onclose 跳过"连接已断开"提示。
let manualClose = false;

// M3: 视频解码器
let videoDecoder = null;
let canvasCtx = null;
let screenWidth = 1920;
let screenHeight = 1080;

// 服务端按 NAL 单元逐个下发：SPS / PPS / IDR 各是一个独立包。
// WebCodecs 要求关键帧 chunk 内联 SPS+PPS，且 configure 后首个 chunk 必须是 key。
// 这里缓存最近的参数集，遇到 IDR 时拼到前面组成完整的关键帧。
let cachedSps = null;
let cachedPps = null;
let keyFrameDecoded = false; // 在收到首个关键帧前，丢弃所有 P 帧

// 鼠标移动节流：~60Hz（≈16ms）。最后一帧未发送时记下，下一窗口补发。
const MOVE_THROTTLE_MS = 16;
let lastMoveSentAt = 0;
let pendingMove = null;

// 已发出 key_down、尚未发出 key_up 的 KeyboardEvent.code 集合。
//
// 浏览器会永久丢掉某些 keyup：最典型的是按住 Alt 再按 Tab —— Alt+Tab 被操作系统
// 当系统快捷键吞掉，页面只收到 blur，Alt 的 keyup 永远不来。被控端于是残留一个
// 按下的 Alt，之后每个普通键都变成 Alt+X 被菜单吃掉，用户看到的就是
// 「键盘被锁定，要在本地敲一下 Alt 才能继续输入」。
// 记账后可在失焦 / 页面隐藏 / 断连前补发释放。
const pressedKeys = new Set();
// 已发出 mouse_down、尚未发出 mouse_up 的按键名。
// 同理会丢：在画面上按住左键拖拽，拖出浏览器窗口再松手，mouse_up 落不到页面上，
// 被控端就留着一个按下的左键（表现为「鼠标一直在框选/拖拽」）。
const pressedButtons = new Set();

// ========== 自动剪切板同步状态 ==========
//
// 方向由用户的按键手势决定，不猜测光标位置：
//   画面内 Ctrl+C/X → 透传按键，再轮询被控端剪切板，拉回本地（远端 → 本地）
//   画面内 Ctrl+V   → 先把本地剪切板推给被控端，再补发 Ctrl+V（本地 → 远端）
// 只在这两个明确手势下触发，不做失焦/划过画面的隐式同步：那既受浏览器手势限制，
// 又会让用户在毫无感知的情况下把剪切板（可能含密码）发到被控端。
let autoClipboard = true;

// 回环去重：记下最近一次"两端已一致"的文本。
// 推上去的内容再被拉回来时与它相同，就不再写回本地，断开 A→B→A 的循环。
let lastSyncedText = null;
// 服务端为 lastSyncedText 算的指纹。随 clipboard_get 带上去，
// 内容没变时服务端只回 clipboard_unchanged，不重传正文 —— 公网带宽很窄
// （用户的服务器只有 3 Mbps），轮询重试重复下载同一份文本会挤掉视频流。
let lastSyncedFingerprint = '';

// Ctrl+V 待处理标记：keydown 里置位，paste 事件里消费。
let pendingPaste = null;
// paste 事件迟迟不来时的兜底定时器。
let pasteFallbackTimer = null;
// Ctrl+V 的 keydown 被延后注入，对应 keyup 要跳过，靠这个标记。
let pasteKeyIntercepted = false;
// 远端剪切板轮询定时器，用于取消上一轮未完成的拉取。
let pullTimer = null;
// 本轮还剩几次重试。
let pullRetriesLeft = 0;
// 右键提示每次连接只弹一次，避免每右键一下都弹。
let rightClickHintShown = false;
// 在途的 clipboard_get 是自动轮询发的（true）还是手动按钮发的（false）。
// 服务端对两者回同一个 clipboard_data，靠这个标记分流，不新增协议消息。
let autoPullPending = false;

// 复制动作在被控端生效需要时间，首次拉取前先等一会儿。
const PULL_FIRST_DELAY_MS = 120;
// 拉到的还是旧内容时的重试间隔与次数。
// 总耗时 120 + 4*180 = 840ms，远小于 transient activation 的 ~5s 窗口，
// 保证最后的 writeText() 仍在 Ctrl+C 这次手势的授权范围内。
const PULL_RETRY_DELAY_MS = 180;
const PULL_MAX_RETRIES = 4;
// paste 事件兜底等待。
const PASTE_EVENT_TIMEOUT_MS = 300;
// 与服务端 ClipboardConfig.max_text_bytes 默认值一致，超限就不白跑一趟 WebSocket。
const CLIP_MAX_BYTES = 1024 * 1024;

// 设置状态文案与颜色（state: idle / connecting / ok / fail）。
function setStatus(text, state) {
    statusEl.textContent = text;
    statusEl.dataset.state = state || 'idle';
}

function wsUrl() {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    return `${scheme}://${location.host}/ws`;
}

function connect() {
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
        return;
    }
    helloReceived = false;
    setStatus('连接中...', 'connecting');
    hintEl.textContent = '正在建立 WebSocket 连接。';

    try {
        ws = new WebSocket(wsUrl());
    } catch (err) {
        setStatus('连接失败', 'fail');
        hintEl.textContent = `无法创建连接: ${err}`;
        return;
    }

    ws.onopen = () => {
        hintEl.textContent = '连接已建立，等待服务端 hello。';
    };

    ws.onmessage = (ev) => {
        // M3: 处理二进制视频帧
        if (ev.data instanceof Blob || ev.data instanceof ArrayBuffer) {
            handleVideoFrame(ev.data);
            return;
        }

        let msg;
        try {
            msg = JSON.parse(ev.data);
        } catch (err) {
            console.warn('无法解析服务端消息:', ev.data);
            return;
        }
        handleMessage(msg);
    };

    ws.onclose = () => {
        // 连接已断，release_all 发不出去了，只清本地记账避免重连后状态错乱；
        // 被控端的残留按键由服务端在连接结束时兜底释放（见 web.rs release_all）。
        pressedKeys.clear();
        pressedButtons.clear();
        if (manualClose) {
            // 用户主动断开：状态已由 backToLogin() 设置，这里不再覆盖。
            manualClose = false;
            return;
        }
        setStatus('连接已断开', 'fail');
        hintEl.textContent = '连接已关闭，可重新输入密码连接。';
        controlPanelEl.hidden = true;
        exitControlMode();
    };

    ws.onerror = () => {
        setStatus('连接错误', 'fail');
        hintEl.textContent = '连接发生错误（检查服务是否运行、证书是否信任）。';
    };
}

function handleMessage(msg) {
    switch (msg.type) {
        case 'hello':
            helloReceived = true;
            setStatus('已连接，发送认证', 'connecting');
            hintEl.textContent = `服务端协议 v${msg.version}，编码 ${msg.video_codec}。`;
            sendAuth();
            break;
        case 'screen_info':
            // M3: 收到屏幕信息，初始化解码器
            screenWidth = msg.width;
            screenHeight = msg.height;
            canvasEl.width = screenWidth;
            canvasEl.height = screenHeight;
            // M4: 让控制区比例与被控端屏幕一致，消除 object-fit:contain 的黑边。
            // 屏幕未必是 16:9（如 1920x972），比例不符会让鼠标坐标归一化偏移。
            surfaceEl.style.aspectRatio = `${screenWidth} / ${screenHeight}`;
            initVideoDecoder();
            break;
        case 'cursor_shape':
            // 收到光标形状，更新 CSS cursor
            updateCursor(msg.width, msg.height, msg.xhot, msg.yhot, msg.rgba_base64);
            break;
        case 'auth_ok':
            setStatus('Ready to control', 'ok');
            hintEl.textContent = `会话 ID: ${msg.session_id}`;
            controlPanelEl.hidden = false;
            bindInputCapture();
            enterControlMode();
            // 新会话重置剪切板同步状态：换了被控端还沿用旧指纹，会让服务端
            // 误判"内容未变"而不下发正文。右键提示也重新给一次。
            lastSyncedText = null;
            lastSyncedFingerprint = '';
            rightClickHintShown = false;
            cancelPendingClipboardSync();
            // 自动聚焦到输入区，方便立刻接收键盘事件。
            surfaceEl.focus();
            break;
        case 'auth_fail':
            setStatus('认证失败', 'fail');
            hintEl.textContent = `失败原因: ${msg.reason}`;
            break;
        case 'error':
            setStatus('服务端错误', 'fail');
            hintEl.textContent = `${msg.code}: ${msg.message}`;
            break;
        case 'clipboard_data':
            // 远端剪切板内容到达。自动轮询和手动按钮用的是同一个请求/响应，
            // 靠 autoPullPending 区分，避免为此新增协议消息类型。
            if (autoPullPending) {
                autoPullPending = false;
                onAutoClipboardData(msg.text || '', msg.fingerprint || '');
            } else {
                applyRemoteClipboard(msg.text || '', msg.fingerprint || '');
            }
            break;
        case 'clipboard_unchanged':
            // 被控端内容与本地已持有的一致，服务端没重传正文（省带宽）。
            // 自动轮询下继续等复制动作落地；手动按钮下直接告诉用户无变化。
            if (autoPullPending) {
                autoPullPending = false;
                retryPullOrGiveUp();
            } else {
                flashButton(btnReadClipboardEl, '内容无变化');
            }
            break;
        case 'clipboard_error':
            console.warn('剪切板操作失败:', msg.op, msg.message);
            if (msg.op === 'get') {
                if (autoPullPending) {
                    // 自动轮询失败：清掉在途标记并停止重试，否则会一直空转。
                    autoPullPending = false;
                    pullRetriesLeft = 0;
                    clearTimeout(pullTimer);
                    showClipToast(`取回失败：${msg.message}`, 'fail');
                } else {
                    flashButton(btnReadClipboardEl, '远端失败');
                }
            } else if (msg.op === 'set') {
                flashButton(btnSendClipboardEl, '远端失败');
                if (autoClipboard) {
                    showClipToast(`同步失败：${msg.message}`, 'fail');
                }
            }
            break;
        case 'pong':
            break;
        default:
            console.debug('未处理的消息类型:', msg.type);
    }
}

function sendAuth() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ type: 'auth', token: passwordEl.value }));
}

function sendInput(payload) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ type: 'input', ...payload }));
}

// M4: 请求服务端尽快下发一个关键帧（用于切回标签页等画面恢复场景）。
function requestKeyframe() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ type: 'control', action: 'request_keyframe' }));
}

// 把控制区内的鼠标坐标换算成 0~1 归一化值，按 docs/03 第 3.1 节约定。
// M4: 基于 canvas 实际显示矩形换算（而非 surface div），避免边框/留边导致偏移。
// M5: canvas 上有 object-fit:contain，元素盒子比例与画面比例不一致时（如浏览器
//     全屏，视口非 16:9），画面会被居中缩放、四周留黑边。getBoundingClientRect()
//     返回的是含黑边的元素盒子，直接除会偏移。这里按画面内在比例算出真正绘制的
//     内容矩形，扣掉黑边再归一化。
function normalizedFromMouse(ev) {
    const rect = canvasEl.getBoundingClientRect();

    // 元素盒子与画面（内在尺寸）的宽高比。
    const boxAspect = rect.width / rect.height;
    const contentAspect = screenWidth / screenHeight;

    // object-fit:contain：取能放进盒子的最大等比矩形，居中放置。
    let contentW = rect.width;
    let contentH = rect.height;
    if (boxAspect > contentAspect) {
        // 盒子比画面更宽 → 左右留黑边（pillarbox）。
        contentW = rect.height * contentAspect;
    } else if (boxAspect < contentAspect) {
        // 盒子比画面更高 → 上下留黑边（letterbox）。
        contentH = rect.width / contentAspect;
    }
    // 黑边偏移：内容在盒子内居中。
    const offsetX = (rect.width - contentW) / 2;
    const offsetY = (rect.height - contentH) / 2;

    const x = (ev.clientX - rect.left - offsetX) / contentW;
    const y = (ev.clientY - rect.top - offsetY) / contentH;
    return {
        x: Math.min(1, Math.max(0, x)),
        y: Math.min(1, Math.max(0, y)),
    };
}

function buttonName(buttonNum) {
    // MouseEvent.button: 0=left, 1=middle, 2=right.
    if (buttonNum === 0) return 'left';
    if (buttonNum === 1) return 'middle';
    if (buttonNum === 2) return 'right';
    return null;
}

function modifiersFromEvent(ev) {
    const mods = [];
    if (ev.shiftKey) mods.push('shift');
    if (ev.ctrlKey) mods.push('ctrl');
    if (ev.altKey) mods.push('alt');
    if (ev.metaKey) mods.push('meta');
    return mods;
}

function flushPendingMove() {
    if (!pendingMove) return;
    sendInput({ event: 'mouse_move', x: pendingMove.x, y: pendingMove.y });
    lastMoveSentAt = performance.now();
    pendingMove = null;
}

function onMouseMove(ev) {
    const norm = normalizedFromMouse(ev);
    const now = performance.now();
    if (now - lastMoveSentAt >= MOVE_THROTTLE_MS) {
        sendInput({ event: 'mouse_move', x: norm.x, y: norm.y });
        lastMoveSentAt = now;
        pendingMove = null;
    } else {
        // 节流窗口内合并最新位置，到时一次性发出。
        pendingMove = norm;
        if (!flushPendingMove.scheduled) {
            flushPendingMove.scheduled = true;
            const delay = MOVE_THROTTLE_MS - (now - lastMoveSentAt);
            setTimeout(() => {
                flushPendingMove.scheduled = false;
                flushPendingMove();
            }, Math.max(0, delay));
        }
    }
}

function onMouseDown(ev) {
    const btn = buttonName(ev.button);
    if (!btn) return;
    surfaceEl.focus();
    const { x, y } = normalizedFromMouse(ev);
    pressedButtons.add(btn);
    sendInput({ event: 'mouse_down', button: btn, x, y });
    ev.preventDefault();
}

function onMouseUp(ev) {
    const btn = buttonName(ev.button);
    if (!btn) return;
    const { x, y } = normalizedFromMouse(ev);
    pressedButtons.delete(btn);
    sendInput({ event: 'mouse_up', button: btn, x, y });
    ev.preventDefault();
}

function onWheel(ev) {
    // 带上归一化坐标，后端会先把指针挪到此处再滚动，
    // 否则滚轮事件会落在 X 指针上一次停留的窗口而非光标所在窗口。
    const { x, y } = normalizedFromMouse(ev);
    sendInput({
        event: 'wheel',
        x,
        y,
        dx: Math.round(ev.deltaX),
        dy: Math.round(ev.deltaY),
    });
    ev.preventDefault();
}

/// 当前是否该采集键盘。监听挂在 window 上，所以必须自己判断：
/// 退回登录界面后控制面板隐藏，此时按键属于密码框，不能透传更不能 preventDefault，
/// 否则密码根本打不进去。
function keyboardCaptureActive() {
    return !controlPanelEl.hidden;
}

function onKeyDown(ev) {
    if (!keyboardCaptureActive()) return;
    const clipAction = clipboardActionOf(ev);

    if (clipAction === 'paste') {
        // 粘贴键先不透传：被控端必须先拿到本地剪切板内容，再执行 Ctrl+V，
        // 否则粘出来的是被控端的旧剪切板。真正的 V 注入延后到 onPaste 里补发。
        //
        // 关键：这里不能 preventDefault，否则浏览器不会生成 paste 事件。
        // paste 事件内读 clipboardData 是唯一无需权限、不弹授权浮层的本地剪切板读取路径
        // （readText() 在 Firefox/Safari 必弹 Paste 浮层，在 Chromium 需 clipboard-read 授权）。
        pendingPaste = { at: performance.now() };
        // 置位让 onKeyUp 跳过这枚 V 的释放；V 不进 pressedKeys，
        // 因为它还没被注入过，不需要失焦时的补释放。
        pasteKeyIntercepted = true;
        // 兜底：若 paste 事件没来（浏览器差异），到时按普通按键把 V 透传出去，
        // 至少退化成被控端本地粘贴，不吞按键。
        clearTimeout(pasteFallbackTimer);
        pasteFallbackTimer = setTimeout(() => {
            if (!pendingPaste) return;
            pendingPaste = null;
            console.warn('paste 事件未触发，退化为直接透传 Ctrl+V');
            sendClipKey();
            showClipToast('粘贴：未读到本地剪切板', 'fail');
        }, PASTE_EVENT_TIMEOUT_MS);
        return;
    }

    pressedKeys.add(ev.code);
    sendInput({
        event: 'key_down',
        code: ev.code,
        modifiers: modifiersFromEvent(ev),
    });

    if (clipAction === 'copy') {
        // 复制键已透传，稍后轮询远端剪切板拉回本地。
        // 这里照常 preventDefault：我们不需要浏览器的 copy 事件，
        // 且放行 default 有可能让空选区的复制动作清空本地剪切板。
        // preventDefault 不会取消 keydown 带来的 transient activation
        // （MDN User_activation：activation 在事件派发时记录），
        // 所以稍后的 writeText() 仍在手势窗口内。
        schedulePullRemoteClipboard();
    }

    // 阻止浏览器自身处理（如 Ctrl+S 弹保存），让按键专注于注入。
    ev.preventDefault();
}

function onKeyUp(ev) {
    if (!keyboardCaptureActive()) return;
    // Ctrl+V 的 keydown 被延后注入了，对应的 keyup 也要跳过，
    // 否则被控端会收到一个没有配对按下的 V 释放。
    // 修饰键（Ctrl）的 keyup 照常透传，由 sendClipKey 自己管 V 的按下/释放。
    if (ev.code === 'KeyV' && pasteKeyIntercepted) {
        pasteKeyIntercepted = false;
        return;
    }
    pressedKeys.delete(ev.code);
    sendInput({
        event: 'key_up',
        code: ev.code,
        modifiers: modifiersFromEvent(ev),
    });
    ev.preventDefault();
}

/// 让被控端释放所有按键，并清空本地记账。
///
/// 浏览器丢 keyup 的场景（见 pressedKeys 注释）一旦发生，被控端会残留按下的
/// 修饰键，表现为「键盘被锁定」。失焦 / 页面隐藏 / 断连前都要调用这里兜底。
/// 服务端自己记着按下集合，一条 release_all 即可，无需逐个补 key_up。
function releaseAllKeys() {
    // 失焦/断连时丢掉在途的剪切板同步：兜底定时器若在用户切走之后才触发，
    // 会往被控端补发一枚没有上下文的 Ctrl+V；轮询也不该在断连后继续。
    cancelPendingClipboardSync();

    if (pressedKeys.size === 0 && pressedButtons.size === 0) return;
    pressedKeys.clear();
    pressedButtons.clear();
    sendInput({ event: 'release_all' });
}

function bindInputCapture() {
    if (inputBound) return;
    inputBound = true;

    surfaceEl.addEventListener('mousemove', onMouseMove);
    surfaceEl.addEventListener('mousedown', onMouseDown);
    surfaceEl.addEventListener('mouseup', onMouseUp);
    surfaceEl.addEventListener('wheel', onWheel, { passive: false });
    // 屏蔽右键菜单，让 right-click 能透传。
    surfaceEl.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        // 右键菜单里最可能点的就是复制/粘贴，但那条路径不经过键盘事件，
        // 前端无从得知，所以这里只提示兜底方式，不自动拉取。
        // 不自动拉的原因见 maybeHintRightClickCopy 注释。
        maybeHintRightClickCopy();
    });

    // 键盘事件挂在 window 上而非控制区：挂在 surface 上时，一旦它失去焦点
    // （点到顶部抽屉按钮、浏览器 UI、系统对话框）就收不到 keyup，
    // 被控端会残留按下的修饰键。window 级监听覆盖页面内任意焦点位置。
    // 用捕获阶段，避免被其它元素的处理器提前 stopPropagation。
    window.addEventListener('keydown', onKeyDown, true);
    window.addEventListener('keyup', onKeyUp, true);

    // 窗口失焦：Alt+Tab 这类系统快捷键会让 keyup 彻底丢失，必须补发释放。
    window.addEventListener('blur', releaseAllKeys);
    // 标签页切到后台：部分平台不触发 blur，单独兜一道。
    document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'hidden') releaseAllKeys();
    });
    // 关闭/刷新页面前释放。pagehide 比 unload 可靠（支持页面缓存场景）。
    window.addEventListener('pagehide', releaseAllKeys);

    // 自动剪切板：paste 事件挂在 window 捕获阶段，和键盘监听保持一致。
    // 画面区是 canvas（不可编辑），paste 的 target 会落到 body 之类的元素上，
    // 所以不能挂在 surfaceEl 上，必须用冒泡到 window 的全局监听。
    window.addEventListener('paste', onPaste, true);

    syncAutoClipboardLabel();

    const hint = '已就绪：移动/点击鼠标、按键将注入被控端。';
    const lockHint = navigator.keyboard && navigator.keyboard.lock
        ? '（全屏后可捕获系统快捷键）'
        : '（浏览器不支持系统快捷键捕获）';
    controlHintEl.textContent = hint + lockHint;
}

formEl.addEventListener('submit', (ev) => {
    ev.preventDefault();
    if (ws && ws.readyState === WebSocket.OPEN && helloReceived) {
        setStatus('已连接，发送认证', 'connecting');
        sendAuth();
    } else {
        connect();
    }
});

// M4: 标签页重新可见时，浏览器可能已丢弃若干帧，主动请求关键帧尽快恢复画面。
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') {
        requestKeyframe();
    }
});

// ========== M3: 视频解码 ==========

/// 初始化 WebCodecs VideoDecoder
///
/// 注意：此处只创建解码器，不立即 configure。真实的 codec 字符串
/// （profile/level）要等收到 SPS 后从码流里解析，写死会和实际码流不符。
function initVideoDecoder() {
    if (!('VideoDecoder' in window)) {
        console.error('浏览器不支持 WebCodecs API');
        controlHintEl.textContent = '浏览器不支持 WebCodecs，无法显示画面（输入仍可用）';
        return;
    }

    canvasCtx = canvasEl.getContext('2d');

    videoDecoder = new VideoDecoder({
        output: (frame) => {
            // 将解码后的帧绘制到 canvas
            canvasCtx.drawImage(frame, 0, 0, canvasEl.width, canvasEl.height);
            frame.close();
        },
        error: (e) => {
            console.error('视频解码错误:', e);
        }
    });

    console.log('VideoDecoder 已创建，等待 SPS 配置');
}

/// 根据 SPS 负载里的 profile_idc / constraint_flags / level_idc 配置解码器。
/// codec 字符串格式：avc1.PPCCLL（各两位十六进制）。
function configureDecoderFromSps(sps) {
    // 跳过 Annex-B 起始码与 NAL 头(0x67)，取后续三字节。
    let i = 0;
    if (sps.length >= 4 && sps[0] === 0 && sps[1] === 0 && sps[2] === 0 && sps[3] === 1) i = 4;
    else if (sps.length >= 3 && sps[0] === 0 && sps[1] === 0 && sps[2] === 1) i = 3;
    // i 指向 NAL 头字节，其后依次是 profile_idc, constraint_flags, level_idc。
    const profile = sps[i + 1];
    const constraint = sps[i + 2];
    const level = sps[i + 3];
    const hex = (n) => n.toString(16).padStart(2, '0');
    const codec = `avc1.${hex(profile)}${hex(constraint)}${hex(level)}`;

    videoDecoder.configure({
        codec,
        optimizeForLatency: true,
    });
    console.log('VideoDecoder 已配置:', codec);
}

/// 处理接收到的视频帧
function handleVideoFrame(data) {
    // 解码器尚未创建时直接丢弃；configure 由 SPS 触发，故这里不再要求已 configured。
    if (!videoDecoder) {
        return;
    }

    // 将 Blob 转换为 ArrayBuffer
    if (data instanceof Blob) {
        data.arrayBuffer().then(buf => processVideoPacket(buf));
    } else {
        processVideoPacket(data);
    }
}

/// 解析并解码视频包
///
/// 服务端逐 NAL 下发：frame_type 1=IDR, 2=P帧, 3=参数集(SPS/PPS)。
/// 参数集本身不解码，缓存起来；遇到 IDR 时拼成 [SPS][PPS][IDR] 一个 key chunk。
function processVideoPacket(buffer) {
    const view = new DataView(buffer);

    // 解析帧头（16 字节）
    const magic = view.getUint8(0);
    if (magic !== 0xAB) {
        console.warn('无效的视频帧 magic:', magic);
        return;
    }

    const frameType = view.getUint8(1);
    const timestampUs = view.getBigUint64(8, true);

    // H.264 负载（从字节 16 开始）。复制一份，避免后续缓存引用到整个 buffer。
    const payload = new Uint8Array(buffer.slice(16));

    // 参数集：缓存，不直接解码。SPS 的 NAL 类型为 7，PPS 为 8。
    if (frameType === 3) {
        const nalType = nalUnitType(payload);
        if (nalType === 7) {
            cachedSps = payload;
            // 首次拿到 SPS 时按真实 profile/level 配置解码器。
            if (videoDecoder.state === 'unconfigured') {
                configureDecoderFromSps(payload);
            }
        } else if (nalType === 8) {
            cachedPps = payload;
        }
        return;
    }

    // 解码器还没配好（尚未收到 SPS）时丢弃。
    if (videoDecoder.state !== 'configured') {
        return;
    }

    let data;
    let chunkType;
    if (frameType === 1) {
        // IDR 关键帧：必须内联 SPS/PPS，否则解码器无法解码。
        if (!cachedSps || !cachedPps) {
            // 尚未收到参数集，等下一个关键帧。
            return;
        }
        data = concatBytes(cachedSps, cachedPps, payload);
        chunkType = 'key';
        keyFrameDecoded = true;
    } else {
        // P 帧：在收到首个关键帧之前丢弃，否则解码器报错。
        if (!keyFrameDecoded) return;
        data = payload;
        chunkType = 'delta';
    }

    const chunk = new EncodedVideoChunk({
        type: chunkType,
        timestamp: Number(timestampUs),
        data,
    });

    try {
        videoDecoder.decode(chunk);
    } catch (e) {
        console.error('解码失败:', e);
    }
}

/// 取 Annex-B NAL 单元的类型（跳过 3 或 4 字节起始码后，首字节低 5 位）。
function nalUnitType(bytes) {
    let i = 0;
    if (bytes.length >= 4 && bytes[0] === 0 && bytes[1] === 0 && bytes[2] === 0 && bytes[3] === 1) {
        i = 4;
    } else if (bytes.length >= 3 && bytes[0] === 0 && bytes[1] === 0 && bytes[2] === 1) {
        i = 3;
    }
    return bytes[i] & 0x1f;
}

/// 拼接多个 Uint8Array。
function concatBytes(...arrays) {
    const total = arrays.reduce((sum, a) => sum + a.length, 0);
    const out = new Uint8Array(total);
    let offset = 0;
    for (const a of arrays) {
        out.set(a, offset);
        offset += a.length;
    }
    return out;
}

// ========== 全屏控制模式 + 顶部抽屉 ==========

/// 进入沉浸模式：隐藏登录界面，让画面铺满视口，显示顶部悬浮抽屉。
function enterControlMode() {
    document.body.classList.add('in-control');
    topDrawerEl.hidden = false;
}

/// 退出沉浸模式：恢复登录界面，隐藏抽屉。
function exitControlMode() {
    document.body.classList.remove('in-control');
    topDrawerEl.hidden = true;
    topDrawerEl.classList.remove('open');
}

/// 关闭 WebSocket 并退回登录界面。
function backToLogin() {
    // 先释放残留按键再关连接：连接关掉后 release_all 就发不出去了，
    // 被控端会残留按下的修饰键。
    releaseAllKeys();
    if (ws) {
        manualClose = true;
        try { ws.close(); } catch (_) { /* ignore */ }
        ws = null;
    }
    // 若处于浏览器全屏，一并退出，避免登录界面在全屏下显示。
    if (document.fullscreenElement) {
        document.exitFullscreen().catch(() => { /* ignore */ });
    }
    exitControlMode();
    controlPanelEl.hidden = true;
    setStatus('未连接', 'idle');
    hintEl.textContent = '输入密码后点击连接，验证 M1 认证流程。';
    // 清空密码并聚焦，方便重新输入。
    passwordEl.value = '';
    passwordEl.focus();
}

btnBackLoginEl.addEventListener('click', () => {
    backToLogin();
});

/// 切换浏览器全屏。请求整个文档而非 surface，避免抽屉被裹在 surface 内。
async function toggleFullscreen() {
    if (document.fullscreenElement) {
        // 退出全屏时解除键盘锁定
        if (navigator.keyboard && navigator.keyboard.unlock) {
            navigator.keyboard.unlock();
        }
        document.exitFullscreen().catch((e) => console.warn('退出全屏失败:', e));
    } else {
        try {
            await document.documentElement.requestFullscreen();
            // 进入全屏后锁定系统按键，让它们能被 keydown 捕获。
            if (navigator.keyboard && navigator.keyboard.lock) {
                await navigator.keyboard.lock([
                    'AltLeft', 'AltRight',
                    'F4', 'F11',
                    'Tab', 'Escape',
                    'MetaLeft', 'MetaRight'  // Windows 键
                ]).catch((e) => {
                    console.warn('键盘锁定失败:', e);
                });
            }
        } catch (e) {
            console.warn('进入全屏失败:', e);
        }
    }
}

/// 同步按钮文案。用户按 ESC 也能退出全屏，所以靠 fullscreenchange 而非点击事件来更新。
function syncFullscreenLabel() {
    btnToggleFullscreenEl.textContent = document.fullscreenElement ? '退出全屏' : '全屏';
}

btnToggleFullscreenEl.addEventListener('click', toggleFullscreen);
document.addEventListener('fullscreenchange', () => {
    syncFullscreenLabel();
    // 全屏切换会让 canvas 的显示矩形变化，焦点也可能丢失：重新聚焦到输入区。
    if (controlPanelEl.hidden === false) {
        surfaceEl.focus();
    }
});

btnSendClipboardEl.addEventListener('click', () => {
    sendLocalClipboard();
});

btnReadClipboardEl.addEventListener('click', () => {
    readRemoteClipboard();
});

btnAutoClipboardEl.addEventListener('click', () => {
    toggleAutoClipboard();
});

btnSendAltF4El.addEventListener('click', () => {
    sendSpecialKey('alt_f4');
});

btnSendCtrlAltDelEl.addEventListener('click', () => {
    sendSpecialKey('ctrl_alt_del');
});

/// 发送特殊快捷键组合（浏览器无法捕获的）
function sendSpecialKey(key) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ type: 'special_key', key }));
}

/// 短暂改按钮文案做反馈，到时恢复原文案。
function flashButton(btn, text, ms = 1500) {
    if (btn._restoreTimer) {
        clearTimeout(btn._restoreTimer);
    } else {
        btn._origText = btn.textContent;
    }
    btn.textContent = text;
    btn._restoreTimer = setTimeout(() => {
        btn.textContent = btn._origText;
        btn._restoreTimer = null;
    }, ms);
}

/// 传输本地剪切板：读浏览器剪切板文本，发给服务端写入被控端。
async function sendLocalClipboard() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (!navigator.clipboard || !navigator.clipboard.readText) {
        flashButton(btnSendClipboardEl, '浏览器不支持');
        return;
    }
    try {
        const text = await navigator.clipboard.readText();
        ws.send(JSON.stringify({ type: 'clipboard_set', text }));
        flashButton(btnSendClipboardEl, '已发送 ✓');
    } catch (e) {
        // 读取被拒（未授权或非安全上下文）或其他异常。
        console.warn('读取本地剪切板失败:', e);
        flashButton(btnSendClipboardEl, '读取被拒');
    }
}

/// 读取远端剪切板：向服务端请求，收到 clipboard_data 后写入浏览器剪切板。
function readRemoteClipboard() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    // 手动拉取也带指纹：重复点按钮时内容没变就不重传正文。
    // 自动轮询可能正在途中，先清掉它的标记，避免这次响应被当成自动拉取处理。
    autoPullPending = false;
    ws.send(JSON.stringify({ type: 'clipboard_get', fingerprint: lastSyncedFingerprint }));
    flashButton(btnReadClipboardEl, '读取中…');
}

/// 收到远端剪切板内容后写入本地浏览器剪切板。
async function applyRemoteClipboard(text, fingerprint) {
    if (!navigator.clipboard || !navigator.clipboard.writeText) {
        flashButton(btnReadClipboardEl, '浏览器不支持');
        return;
    }
    try {
        await navigator.clipboard.writeText(text);
        // 记下已同步内容，避免它再被当作"本地新内容"推回远端（见 lastSyncedText）。
        lastSyncedText = text;
        lastSyncedFingerprint = fingerprint || '';
        flashButton(btnReadClipboardEl, '已写入本地 ✓');
    } catch (e) {
        console.warn('写入本地剪切板失败:', e);
        flashButton(btnReadClipboardEl, '写入被拒');
    }
}

// ========== 自动剪切板同步 ==========

/// 判断这次 keydown 是不是剪切板快捷键，返回 'copy' / 'paste' / null。
///
/// 只认 Ctrl（Windows/Linux）和 Meta（macOS）单修饰的组合：带 Shift/Alt 的
/// 多半是别的快捷键（如 Ctrl+Shift+V 粘为纯文本在被控端自己处理即可）。
function clipboardActionOf(ev) {
    if (!autoClipboard) return null;
    if (!ws || ws.readyState !== WebSocket.OPEN) return null;
    // Ctrl 和 Meta 二者有其一，且不叠加 Shift/Alt。
    const cmd = ev.ctrlKey !== ev.metaKey;
    if (!cmd || ev.shiftKey || ev.altKey) return null;
    if (ev.code === 'KeyC' || ev.code === 'KeyX') return 'copy';
    if (ev.code === 'KeyV') return 'paste';
    return null;
}

/// 补发被延后的 Ctrl+V。
///
/// 这里要解决一个时序问题：onKeyDown 虽然已经把真实的 Ctrl 按下透传过了，
/// 但从那时到这里隔了一次 clipboard_set 往返（兜底路径更是隔了 300ms），
/// 用户可能已经松开 Ctrl —— 它的 keyup 会照常透传，失焦时 release_all
/// 也会把它释放。那样这里补的 V 就成了裸 V，会往被控端打进一个字母 v。
///
/// 所以按 Ctrl 的实际状态分两种发法，而不是无条件自己包一层：
/// 用户还按着时只补 V（自己再发一遍 Ctrl 的释放，会把他正按着的 Ctrl 提前
/// 松掉，之后的 Ctrl+A 就退化成裸 A）；已经松开了才自己成对包一次 Ctrl。
///
/// `pressedKeys` 是键盘那边维护的「已透传按下、尚未透传释放」集合，
/// 真实的 Ctrl 走的是普通按键路径，所以它的状态在这里是准的。
function sendClipKey() {
    // 用户此刻真按着的修饰键，要如实带进 modifiers。漏报会让服务端的
    // reconcile_modifiers 认为「浏览器说它已松」，把被控端对应的键补发释放。
    // 典型是 macOS 的 Cmd+V：按着的是 MetaLeft，若只报 ['ctrl']，
    // 服务端会把被控端的 Meta 释放掉，记账和实际短暂不一致。
    const mods = ['ctrl'];
    if (pressedKeys.has('MetaLeft') || pressedKeys.has('MetaRight')
        || pressedKeys.has('OSLeft') || pressedKeys.has('OSRight')) {
        mods.push('meta');
    }

    const ctrlHeld = pressedKeys.has('ControlLeft') || pressedKeys.has('ControlRight');
    if (ctrlHeld) {
        sendInput({ event: 'key_down', code: 'KeyV', modifiers: mods });
        sendInput({ event: 'key_up', code: 'KeyV', modifiers: mods });
        return;
    }
    // Ctrl 已松开（或 macOS 下用户按的是 Cmd）：自己成对包一次 Ctrl。
    // 被控端是 Windows/Linux，Ctrl+V 才是那边的粘贴键，所以 Cmd+V 也注入 Ctrl+V。
    sendInput({ event: 'key_down', code: 'ControlLeft', modifiers: mods });
    sendInput({ event: 'key_down', code: 'KeyV', modifiers: mods });
    sendInput({ event: 'key_up', code: 'KeyV', modifiers: mods });
    // 末尾释放自己按下的 Ctrl：去掉 'ctrl'，但保留用户仍按着的 meta。
    sendInput({ event: 'key_up', code: 'ControlLeft', modifiers: mods.filter((m) => m !== 'ctrl') });
}

/// Ctrl+V：在 paste 事件里拿到本地剪切板文本，推给被控端，再补发粘贴键。
///
/// 这是整个方案的关键路径——事件处理器内读 clipboardData 不需要 clipboard-read
/// 权限、不弹授权浮层（MDN Clipboard_API：事件由用户发起，故 handler 内免检），
/// 而 readText() 在 Firefox/Safari 每次都会弹 Paste 浮层。
function onPaste(ev) {
    if (!pendingPaste) return; // 不是我们拦下的那次粘贴，交给浏览器默认行为
    pendingPaste = null;
    clearTimeout(pasteFallbackTimer);
    ev.preventDefault(); // 画面区不需要浏览器自己插入文本

    const text = (ev.clipboardData || window.clipboardData).getData('text/plain') || '';

    if (!text) {
        // 剪切板是空的，或装的是图片/文件（本次不支持非文本）。
        // 仍然把 Ctrl+V 透传出去，让被控端用它自己的剪切板粘贴。
        sendClipKey();
        showClipToast('粘贴：本地剪切板无文本');
        return;
    }

    // 超限时不推送，但按键照常透传，退化成被控端本地粘贴。
    const bytes = new TextEncoder().encode(text).length;
    if (bytes > CLIP_MAX_BYTES) {
        console.warn(`本地剪切板 ${bytes} 字节超过上限 ${CLIP_MAX_BYTES}，跳过同步`);
        sendClipKey();
        showClipToast('粘贴：内容过大，未同步', 'fail');
        return;
    }

    if (text === lastSyncedText) {
        // 两端内容已经一致（通常是刚从远端拉回来的），不必再推一遍。
        // 这是断开 A→B→A 回环的地方。
        sendClipKey();
        showClipToast('粘贴：内容一致，免同步');
        return;
    }

    // 顺序有保证：服务端的 WS 循环是顺序 await，clipboard_set 会先把文本写进
    // 被控端系统剪切板，之后才处理这条 input 消息，所以粘出来的一定是新内容。
    ws.send(JSON.stringify({ type: 'clipboard_set', text }));
    lastSyncedText = text;
    sendClipKey();
    showClipToast(`已同步到远端（${bytes} 字节）`);
}

/// Ctrl+C/X：按键已透传，延迟后轮询被控端剪切板，拿到新内容就写入本地。
///
/// 要延迟和重试，是因为被控端的复制动作不是瞬时的：按键注入到应用真正把内容
/// 放进系统剪切板之间有几十到上百毫秒。拉到的还是旧内容就再等一会儿。
function schedulePullRemoteClipboard() {
    clearTimeout(pullTimer);
    pullRetriesLeft = PULL_MAX_RETRIES;
    showClipToast('正在从远端取…');
    pullTimer = setTimeout(requestRemoteClipboardForAuto, PULL_FIRST_DELAY_MS);
}

function requestRemoteClipboardForAuto() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    autoPullPending = true;
    // 带上已持有内容的指纹：内容没变时服务端回 clipboard_unchanged（几十字节），
    // 不会把同一份文本在重试里反复下载。
    ws.send(JSON.stringify({ type: 'clipboard_get', fingerprint: lastSyncedFingerprint }));
}

/// 处理自动轮询拿回的远端剪切板内容（内容确有变化时服务端才发这条）。
async function onAutoClipboardData(text, fingerprint) {
    if (!text || text === lastSyncedText) {
        // 空剪切板，或内容与已持有的相同（老服务端不认 fingerprint 时会走到这里）。
        retryPullOrGiveUp();
        return;
    }
    // 拿到了新内容，写入本地剪切板。
    // 这里仍在 Ctrl+C 的 transient activation 窗口内（总延迟 < 1s，窗口约 5s），
    // 所以 Firefox/Safari 的 writeText() 也能通过。
    try {
        await navigator.clipboard.writeText(text);
        lastSyncedText = text;
        lastSyncedFingerprint = fingerprint || '';
        pullRetriesLeft = 0;
        showClipToast(`已从远端取回（${text.length} 字符）`);
    } catch (e) {
        console.warn('写入本地剪切板失败:', e);
        pullRetriesLeft = 0;
        showClipToast('取回失败：浏览器拒绝写入', 'fail');
    }
}

/// 远端内容还没变：被控端的复制动作可能没完成，再等一轮。
///
/// 服务端回 clipboard_unchanged 时也走这里 —— 那条消息只有几十字节，
/// 重试的流量代价与正文无关，不会因为剪切板大就反复下载。
function retryPullOrGiveUp() {
    if (pullRetriesLeft > 0) {
        pullRetriesLeft -= 1;
        clearTimeout(pullTimer);
        pullTimer = setTimeout(requestRemoteClipboardForAuto, PULL_RETRY_DELAY_MS);
        return;
    }
    // 重试用尽。可能是用户复制的内容本来就和当前剪切板相同，也可能没复制成功。
    showClipToast('远端剪切板无变化');
}

/// 右下角浮层提示。剪切板里常有密码，每次自动同步都让用户看见方向和大小。
function showClipToast(text, state, ms = 2000) {
    clipToastEl.textContent = text;
    clipToastEl.dataset.state = state || 'ok';
    clipToastEl.hidden = false;
    clearTimeout(showClipToast._timer);
    showClipToast._timer = setTimeout(() => {
        clipToastEl.hidden = true;
    }, ms);
}

/// 右键菜单弹出时提示一次「右键复制请点工具栏按钮取回」。
///
/// 为什么不自动拉取：右键复制走不到键盘事件，前端只知道"用户开了右键菜单"，
/// 不知道他点的是复制、粘贴还是别的项。自动拉会带来三个问题：
///   1. 每次右键都发一轮 clipboard_get + 重试，用户的服务器只有 3 Mbps，
///      白跑的请求要和视频流抢带宽；
///   2. 拿到内容后得调 writeText()，它要 transient activation。规范上
///      右键的 mousedown 算 activation（HTML spec 的事件列表不区分按键），
///      但各引擎对副按键的处理有分歧，拿不准就会变成偶发失败；
///   3. 用户只是想右键看看菜单就关掉时，远端剪切板会被无意拉到本地。
/// 所以这里只做提示，拉取由用户点按钮触发 —— 那是确定的手势，writeText() 必过。
///
/// 每次连接只提示一次，避免每右键一下都弹。
function maybeHintRightClickCopy() {
    if (rightClickHintShown) return;
    rightClickHintShown = true;
    showClipToast('右键复制的内容，请点工具栏「读取远端剪切板」取回', 'ok', 4500);
}

/// 取消所有在途的自动同步状态：待处理的粘贴、兜底定时器、远端轮询。
/// 失焦、断连、关闭开关都要调，避免定时器在上下文已失效后才落地。
function cancelPendingClipboardSync() {
    pendingPaste = null;
    pasteKeyIntercepted = false;
    autoPullPending = false;
    pullRetriesLeft = 0;
    clearTimeout(pasteFallbackTimer);
    clearTimeout(pullTimer);
}

/// 同步开关按钮的文案与置灰状态。
function syncAutoClipboardLabel() {
    btnAutoClipboardEl.textContent = `自动剪切板：${autoClipboard ? '开' : '关'}`;
    btnAutoClipboardEl.dataset.off = autoClipboard ? '0' : '1';
}

/// 切换自动同步开关。关掉后退回纯手动按钮模式。
function toggleAutoClipboard() {
    autoClipboard = !autoClipboard;
    syncAutoClipboardLabel();
    if (!autoClipboard) {
        // 取消在途的轮询，别让关掉之后还有一次拉取落地。
        cancelPendingClipboardSync();
    }
    showClipToast(autoClipboard ? '自动剪切板已开启' : '自动剪切板已关闭');
}

/// 更新控制区的光标样式
function updateCursor(width, height, xhot, yhot, rgba_base64) {
    try {
        // 将 base64 解码为 Uint8Array
        const binaryString = atob(rgba_base64);
        const bytes = new Uint8Array(binaryString.length);
        for (let i = 0; i < binaryString.length; i++) {
            bytes[i] = binaryString.charCodeAt(i);
        }

        // 创建 ImageData
        const imageData = new ImageData(width, height);
        imageData.data.set(bytes);

        // 用 OffscreenCanvas 转成 data URL
        const canvas = new OffscreenCanvas(width, height);
        const ctx = canvas.getContext('2d');
        ctx.putImageData(imageData, 0, 0);

        canvas.convertToBlob({ type: 'image/png' }).then(blob => {
            const url = URL.createObjectURL(blob);
            // CSS cursor 语法：url(data) xhot yhot, fallback
            surfaceEl.style.cursor = `url('${url}') ${xhot} ${yhot}, auto`;
            // 释放旧的 URL（如果有）
            if (surfaceEl._cursorUrl) {
                URL.revokeObjectURL(surfaceEl._cursorUrl);
            }
            surfaceEl._cursorUrl = url;
        });
    } catch (e) {
        console.warn('更新光标失败:', e);
    }
}
