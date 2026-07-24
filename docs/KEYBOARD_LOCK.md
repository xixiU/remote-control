# 系统快捷键捕获方案

## 问题
Web 浏览器出于安全限制，无法直接捕获系统级快捷键（如 `Alt+F4`、`Ctrl+Alt+Del`、`Alt+Tab` 等）。

## 解决方案

### 方案 1：Keyboard Lock API（全屏时生效）✅

使用 Web 标准的 [Keyboard Lock API](https://developer.mozilla.org/en-US/docs/Web/API/Keyboard/lock) 在全屏模式下拦截系统快捷键。

**实现位置**：web/app.js:548-571

```javascript
await document.documentElement.requestFullscreen();
await navigator.keyboard.lock([
    'AltLeft', 'AltRight',
    'F4', 'F11',
    'Tab', 'Escape',
    'MetaLeft', 'MetaRight'
]);
```

**浏览器兼容性**：
- ✅ Chrome 68+
- ✅ Edge 79+
- ❌ Firefox（尚未支持）
- ❌ Safari（尚未支持）

**用户体验**：
- 进入全屏后自动锁定系统按键
- 退出全屏时自动解锁
- 用户界面会提示 "（全屏后可捕获系统快捷键）"

---

### 方案 2：专用按钮（始终可用）✅

在顶部抽屉添加专用按钮，通过 WebSocket 消息直接让服务端注入特殊快捷键。

**前端实现**：
- web/index.html:47-48 - 按钮 UI
- web/app.js:581-594 - 消息发送逻辑

**后端实现**：
- crates/server/src/protocol.rs:96-102 - SpecialKey 消息定义
- crates/server/src/web.rs:504-524 - 消息处理
- crates/server/src/web.rs:730-776 - 快捷键注入函数

**支持的快捷键**：
| 快捷键 | 功能 | 标识符 |
|--------|------|--------|
| Alt+F4 | 关闭当前窗口 | alt_f4 |
| Ctrl+Alt+Del | Windows 安全屏幕 | ctrl_alt_del |
| Win+D | 显示桌面 | win_d |
| Win+L | 锁定计算机 | win_l |

**协议格式**：
```json
{
  "type": "special_key",
  "key": "alt_f4"
}
```

---

## 使用指南

### 普通用户
1. **日常操作**：正常按键即可（Ctrl+C、Ctrl+V 等）
2. **系统快捷键**：
   - 点击 **全屏** 按钮后自动启用 Keyboard Lock（支持的浏览器）
   - 或点击顶部抽屉的专用按钮（如 Alt+F4、Ctrl+Alt+Del）

### 开发者
添加新的特殊快捷键：

1. 在 web/app.js 中添加按钮（可选）
2. 在 crates/server/src/web.rs:handle_special_key() 中添加处理逻辑：
   ```rust
   "win_e" => {
       // Windows+E：打开文件资源管理器
       injector.key("MetaLeft", true)?;
       injector.key("KeyE", true)?;
       injector.key("KeyE", false)?;
       injector.key("MetaLeft", false)?;
       Ok(())
   }
   ```

---

## 限制

### Windows 安全限制
Ctrl+Alt+Del 在 Windows 上受到特殊保护，SendInput API **无法模拟**此组合键。要真正注入需要：
- 驱动级输入注入（如 Interception Driver）
- 或专用的 Secure Attention Sequence (SAS) API

当前实现会尝试注入，但在标准权限下可能无效。

### Keyboard Lock API 限制
- 必须在**全屏模式**下才能生效
- 用户按 Esc 可退出全屏（同时解除锁定）
- 某些浏览器可能会显示权限提示

---

## 测试

### 单元测试
```bash
cargo test special_key_message_parses
```

### 手动测试
1. 启动服务端：cargo run --release
2. 打开浏览器访问 https://localhost:9443
3. 认证后，点击 **全屏** 按钮
4. 测试按下 Alt+F4（应关闭被控端的当前窗口）
5. 或点击顶部抽屉的 Alt+F4 按钮

---

## 相关资源

- [Keyboard Lock API - MDN](https://developer.mozilla.org/en-US/docs/Web/API/Keyboard/lock)
- [Can I Use: Keyboard Lock](https://caniuse.com/mdn-api_keyboard_lock)
- [Windows SendInput API](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
