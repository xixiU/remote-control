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
const btnSendClipboardEl = document.getElementById('btn-send-clipboard');
const btnReadClipboardEl = document.getElementById('btn-read-clipboard');

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
        case 'auth_ok':
            setStatus('Ready to control', 'ok');
            hintEl.textContent = `会话 ID: ${msg.session_id}`;
            controlPanelEl.hidden = false;
            bindInputCapture();
            enterControlMode();
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
            // 远端剪切板内容到达，写入本地浏览器剪切板。
            applyRemoteClipboard(msg.text || '');
            break;
        case 'clipboard_error':
            console.warn('剪切板操作失败:', msg.op, msg.message);
            if (msg.op === 'get') {
                flashButton(btnReadClipboardEl, '远端失败');
            } else if (msg.op === 'set') {
                flashButton(btnSendClipboardEl, '远端失败');
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
    sendInput({ event: 'mouse_down', button: btn, x, y });
    ev.preventDefault();
}

function onMouseUp(ev) {
    const btn = buttonName(ev.button);
    if (!btn) return;
    const { x, y } = normalizedFromMouse(ev);
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

function onKeyDown(ev) {
    sendInput({
        event: 'key_down',
        code: ev.code,
        modifiers: modifiersFromEvent(ev),
    });
    // 阻止浏览器自身处理（如 Ctrl+S 弹保存），让按键专注于注入。
    ev.preventDefault();
}

function onKeyUp(ev) {
    sendInput({
        event: 'key_up',
        code: ev.code,
        modifiers: modifiersFromEvent(ev),
    });
    ev.preventDefault();
}

function bindInputCapture() {
    if (inputBound) return;
    inputBound = true;

    surfaceEl.addEventListener('mousemove', onMouseMove);
    surfaceEl.addEventListener('mousedown', onMouseDown);
    surfaceEl.addEventListener('mouseup', onMouseUp);
    surfaceEl.addEventListener('wheel', onWheel, { passive: false });
    // 屏蔽右键菜单，让 right-click 能透传。
    surfaceEl.addEventListener('contextmenu', (e) => e.preventDefault());

    // 键盘事件挂在控制区上，需要焦点。
    surfaceEl.addEventListener('keydown', onKeyDown);
    surfaceEl.addEventListener('keyup', onKeyUp);

    controlHintEl.textContent = '已就绪：移动/点击鼠标、按键将注入被控端。';
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
function toggleFullscreen() {
    if (document.fullscreenElement) {
        document.exitFullscreen().catch((e) => console.warn('退出全屏失败:', e));
    } else {
        document.documentElement.requestFullscreen().catch((e) => {
            console.warn('进入全屏失败:', e);
        });
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
    ws.send(JSON.stringify({ type: 'clipboard_get' }));
    flashButton(btnReadClipboardEl, '读取中…');
}

/// 收到远端剪切板内容后写入本地浏览器剪切板。
async function applyRemoteClipboard(text) {
    if (!navigator.clipboard || !navigator.clipboard.writeText) {
        flashButton(btnReadClipboardEl, '浏览器不支持');
        return;
    }
    try {
        await navigator.clipboard.writeText(text);
        flashButton(btnReadClipboardEl, '已写入本地 ✓');
    } catch (e) {
        console.warn('写入本地剪切板失败:', e);
        flashButton(btnReadClipboardEl, '写入被拒');
    }
}
