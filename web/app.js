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

let ws = null;
let helloReceived = false;
let inputBound = false;

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
        setStatus('连接已断开', 'fail');
        hintEl.textContent = '连接已关闭，可重新输入密码连接。';
        controlPanelEl.hidden = true;
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
            initVideoDecoder();
            break;
        case 'auth_ok':
            setStatus('Ready to control', 'ok');
            hintEl.textContent = `会话 ID: ${msg.session_id}`;
            controlPanelEl.hidden = false;
            bindInputCapture();
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

// 把控制区内的鼠标坐标换算成 0~1 归一化值，按 docs/03 第 3.1 节约定。
function normalizedFromMouse(ev) {
    const rect = surfaceEl.getBoundingClientRect();
    const x = (ev.clientX - rect.left) / rect.width;
    const y = (ev.clientY - rect.top) / rect.height;
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
    sendInput({
        event: 'wheel',
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
