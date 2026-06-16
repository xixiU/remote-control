// RemoteControl 前端
// M1：WebSocket 连接 + 密码认证。
// M2：在认证后采集鼠标/键盘事件，归一化坐标后发送到服务端注入到被控端。

const statusEl = document.getElementById('status');
const hintEl = document.getElementById('hint');
const formEl = document.getElementById('auth-form');
const passwordEl = document.getElementById('password');
const controlPanelEl = document.getElementById('control-panel');
const surfaceEl = document.getElementById('control-surface');
const controlHintEl = document.getElementById('control-hint');

let ws = null;
let helloReceived = false;
let inputBound = false;

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
