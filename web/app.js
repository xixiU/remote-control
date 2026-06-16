// RemoteControl 前端
// M1：WebSocket 连接 + 密码认证流程验证。
// 后续里程碑将在此实现输入采集、WebCodecs 解码渲染。

const statusEl = document.getElementById('status');
const hintEl = document.getElementById('hint');
const formEl = document.getElementById('auth-form');
const passwordEl = document.getElementById('password');
const connectBtn = document.getElementById('connect-btn');

let ws = null;
let helloReceived = false;

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
            // 收到 hello 后提交认证。
            sendAuth();
            break;
        case 'auth_ok':
            setStatus('认证成功', 'ok');
            hintEl.textContent = `会话 ID: ${msg.session_id}`;
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
    if (!ws || ws.readyState !== WebSocket.OPEN) {
        return;
    }
    const token = passwordEl.value;
    ws.send(JSON.stringify({ type: 'auth', token }));
}

formEl.addEventListener('submit', (ev) => {
    ev.preventDefault();
    // 已连接且收到 hello 则直接重发认证，否则先建立连接（onmessage 收到 hello 后自动发认证）。
    if (ws && ws.readyState === WebSocket.OPEN && helloReceived) {
        setStatus('已连接，发送认证', 'connecting');
        sendAuth();
    } else {
        connect();
    }
});
