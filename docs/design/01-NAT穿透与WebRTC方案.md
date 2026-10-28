# 01 - 外网双向控制方案调研与设计

> **要解决的问题**：现在被控端在 NAT 内、控制端在外网时完全不通。本文调研**所有**可行路线，按「用最小代价实现双向控制」排序，而不是预设某个技术。
>
> **不重复**：总体架构见 [../01-架构设计.md](../01-架构设计.md)，crate 选型基线见 [../02-技术选型.md](../02-技术选型.md)，现有消息/帧格式见 [../03-通信协议.md](../03-通信协议.md)，认证与威胁模型见 [../04-安全设计.md](../04-安全设计.md)，里程碑风格见 [../05-实施计划.md](../05-实施计划.md)。
>
> **文档性质**：调研 + 设计，不含实现代码。文中严格区分「已核实」（读过代码 / 查过官方文档）与「待实测 / 推测」，见 [附录 A](#附录-a已核实事实与推测的边界)。

---

## 0. 结论先行

### 0.1 推荐做什么

**先用 frp 反向隧道把外网双向控制跑通，本工程一行代码都不用改。** 自研中继和 WebRTC 作为后续可选演进，不是现在要做的事。

理由三句话：现有被控端已经是标准 WSS 服务（listen 在 `0.0.0.0:8443`），frpc 跑在被控端机器上，替它完成了「主动出站」这件唯一缺失的能力 —— 而这恰好就是自研路线要花 10~15 人日去实现的东西。用户已有一台公网服务器，装 frps 是一下午的事。**先用现成方案验证「双向控制在外网到底好不好用」，再决定要不要投入自研，这个顺序能避免把 20~40 人日押在未验证的需求上。**

### 0.2 决策树

```
只在局域网内用？
 └─ 是 → 【什么都不用做】现状已满足。见 docs/README.md 的需求基线。
 └─ 否 ↓

要外网控制，有公网服务器，希望本工程零改动？
 └─ 是 → 【路线 Z：frp 反向隧道】★ 最推荐
          本工程零改动，一下午搞定，完整配置见第 9 节。
          代价：全部流量过你的服务器（带宽成本见第 11 节）。
 └─ 否 ↓

不想自己维护服务器？
 └─ 是 → 【Cloudflare Tunnel】⚠ 条款风险
          免费、自带 TLS 和域名、WebSocket 可用，但 Cloudflare
          服务条款明确要求「通过 CDN 传视频必须用付费产品」。
          本工程传的就是视频流，长期高带宽使用有被限速/封禁风险。
          见第 10 节的原文引用。
 └─ 否 ↓

要产品化 / 多设备管理 / 不让用户碰网络配置？
 └─ 是 → 【路线 0：自研应用层中继】见第 12 节
          带宽成本与 frp 完全相同，但换来设备管理、
          短码呼叫、统一前端入口等产品化能力。
 └─ 还要省中继带宽？
     └─ 是 → 【WebRTC】见第 2~8 节
              P2P 打通时省服务器带宽，且中继看不到明文。
              代价最大：+60~90 crate、信令+TURN 两套服务、调试困难。
```

### 0.2.1 本文阅读顺序

本文的章节顺序是**按技术深度**而非推荐优先级排列的，请按需跳读：

| 节 | 内容 | 谁该读 |
| --- | --- | --- |
| **1** | 问题定位：反向为什么不通 | **所有人必读**，这是所有路线的共同前提 |
| **9** | **路线 Z：frp 反向隧道（最推荐，含完整配置）** | **想尽快跑通外网控制的人**，可从第 1 节直接跳到这里 |
| **10** | Cloudflare Tunnel 评估（含条款风险核实） | 不想自己维护服务器的人 |
| **11** | **加密的真实边界 + E2E 加密方案** | **所有选中继/隧道路线的人必读** |
| **12** | 路线 0：自研应用层中继设计 + 带宽成本精确计算 | 考虑产品化的人 |
| 2~8 | WebRTC 完整设计（原理、路线取舍、crate 选型、信令设计、实施计划） | **设计储备**，确认要做 P2P 时再读 |
| 13 | 成熟开源方案对比（RustDesk / MeshCentral） | 评估「是否该继续自研」时读 |

**第 2~8 节是未来演进的设计储备**，内容完整且事实已核实，但按 0.1 节的结论，现在不建议实施。

### 0.3 直接回答「WebRTC 相比纯中继到底强在哪」

| | WebRTC 的优势 | 代价 |
| --- | --- | --- |
| **带宽** | 打洞成功时流量不过服务器，省中继带宽 | 打洞失败的那部分仍需 TURN，成本与纯中继相同 |
| **延迟** | P2P 直连少一跳 | 建连慢（ICE 协商需数百 ms 到数秒） |
| **加密** | **DTLS 端到端，中继看不到明文**（这是真正的硬优势，已核实） | 无 |
| **工程** | — | +60~90 crate、信令 + TURN 两套服务、`web.rs` 核心循环重构、调试困难、pre-1.0 库 |
| **运维** | — | 两套公网服务，TURN 要防白用 |

**建议：现在不做。** 理由是 WebRTC 的三个优势里，带宽和延迟都只在「打洞成功」时成立，而打洞成功率**缺乏可引用的权威实测数据**（见 [附录 A.3](#a3-推测--待实测--不可作为决策依据)，我核实了多个常被引用的来源，发现数字多为估计值而非实测）；唯一确定成立的优势是端到端加密，**而这一点可以用「在中继方案上加一层应用层 E2E 加密」以低得多的成本达成**（见 [第 11.3 节](#113-如果要做到中继看不见内容)）。

### 0.4 必须讲清楚的两件事

**（1）加密的真实边界 —— 两段 TLS 不等于端到端加密**

frp 和自研中继都是**两段 TLS**（浏览器↔服务器、服务器↔被控端），TLS 在服务器上终止，**公网服务器能看到完整的屏幕画面和键盘输入明文**。远程桌面内容高度敏感（可能含密码输入、私密文件），这一点必须明确知晓。如果服务器完全由用户自己掌控，这个风险可接受；如果不是，必须加应用层 E2E 加密。详见 [第 11 节](#11-加密的真实边界)。

**（2）带宽成本 —— 中继方案的真实代价**

按现有 `auto_bitrate()` 实际公式计算（`encoder.rs:173`，已读代码核实），1080p@30fps 单路会话 **6.22 Mbps**，中继需要**进出双份**。完整表格见 [第 12.4 节](#124-带宽成本精确计算)。关键数字：**一台 100 Mbps 出向带宽的服务器，1080p@30 只能支撑约 16 路并发**；建议中继模式下默认降到 720p 或 15fps。

---

## 1. 问题定位：反向为什么不通

### 1.1 代码事实

被控端的**全部网络行为**集中在 `crates/server/src/web.rs:135-153`，只有两个分支，都是 `listen`：

```rust
// web.rs:141  TLS 分支
axum_server::bind_rustls(addr, tls_config)
    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
// web.rs:145  明文分支
let listener = TcpListener::bind(&bind).await?;
```

`bind` 默认 `0.0.0.0:8443`（`config.rs:96`）。全工程 **grep 不到任何 outbound 连接代码** —— 没有 `TcpStream::connect`，没有 HTTP 客户端 crate（`Cargo.toml` 里连 `reqwest`/`hyper-client` 都没有）。

控制端侧，`web/app.js:53-54` 固定用当前页面 host 回连：

```js
const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
return `${scheme}://${location.host}/ws`;
```

即前端是从「被控端自己托管的静态页面」加载的，天然假设浏览器已经能访问到被控端。

### 1.2 被拦在哪一跳

```
控制端浏览器                 公网                 被控端家庭/企业 NAT           被控端主机
    │                                                   │                      │
    │──── TCP SYN → 公网IP:8443 ────────────────────────>│                      │
    │                                                   │  查 NAT 映射表        │
    │                                                   │  无此表项 → DROP      │
    │<─────────── (超时，或 ICMP unreachable) ───────────│                      │
```

拦截点有三层，任一层生效就不通：

1. **NAT 设备（最根本）**：NAT 是「出站时建表、入站时查表」。被控端从不出站，映射表就没有 `公网IP:port → 内网IP:8443` 的条目，入站包无处投递，直接丢弃。这是**协议性的不通，不是配置问题**（除非手工做端口映射 / UPnP）。
2. **运营商 CGNAT**：家庭宽带越来越多走 CGNAT（运营商级 NAT），用户连路由器管理页都改不了映射，手工端口映射这条路直接断掉。
3. **主机防火墙**：Windows Defender 防火墙默认拦入站，Linux firewalld/ufw 同理。

对比：现有场景能跑通，是因为局域网内没有 NAT 隔离（同一广播域），或被控端在公网 / 已做端口映射。`docs/README.md` 的需求基线写的「同一局域网 / 可直连，无需中转服务器」就是这个前提，本方案正是要突破它。

### 1.3 结论：必须反转连接方向

> 要让 NAT 内的被控端可达，**唯一的通用解法是让被控端主动出站**，在某个双方都能访问的公网点「会合」。

出站是 NAT 友好的（出站即建表），所有穿透技术（WebRTC、frp、SSH 反向隧道、Cloudflare Tunnel）本质都是这一招，区别只在「会合之后数据怎么走」。

这意味着**被控端必须新增 outbound 能力** —— 这是本方案对现有架构最本质的改动，而不是加个 crate 那么简单。

### 1.4 一个必须纠正的认知：STUN ≠ TURN，Google 不提供 TURN

需求讨论里提到的「google 的 TUNL」，正确名称是 **STUN**（Session Traversal Utilities for NAT，RFC 8489）。这两者**完全不是一回事**：

| | STUN | TURN |
| --- | --- | --- |
| 全称 | Session Traversal Utilities for NAT | Traversal Using Relays around NAT |
| 干什么 | 告诉你「你的公网 IP:端口是什么」，然后两端自己打洞直连 | 服务器**代转发所有流量**，两端都只和服务器通信 |
| 流量过服务器 | **不过**（只过几个探测包，几 KB） | **全过**（视频流有多大就占多少带宽） |
| 服务器成本 | 极低，可白嫖公共服务 | 高，按流量计费，**必须自建或购买** |
| Google 是否提供公共服务 | **提供**：`stun.l.google.com:19302`（另有 stun1~stun4.l.google.com） | **不提供**，从来没有过 |

两个必须讲清楚的点：

1. **Google 只有公共 STUN，没有公共 TURN。** 这是行业共识，也容易理解 —— STUN 只回一个 UDP 包，成本可忽略；TURN 要代转发视频流，Google 没有理由给全世界免费当中继。社区里的公认说法是：Google STUN 可用于开发，但**无 SLA、不适合商业产品依赖**（[discuss-webrtc 官方邮件组](https://groups.google.com/g/discuss-webrtc/c/shcPIaPxwo8/m/F8Vc8QlIyn0J) 里 WebRTC 团队的回复即是「free for development purposes, no SLA, 商业部署应自行部署」）。网上流传的「免费 TURN 列表」要么是别人的测试服务器（随时关停、带宽被挤爆），要么是免费试用额度，**不能作为产品依赖**。

2. **两端都是对称型 NAT（Symmetric NAT）时，STUN 打洞必然失败，只能走 TURN。** 对称型 NAT 对**每个不同的目标地址**都分配不同的外部端口，所以 A 通过 STUN 问到的「我的公网端口」只对 STUN 服务器有效，B 往那个端口发包会被 NAT 当作非法流量丢弃 —— 洞打不通。此时唯一出路是 TURN 中继。行业经验值是约 **10%~20% 的连接最终需要 TURN 兜底**（具体比例取决于用户网络构成，企业网/校园网/CGNAT 环境占比更高；这个比例是业界通行估计，本工程未实测）。

#### TURN 与 coturn 的关系（容易混淆，澄清一下）

- **TURN 是协议**，由 RFC 8656 定义，规定了「客户端如何向中继服务器申请一个中继地址、如何通过它收发数据」。它是一份规范文档，不是软件。
- **coturn 是实现**，是最常用的开源 TURN 服务器软件（C 写的，同时实现 STUN + TURN，Debian/Ubuntu/CentOS 均有包，Docker 镜像现成）。仓库：[github.com/coturn/coturn](https://github.com/coturn/coturn)。

类比：HTTP 是协议，nginx 是实现。说「装一个 TURN」在口语上没问题，准确说法是「装一个 TURN 服务器实现，通常选 coturn」。

#### 「没有免费午餐」的真正含义

Google 不提供公共 TURN**不是吝啬，而是成本结构决定的**：

- STUN 的工作是「收一个 UDP 包，回一个 UDP 包告诉你公网地址」。单次交互几百字节，一台服务器能服务海量客户端，**成本可忽略** → 所以 Google 愿意免费开放。
- TURN 的工作是**代转发全部媒体流量**。按本工程 1080p@30fps 的 6.22 Mbps 计算，中继一路会话就要占服务器 6.22 Mbps 进 + 6.22 Mbps 出。**带宽是真金白银**，没有任何公司会免费给全世界当中继。

所以网上流传的「免费 TURN 列表」要么是别人的测试服务器（随时关停、带宽被挤爆），要么是免费试用额度，**不能作为产品依赖**。

#### 用户已有公网服务器，这台机器怎么用

这是个关键的现实判断。**同一台公网服务器，两种用法：**

| 用法 | 装什么 | 复杂度 | 流量 |
| --- | --- | --- | --- |
| **当隧道 / 中继服务器** | frps（路线 Z）或自研中继（路线 0） | **低** | 全部过它 |
| 当 TURN 服务器 | coturn + 还要另外部署信令服务 | **高**（两套服务 + 本工程改造） | 只有打洞失败的那部分过它 |

**后者简单得多的反面：WebRTC 路线下这台机器要同时跑 coturn 和信令服务，还要改本工程的代码。** 而前者只是装个 frps，本工程零改动。既然服务器带宽是同一份，**在验证需求阶段，没有理由先选复杂的那条路**。

**所以必须明确预期：想做到「任意网络下都能连」，TURN 这笔运维成本躲不掉。** 只上 STUN 的方案能覆盖大部分家宽场景，但会有一部分用户连不上，且失败原因对用户完全不可见（表现为「转圈然后超时」）—— 这是最差的用户体验，必须要么上 TURN，要么保留一条隧道兜底路径（见 [第 7 节](#7-横向对比诚实评估)）。

---

## 2. WebRTC 怎么解决

### 2.1 ICE：候选收集与连通性检查

WebRTC 不是「一种连接方式」，而是一套**自动择优的连通性框架**（ICE，Interactive Connectivity Establishment，RFC 8445）。两端各自收集所有可能的「候选地址」，交换后两两配对做连通性检查，选出能通且最优的一对。

候选分三类，优先级从高到低：

| 候选类型 | 来源 | 适用场景 | 流量路径 |
| --- | --- | --- | --- |
| **host**（主机候选） | 本机网卡地址，无需任何外部服务 | 同一局域网 | 完全直连，零第三方 |
| **srflx**（服务器自反候选） | 问 STUN「我的公网地址是什么」 | 跨 NAT，非对称型 | **P2P 直连**，流量不过服务器 |
| **relay**（中继候选） | 向 TURN 申请一个中继地址 | 对称型 NAT / UDP 全封 | 全程过 TURN 服务器 |

ICE 的价值在于**这个择优是自动的**：局域网用户走 host 候选，直连速度和现在完全一样；跨网用户自动用 srflx 打洞；打不通的自动退到 relay。不需要用户选、不需要配置判断。

这天然就是需求里的三级降级阶梯，由协议本身实现（详见 [第 6 节](#6-降级阶梯与选路)）。

### 2.2 STUN 打洞的原理

```
被控端(NAT A)                STUN 服务器                 控制端(NAT B)
     │                            │                           │
     │─── Binding Request ───────>│  (出站，NAT A 建表)         │
     │<── "你是 1.2.3.4:50001" ───│                           │
     │                            │<── Binding Request ───────│
     │                            │─── "你是 5.6.7.8:60002" ──>│
     │                                                        │
     │<══════ 通过信令交换上面两个地址（关键！）═══════════════>│
     │                                                        │
     │─── 往 5.6.7.8:60002 发包 ──X (被 NAT B 丢，但 NAT A 建了表)
     │ (被 NAT A 丢，但 NAT B 建了表) X ── 往 1.2.3.4:50001 发包 ──│
     │<═══════════ 双向洞已打通，后续包直达 ═══════════════════>│
```

要点：**第一个包必然被对端 NAT 丢弃，它的作用是在自己这侧 NAT 建表。** 两边都发过一次之后，两边 NAT 都有了对方的表项，通路建立。所以打洞必须**双向同时**进行，这也是为什么必须先通过信令交换候选地址 —— 没有信令，两端互相不知道往哪打。

### 2.3 DTLS/SRTP：加密是强制的

WebRTC 规范**强制加密**，没有关闭选项：

- DataChannel 走 **SCTP over DTLS**（DTLS 1.2+）
- 媒体轨走 **SRTP**，密钥由 DTLS-SRTP 协商（RFC 5764）

这对本工程有一个直接好处：**端到端加密的职责从 rustls 转移到 DTLS，且中继节点看不到明文**。即使流量经过 TURN，TURN 服务器也只是转发 DTLS 密文，看不到屏幕内容和输入事件 —— 这比「自建 WebSocket 中继服务器」的安全模型好得多（后者中继必须终止 TLS，能看到全部明文，除非再套一层应用层加密）。

另一个好处：现有自签名证书的「浏览器警告」问题在 WebRTC 路径上消失了。DTLS 用的是指纹校验（SDP 里的 `a=fingerprint`）而非 CA 信任链，不存在证书告警。但注意 —— 加载前端页面本身仍然需要 HTTPS（见 [2.6 节](#26-前端页面从哪里加载)）。

### 2.4 最容易被忽略的一点：WebRTC 不自带信令

这是整个方案的**核心约束**，必须讲透：

> **WebRTC 规范刻意不定义信令协议。** 它规定了 SDP 的格式、ICE 候选的格式，但「这些字节怎么从 A 送到 B」完全由应用自己解决。

也就是说：**WebRTC 解决了「数据怎么走」，但没解决「两端怎么先找到对方」。** 后者必须靠一个双方都能出站访问的公网会合点 —— 这就是信令服务，是本方案**唯一必须新增的公网组件**。

这里有一个看似矛盾的地方值得澄清：既然还是要一台公网服务器，那和「直接用公网服务器中继」有什么区别？区别在**流量**：

| | 信令服务（WebRTC） | WebSocket 中继服务器 |
| --- | --- | --- |
| 承载内容 | 只有建连阶段的 SDP + ICE 候选，几 KB | **全部视频流**，4 Mbps × 连接数 |
| 带宽成本 | 可忽略，一台 1核1G 小鸡能支撑数千设备 | 每 10 个并发就吃掉 40 Mbps 上行 |
| 建连后是否参与 | **完全退出**（P2P 成功时） | 全程参与，断了连接就断 |
| 能否看到明文 | 看不到（只过 SDP，数据走 DTLS 直连） | 能看到（除非额外加密） |

**成功打洞后信令服务完全退出数据路径** —— 这是 WebRTC 相比纯中继的核心收益。

### 2.5 被控端如何摆脱「必须被连入」

这是架构反转的关键，分解为四步：

```
                      ┌──────────────────────────┐
                      │   信令服务（公网，必需）    │
                      │   wss://signal.xxx.com   │
                      └────▲────────────────▲────┘
             ① 出站注册 +   │                │   ② 出站连接 +
                长连接保活   │                │      请求连接设备
                           │                │
              ┌────────────┴───┐      ┌─────┴──────────┐
              │  被控端(NAT 内) │      │  控制端浏览器    │
              │                │      │                │
              │                │      │                │
              └────────┬───────┘      └───────┬────────┘
                       │  ③ 交换 SDP + ICE     │
                       │<─────────────────────>│
                       │                       │
                       │  ④ P2P 直连建立        │
                       │<═════════════════════>│  (信令退出)
                       │   DTLS 加密的数据通道   │
                       └───────────────────────┘
```

1. **被控端启动后主动出站**连接信令服务（`wss://signal.example.com/register`），上报自己的设备 ID。这条连接**由被控端发起**，NAT 对出站无限制，所以必然成功。
2. 被控端**长期保持**这条连接（心跳保活 + 断线指数退避重连），信令服务由此知道「设备 X 在线，且我有一条能推消息给它的通道」。
3. 控制端要连设备 X 时，也出站连信令服务，发起 `connect` 请求。信令服务通过已有的长连接把请求**推给**被控端。
4. 两端通过信令服务交换 SDP offer/answer 和 ICE 候选，建立 P2P 连接。之后数据走 P2P（或 TURN），不再经过信令。

**反转之后，被控端不再需要任何入站可达性** —— 不需要公网 IP、不需要端口映射、不怕 CGNAT。这就是对 `docs/README.md` 需求基线「同一局域网/可直连」那一条的突破。

### 2.6 前端页面从哪里加载

现有前端是被控端自己用 `ServeDir` 托管的（`web.rs:124`）。被控端不可达时，**这个页面也加载不出来**。所以必须解决前端宿主问题，三个选项：

| 方案 | 说明 | 评价 |
| --- | --- | --- |
| **A. 信令服务同时托管前端** | 信令 crate 用 axum 的 `ServeDir` 托管同一份 `web/`，浏览器访问 `https://signal.xxx.com` | **推荐**。只需一个域名一张证书，且能用正式证书（Let's Encrypt），顺手解决了自签证书告警问题 |
| B. 静态托管（CDN / GitHub Pages） | 前端放对象存储，信令只做 WebSocket | 适合规模化，但多一处部署，当前阶段无必要 |
| C. 控制端本地文件打开 | `file://` 协议 | **不可行**。WebRTC 和 WebCodecs 都要求安全上下文，`file://` 下 API 受限 |

选 A。注意：被控端继续保留自己的 `ServeDir`，局域网直连场景不受影响（见 [8.7 节兼容承诺](#87-对现有直连模式的兼容承诺)）。

---

## 3. 两条数据通道路线的取舍

**结论先行：先 A（RTCDataChannel）后 B（媒体轨）。** 理由见 [3.4 节](#34-判断依据与推荐)。

### 3.1 路线 A：RTCDataChannel 承载现有帧

把现有 WebSocket 的两种消息原样搬到 DataChannel 上：二进制 0xAB 帧走一条 channel，JSON 控制消息走另一条（或同一条按类型分流）。

**改动面评估（已核实代码位置）**：

| 模块 | 改动 | 量级 |
| --- | --- | --- |
| `pipeline.rs` | **零改动**。`serialize_packet()` 产出的格式不变 | 0 |
| `encoder.rs` | **零改动** | 0 |
| `protocol.rs` | **零改动**（`ServerMsg`/`ClientMsg` 原样复用），只新增信令消息枚举 | +新增 |
| `capture/`、`input/`、`clipboard.rs` | **零改动** | 0 |
| `web/app.js` | `processVideoPacket()`（app.js:425）、消息分发、输入上行**逻辑全部复用**，只换传输层：`ws.send()` → `dc.send()`，`ws.onmessage` → `dc.onmessage` | 小 |
| `web.rs` | 需要把会话循环从「绑定 `WebSocket`」抽象为「绑定一个可收发的 transport」 | **中（主要工作量）** |
| 新增 | `webrtc.rs`（PeerConnection 管理）、`signaling.rs`（信令客户端） | 新文件 |

关键优势：**WebCodecs 解码路径一字不改**。`app.js:425-500` 那套「frame_type 3 缓存 SPS/PPS、frame_type 1 拼成 `[SPS][PPS][IDR]` key chunk、frame_type 2 作 delta」的逻辑是本工程调过的、能跑的，保留它等于保留了确定性。

**web.rs 的抽象改造是主要工作量。** 现在 `handle_socket()`（web.rs:168-539）里 `socket` 是具体的 `axum::extract::ws::WebSocket`，`tokio::select!` 直接 `socket.recv()` / `socket.send()`。要支持两种传输，需要抽出一个 trait（形如 `trait Transport { async fn recv() -> Option<Msg>; async fn send(&mut self, msg: Msg) }`），让 WebSocket 和 DataChannel 都实现它，会话循环改为泛型或 `Box<dyn>`。这是机械性重构，风险可控，但要动 371 行的核心循环，**必须有回归验证**。

#### SCTP 的四个必须评估的点

**(1) 吞吐**

SCTP over DTLS over UDP 的开销比 WebSocket over TLS over TCP 略高（多一层 SCTP 头 + DTLS 记录头，每包约 30~40 字节），但对本场景（4 Mbps 量级）不是瓶颈。社区实践中 DataChannel 跑到几十 Mbps 是常态。**待实测**：本工程实际帧率下的 CPU 与延迟，尤其是 webrtc-rs 的 SCTP 实现性能（Rust 实现成熟度不及 libwebrtc 的 C++ usrsctp）。

**(2) 拥塞控制 —— 这是方案 A 最大的隐患**

DataChannel 默认是**可靠有序**（reliable ordered）传输，等价于 TCP 语义。这意味着**丢包时会重传并阻塞后续数据** —— 对实时视频是灾难：网络抖动一下，视频会「卡住然后快进」，而不是「丢几帧继续」。

解决办法：创建 channel 时配置为**部分可靠 + 无序**：

```js
// 视频通道：不重传、不保序，丢了就丢（延迟优先）
const videoDc = pc.createDataChannel('video', {
    ordered: false,
    maxRetransmits: 0,
});
// 控制通道：必须可靠有序（输入事件不能丢、不能乱序）
const ctrlDc = pc.createDataChannel('control');  // 默认 reliable ordered
```

**两条 channel 分流是必须的，不是可选的** —— 视频要「宁丢不等」，输入事件要「绝不能丢」（丢一个 key_up 会导致按键粘滞）。

即便如此，SCTP 的拥塞控制仍然是「尽力填满带宽」而非「按视频质量调节」，它不会告诉应用层「现在该降码率了」。方案 A 下**码率自适应只能靠自己实现**（比如用 DataChannel 的 `bufferedAmount` 做反压信号，见下），这是它相比方案 B 的根本短板。

**(3) 大 I 帧分片**

已核实：本工程是**逐 NAL 下发**的。`pipeline.rs:184-189` 对 `encoder.encode()` 返回的每个 `EncodedPacket` 单独 `serialize_packet()` 并 `tx.send()`，`web.rs:227` 再逐个 `Message::Binary` 发出。所以单条消息是「16 字节头 + 一个 NAL」，不是整帧。

消息大小约束（已核实）：Chrome 的 `maxMessageSize` 实测报错信息为 `Message too large (can send a maximum of 262144 bytes)`，即 **256 KiB**；规范建议的互操作安全值是 **64 KiB**；社区经验是**按 16 KiB 分片吞吐最优**（[Mozilla WebRTC 博客](https://blog.mozilla.org/webrtc/large-data-channel-messages/) 对此有专门讨论）。

对本工程的影响评估：
- P 帧 NAL：桌面内容通常几 KB ~ 几十 KB，**无压力**。
- SPS/PPS：几十字节，无压力。
- **IDR NAL：1080p 单片编码下可达 100~300 KB**，有突破 256 KiB 的风险。openh264 默认不开多 slice，所以一个 IDR 就是一个大 NAL。

结论：**必须实现应用层分片**，不能赌 IDR 不超限。两种做法：
- **(a) 传输层分片（推荐）**：在 0xAB 头之外再套一层分片头（`chunk_index` / `chunk_total` 或 `is_last` 标志位），发送端按 16 KiB 切，接收端重组后再喂给现有 `processVideoPacket()`。好处是现有帧格式和解码逻辑完全不动，分片逻辑是独立的一层。**现有 16 字节头里有 2 字节 `reserved`（`pipeline.rs:239`），正好可以用来放分片信息** —— 这是个现成的扩展位。
- (b) 编码器层切片：给 openh264 配多 slice，让每个 NAL 天然小于阈值。好处是不加协议层，坏处是要动 `encoder.rs`，且 slice 边界会轻微影响压缩率和画质。

选 (a)。理由：不碰已经调通的编码路径，且分片层对 WebSocket 直连模式也无害（可以统一走同一条路径，减少两套代码的分叉）。

**(4) 丢帧策略与背压**

现状（已核实）：`pipeline.rs` 用 `tokio::sync::broadcast`，容量硬编码 8（`web.rs:82`）；慢消费者触发 `RecvError::Lagged(n)`，`web.rs:233-236` 的处理是记一条 debug 日志然后 `continue` —— 即**自动丢弃最旧的帧**。这是正确的实时流策略，但有个缺陷：**丢帧后没有请求关键帧**，所以可能连续若干帧解码失败（P 帧依赖的参考帧被丢了）。

方案 A 下需要新增的机制：
- 用 `RTCDataChannel.bufferedAmount` 作为真实的网络背压信号（`broadcast` 的 Lagged 只反映「消费者读得慢」，反映不出「网络发不出去」）。发送前检查 `bufferedAmount`，超阈值则**主动丢掉这一帧**而不是排队，避免缓冲区无限增长导致延迟雪崩。
- 丢帧之后应当触发一次关键帧请求（复用现有 `state.keyframe_tx`，`web.rs:494` 已有该路径），让画面尽快恢复。**这个改进对现有 WebSocket 模式同样有价值，可以独立先做。**

### 3.2 路线 B：WebRTC 媒体轨（RTP/SRTP）

把 H.264 码流作为标准媒体轨推给浏览器，前端用 `<video>` 元素原生播放。

**收益**：
- 浏览器原生解码，不依赖 WebCodecs（**兼容性更好** —— WebCodecs 在 Safari 上支持较晚，而媒体轨是 WebRTC 基础能力）。
- **自带 NACK / PLI / FIR**：丢包自动重传、解码失败自动请求关键帧，不用自己实现。
- **自带带宽估计（REMB / TWCC）**：能拿到「当前网络能承载多少码率」，可以据此动态调编码器码率 —— 这是方案 A 拿不到的信息。
- 自带 jitter buffer、A/V 同步（如果以后加音频）。
- 浏览器 `chrome://webrtc-internals` 有完整统计面板，**调试体验远好于自己撸的协议**。

**代价（改动面明显更大）**：

| 工作项 | 说明 |
| --- | --- |
| RTP 打包 | H.264 要按 RFC 6184 做 payload 化（Single NAL / FU-A 分片 / STAP-A 聚合），MTU 约 1200 字节。好消息：**webrtc crate 自带 H.264 payloader**，不用手写（见 [4.3 节](#43-h264-支持已核实)） |
| 时间戳 | 要把 `timestamp_us` 换算成 90 kHz RTP 时钟 |
| PLI/FIR 对接 | 浏览器发 PLI 时要转成 `keyframe_tx.send(())`。现有路径已存在（`web.rs:491-497`），对接成本低 |
| 码率自适应对接 | 要从带宽估计读目标码率，再调 openh264。**这是真正的新工作**：现有 `H264Encoder`（`encoder.rs:36`）的码率在 `new()` 时固定，没有运行期 `set_bitrate` 接口，需要新增（openh264 的 `SetOption(ENCODER_OPTION_BITRATE)` 支持动态调，但 `openh264` crate 0.6 是否暴露该接口**待核实**） |
| 前端重构 | `app.js` 的 WebCodecs 解码路径（约 100 行）整段废弃，换成 `<video srcObject>`。**现有调通的逻辑作废** |
| 控制通道 | 媒体轨只能传视频，JSON 控制消息和输入事件**仍然需要一条 DataChannel** —— 所以方案 B 是「媒体轨 + DataChannel」，不是「纯媒体轨」 |

一个隐性风险：**浏览器对 H.264 媒体轨的 profile 支持有约束**。浏览器通常要求 Constrained Baseline（`profile-level-id=42e01f`）才保证硬解，而 openh264 默认输出的 profile 需要核实是否匹配（`app.js:387-403` 的 `configureDecoderFromSps` 是从 SPS 现读 profile/level 的，说明当前并未强制某个 profile）。**待实测**。另外 SDP 里的 `packetization-mode` 协商（0 单 NAL / 1 允许 FU-A）也要对齐。

### 3.3 一个不推荐的变体：媒体轨 + insertable streams

理论上可以用媒体轨拿 NACK/BWE，又用 Encoded Transform（insertable streams）绕过浏览器解码器。实测复杂度高、浏览器支持参差（Firefox 支持不完整），**不建议在本工程尝试**。

### 3.4 判断依据与推荐

**推荐：先 A 后 B。**

判断依据（按权重排序）：

1. **风险隔离**。本次要解决的是**穿透**这一个问题。方案 A 让「穿透」成为唯一变量 —— 视频编码、解码、输入注入全链路保持不变，出问题必然在网络层，排查范围小。方案 B 同时改变穿透方式和编解码路径，两个变量叠加，一旦画面不对就很难判断是 RTP 打包错、profile 不匹配，还是打洞本身有问题。
2. **可复用的确定性资产**。`app.js` 的 WebCodecs 解码逻辑（SPS 触发 configure、IDR 内联 SPS/PPS、首关键帧前丢 P 帧）是已经踩过坑调通的。方案 B 要把它整段丢掉重来。
3. **验证路径短**。方案 A 可以分两步独立验证：先只换传输（局域网内用 WebRTC host 候选跑通，排除打洞变量），再加 STUN/TURN。方案 B 没法这样切。
4. **方案 B 的收益主要体现在弱网**，而弱网优化本就是 `docs/05-实施计划.md` 里 M5「性能优化（按需）」的范畴，不是当前要解决的问题。
5. **方案 A 不是死路**。信令、ICE、TURN、PeerConnection 管理这些基础设施，A 和 B **完全共用**。做完 A 再上 B，是在已有基础上换数据面，不是推倒重来。沉没成本只有「分片层」和「前端传输适配」两块，都不大。

什么时候该上 B：出现以下任一信号时启动（量化判据，避免凭感觉）：
- 实测弱网（丢包 > 2%）下方案 A 的画面卡顿不可接受，且调 `maxRetransmits`/分片大小无法改善；
- 需要支持 Safari 或其他 WebCodecs 支持不佳的浏览器；
- 需要真正的码率自适应（用户网络波动大，固定码率体验差）。

---

## 4. Rust 侧选型落地

> 本节所有版本号与依赖关系均为 **2026-10 实查** crates.io API / docs.rs / GitHub README 所得，非凭记忆。原始出处见 [附录 A](#附录-a已核实事实与推测的边界)。

### 4.1 webrtc-rs 的真实版本与维护状态（已核实）

| 项 | 事实 |
| --- | --- |
| crate 名 | `webrtc`（[crates.io/crates/webrtc](https://crates.io/crates/webrtc)） |
| **最新版本** | **0.21.1**，发布于 **2026-10-11** |
| 版本总数 | 72（仅 0.7.0 被 yank） |
| 维护者 | rainliu（Rusty Rain），GitHub `webrtc-rs/webrtc` |
| 社区热度 | 5.2k stars / 531 forks / 3820 commits / 5 open issues / 6 open PRs |
| License | MIT / Apache-2.0（与本工程无冲突） |
| 自述状态 | README 明确 0.21.0 为「current and recommended version」，仍是 **pre-1.0**：「while on 0.x, minor version bump may include breaking changes」。1.0 剩余工作跟踪在 issue #836 |

**维护状态结论：活跃。** 2026 年 6 月起进入密集发布期（0.20 的 alpha/beta/rc 列车 6 月下旬集中发布，7 月逐周 rc，0.21 系列 8 月 alpha → 9 月正式 → 10 月 patch）。同时维护多条线：0.17.2（2026-07-20）、0.20.5（2026-09-04）与 0.21 的 pre-release 并行发布，说明旧线仍在收补丁。

**一个必须知道的重大变化**：0.17.x → 0.20.0 是一次**重写级改动**，不是普通升级。核实到的证据：
- edition 从 2021 跳到 **2024**；
- `webrtc` crate 自身代码量从约 20783 行**缩减到约 3235 行** —— 协议栈被整体移到了新的 sans-IO 核心 crate `rtc`；
- feature 从 openssl/pem 一套换成 runtime 选择（`runtime-tokio` / `runtime-smol` / `runtime-mock`）；
- 描述从「A pure Rust implementation of WebRTC API」改为「Async-friendly WebRTC implementation in Rust」；
- 0.21.0 又新增了加密后端 feature（`crypto-ring` / `crypto-aws-lc-rs`）。

现在的架构是：`rtc`（sans-IO 协议核心，自称 95%+ W3C API 兼容）+ `webrtc`（薄异步层：`PeerConnection` 句柄 + 后台 `PeerConnectionDriver` 持有 socket + `Runtime` trait 抽象 timer/spawn/socket）。

**对本工程的实际影响：网上绝大多数 webrtc-rs 教程、示例、StackOverflow 答案都是旧 API（`api::media_engine::MediaEngine` 那套）**，在 0.21 上不适用（顶层已无 `api` 模块）。好消息是官方仓库带 **39 个可运行示例**，覆盖 data channel、media playback、simulcast、ICE restart、insertable streams —— 这是唯一可靠的参考来源。

### 4.2 加密后端：与本工程 ring 路线的兼容性（核心风险点，已核实）

本工程在 `Cargo.toml` 里刻意统一 ring，注释写明「统一使用 ring 后端，避免 aws-lc-sys 的 C 编译/链接负担」—— 这对龙芯/交叉编译是硬需求（aws-lc-sys 要编 C 代码，交叉编译链踩坑概率极高）。

**核实结果：webrtc 0.21.x 与此完全兼容。**

逐层核实的依赖链：

```
webrtc 0.21.1
  features: default = ["runtime-tokio", "crypto-ring"]   ← ring 是默认
            crypto-ring       → rtc/crypto-ring
            crypto-aws-lc-rs  → rtc/crypto-aws-lc-rs     ← opt-in，不开就不拉
  deps (仅 11 个): async-broadcast, async-channel, async-trait, bytes,
                   event-listener, futures, log, quinn-udp, rtc,
                   smol(optional), tokio(optional)
  └── rtc 0.21.1  (default-features = false)
        deps (28 个): rustls ^0.23.27 (default-features=false, 仅 "std")
                      rcgen ^0.14.8  (default-features=false, 仅 "pem")
                      pem, x509-parser, hex, rand, sansio, serde, url, unicase
                      + 18 个 rtc-* 子 crate (ice/dtls/srtp/sctp/stun/turn/
                        sdp/rtp/rtcp/crypto/datachannel/interceptor/mdns/
                        media/shared)
        └── rtc-crypto 0.21.1
              required: rand, subtle, thiserror, zeroize
              optional: ring ^0.17.14          ← crypto-ring 走这条
                        aws-lc-rs ^1.17.3      ← 不开就不编译
                        aes, ccm, ctr, hmac, md-5, sha1 (RustCrypto 路径)
```

**全依赖链无 openssl / openssl-sys 的非可选依赖。** `openssl` 只作为 rcgen 在 unix 下的 **dev-dependency** 出现，不进生产构建。

版本对齐情况（与本工程 `Cargo.lock` 实际值逐一比对）：

| crate | rtc 要求 | 本工程 Cargo.lock 现值 | 结论 |
| --- | --- | --- | --- |
| `rustls` | ^0.23.27 | **0.23.40** | 同 major，cargo 自动 unify，feature 取并集（ring+logging+std+tls12）。**无冲突** |
| `ring` | ^0.17.14 | **0.17.14** | 完全吻合 |
| `tokio` | ^1.52.3 | **1.52.3** | 完全吻合 |
| `rcgen` | ^0.14.8 | **0.13.2** | **semver 不兼容，两份并存编译** |

关于 rcgen 分叉的风险评估：cargo 允许 0.13 与 0.14 共存（不同 major 视作不同 crate），代价只是多一份编译量。**不会引入 aws-lc**：已核实 rcgen 0.14.8 的 `ring` 和 `aws-lc-rs` 都是 **optional**，而 rtc 对 rcgen 只开了 `pem` 一个 feature，两个后端都没开。且因为 major 不同，**本工程 rcgen 0.13 的 `ring` feature 与 rtc 的 rcgen 0.14 之间不发生 feature unification** —— 互不污染。

可选的收敛动作：把本工程的 `rcgen` 从 0.13 升到 0.14（API 变动很小，主要是 `KeyPair` 相关），消除重复编译。**非必需，建议作为独立的小改动单独评估，不要和穿透功能混在一个 PR 里。**

还有一个加密后端的额外保障（README 实查）：`crypto-ring` 与 `crypto-aws-lc-rs` 是 **additive（可加性）** 的 —— 两个都开也只是都编译进去，**ring 仍然是默认选择**，README 的原话是一个传递依赖「can't silently swap it」。而且后端选择是**按 PeerConnection 粒度**通过 `SettingEngine` 指定，不是进程全局。这意味着即使将来某个间接依赖开了 aws-lc feature，也不会悄悄改变本工程的加密后端 —— 只会多编一份（届时可用 `cargo tree -i aws-lc-rs` 定位并处理）。

**建议的依赖声明（显式关死所有不需要的东西）**：

```toml
# 注意：webrtc 0.21.x 是 edition 2024，要求 rustc >= 1.85
webrtc = { version = "0.21", default-features = false, features = [
    "runtime-tokio",   # 复用现有 tokio 运行时
    "crypto-ring",     # 与本工程 rustls/rcgen 统一 ring，避开 aws-lc-sys
] }
```

显式 `default-features = false` 再逐项加回，是为了**防止上游将来改 default 时把 aws-lc 悄悄带进来**（虽然按上述 additive 设计不会换掉 ring，但能少编一份 C 代码总是好的）。

### 4.3 H.264 支持（已核实）

查 `docs.rs/rtc/0.21.1` 的 `peer_connection::configuration::media_engine` 模块，**`MIME_TYPE_H264` 常量存在**，文档描述为 "H.264 video codec MIME type"。该模块共 15 个 MIME 常量：AV1、FLEX_FEC、FLEX_FEC03、G722、**H264**、HEVC、OPUS、PCMA、PCMU、RTX、SMPTE336M、TELEPHONE_EVENT、ULP_FEC、VP8、VP9。

`MediaEngine::register_default_codecs()` 的文档示例注明默认注册集包含 "Opus, G722, PCMU, PCMA, VP8, VP9, **H264**, AV1"。

**所以方案 B 所需的 H.264 媒体轨能力是现成的，不用自己写 RTP payloader。** 不过要注意：
- 官方示例是 VP8/VP9 导向的（`save-to-disk-vpx` / `play-from-disk-vpx`），**没有 H.264 示例** —— 实际对接时参考资料较少。
- `media_stream` 模块提供 `TrackLocalStaticSample`（喂编码后的帧，库内部负责 payload 化）和 `TrackLocalStaticRTP`（自己打好 RTP 包）。方案 B 应该用前者。
- **待实测**：`TrackLocalStaticSample` 对 Annex-B 输入的处理（现有 `encoder.rs` 输出 Annex-B，含起始码），以及 openh264 输出的 profile 能否被浏览器硬解。

### 4.4 DataChannel 支持（已核实）

`rtc::data_channel` 模块提供 `RTCDataChannel`、`RTCDataChannelInit`、`RTCDataChannelMessage`，API 形如 `pc.create_data_channel("name", Some(init))` + `send_text`/`send`。`webrtc` 0.21.1 的 `data_channel` 模块暴露 `DataChannel` trait，覆盖 `send`、`try_send` 和 **send-buffer back-pressure**。

0.21 的 changelog 里有两条与方案 A 直接相关的修复（README 实查）：
- 可靠 channel 上**用背压代替静默丢消息**（之前会悄悄丢，这对控制通道是严重 bug）；
- SCTP T1 RTO 按 RFC 9260 修正为 1s；
- inbound `DATA_CHANNEL_OPEN` 在 association 建立前被正确保留。

这几条说明 SCTP 实现**最近才在修正确性问题** —— 既是好事（在修）也是风险信号（之前有坑）。**强烈建议锁 0.21.1 或更新，不要用 0.20.x 及更早。**

### 4.5 ICE / STUN / TURN 配置（已核实）

`RTCIceServer` 位于 `rtc::peer_connection::transport`，经 `RTCConfigurationBuilder::with_ice_servers` 传入。feature 列表声明支持 "NAT traversal with STUN/TURN"，且 `rtc-turn` 是实打实的依赖。Trickle ICE 与 ICE restart 均声明支持（trickle ICE 对本场景很重要 —— 可以边收集候选边尝试连接，显著缩短建连时间）。

**待核实**：docs.rs 页面上的示例都是 `urls` + `..Default::default()`，**TURN 的 username/credential 字段没有在示例中出现**。实现时需要查 `RTCIceServer` 的完整字段定义确认凭据怎么传。

### 4.6 依赖体量评估

当前 `Cargo.lock` 共 **249 个 package**（已核实）。引入 `webrtc` 后新增：`rtc` + 18 个 `rtc-*` 子 crate + `sansio` + `quinn-udp` + `async-broadcast` + `async-channel` + `event-listener` + `x509-parser` + `yasna` + 第二份 rcgen 及其依赖树。

**估算新增 60~90 个 package（推测，未实测）。** 准确数字需要 `cargo tree` 或 `cargo add --dry-run`，本轮按规则不执行构建。

编译时间影响：新增的都是纯 Rust crate，**无新增 C 编译单元**（这是选 ring 路线的直接收益）。对交叉编译友好度无负面影响，唯一的新约束是 rustc 版本。

### 4.7 备选方案 str0m：不推荐（已核实原因）

[str0m](https://crates.io/crates/str0m) 是另一个 sans-IO 的 Rust WebRTC 实现，由 Martin Algesten 维护，最新 0.24.1（2026-10-03），38 个版本，累计下载 278 万、近期 119 万 —— **成熟度和实际使用量都很可观**，技术上是个好库。

**但它与本工程的加密后端路线直接冲突**：

```
str0m 0.24.1 features:
  default = ["aws-lc-rs", "examples"]     ← 默认就是 aws-lc-rs
  可选后端: openssl (+vendored), rust-crypto, apple-crypto, wincrypto
  ──> 没有 ring 后端
```

已核实 str0m **不提供 ring 后端**。可选项里 `rust-crypto`（纯 Rust RustCrypto）理论上也能避开 C 编译，但：
1. 偏离了本工程已经统一的 ring 路线，等于引入第三套加密实现；
2. RustCrypto 的 AES-GCM 性能通常不及 ring/aws-lc（无专门的汇编优化路径），对视频流的加解密开销敏感；
3. str0m 同样是 edition 2024 / rustc ≥ 1.85，在这一点上没有优势。

另外 str0m 是**纯 sans-IO**，socket 和 timer 全由调用方管理，接入成本比 webrtc-rs 的 `runtime-tokio` 薄封装高一截。

**结论：选 webrtc 0.21.x。** str0m 作为备选记录在案 —— 如果 webrtc-rs 的 SCTP 实现在实测中暴露严重问题，再评估 str0m + `rust-crypto` 后端。

### 4.8 信令服务用什么写

**推荐：workspace 内新增独立 crate `crates/signal`，用 axum 0.7 写。**

理由：
- **技术栈零学习成本**。被控端已经在用 axum 0.7 + `ws` feature + tokio（`Cargo.toml` 实查），信令服务要的「WebSocket 长连接 + 房间路由 + 静态文件托管」正好是 axum 的舒适区，`ServeDir` 还能顺手托管前端（见 [2.6 节](#26-前端页面从哪里加载)）。
- **可共享协议定义**。信令消息类型可以放在共享位置，被控端和信令服务用同一份 `serde` 定义，避免两边 JSON 字段写错。
- **依赖极轻**。信令服务不需要 openh264、x11rb、windows、enigo、arboard 这些重依赖，独立 crate 能把依赖面压到最小（axum + tokio + serde + tracing 即可），**编译快、镜像小、公网暴露面小**。
- 部署简单：单二进制 + systemd / Docker，配 Caddy 或 nginx 做 TLS 终止（或直接复用现有 `tls.rs` 的 rustls 能力）。

为什么**不**用现成的开源信令服务：本方案的信令需要和设备 ID 模型、认证边界（[5.5 节](#55-与现有-argon2-认证的串联)）、限流策略（[5.6 节](#56-滥用风险与限流)）深度耦合，自己写约 500~800 行就能覆盖，比改造一个通用信令服务更省事。

workspace 改动：

```toml
# 根 Cargo.toml
[workspace]
members = ["crates/server", "crates/signal"]
```

> 注意：根 `Cargo.toml` 现有 `[patch.crates-io]` 把 `openh264-sys2` 指向 `vendor/`。patch 是 workspace 全局的，但 `crates/signal` 不依赖 openh264，**不受影响**（已核实 patch 只影响实际依赖该 crate 的成员）。

---

## 5. 信令与会合服务设计

### 5.1 设备 ID 模型

不用「房间」模型（房间是多人会议的概念），用**设备注册 + 定向呼叫**模型，更贴合远程控制的 1 对 1 语义。

| 概念 | 说明 |
| --- | --- |
| `device_id` | 被控端的稳定标识。**首次启动生成 UUID v4 并持久化**到配置目录（`uuid` crate 已是现有依赖，`Cargo.toml` 实查），之后每次启动复用。不要用主机名或 MAC —— 前者会重名，后者涉及隐私且虚拟网卡会变 |
| `display_code` | 给人看的短码，如 `472-915-308`（9 位数字分组）。由信令服务在注册时分配并维护 `display_code → device_id` 映射。**理由**：UUID 没法口头传达或手输，而远程协助场景就是「被控端的人把号码报给控制端的人」 |
| `device_name` | 可选的人类可读名（如「办公室台式机」），被控端配置项，仅用于展示 |
| `register_token` | 被控端向信令服务证明「我有权注册」的凭据。**与远程控制的访问密码完全无关**（见 [5.5 节](#55-与现有-argon2-认证的串联)） |

`display_code` 的安全设计（重要）：
- **短码必须可轮换**。建议默认每次启动重新分配，或配置为固定（牺牲安全性换便利）。
- 短码是**公开可枚举**的（9 位数字约 10^9 空间，但分配出去的是稀疏子集）。所以**短码绝不能是认证凭据** —— 知道短码只能"发起呼叫"，连接后仍必须过 argon2 密码。这一点和 TeamViewer 的 ID + 密码模型一致。
- 信令服务必须对「按短码查设备」做严格限流，否则可被枚举出所有在线设备（见 [5.6 节](#56-滥用风险与限流)）。

### 5.2 被控端出站长连接：保活与重连

被控端新增一个 tokio task，维护到信令服务的 WebSocket 长连接。

**保活**：
- 应用层心跳：每 **30s** 发一次 `{"type":"heartbeat"}`，信令服务回 `{"type":"heartbeat_ack"}`。
- **为什么不能只靠 WebSocket 的 ping/pong**：很多中间设备（NAT、负载均衡、运营商网关）对空闲 TCP 连接有 60s~300s 的回收策略，而且**连接被静默回收时本端往往感知不到**（没有 FIN，只是后续包无响应）。应用层心跳 + 超时判定是唯一可靠的探活方式。
- 超时判定：发出心跳后 **10s** 未收到 ack 即判定连接已死，主动关闭并重连。

**重连（指数退避 + 抖动）**：

```
重连间隔: 1s → 2s → 4s → 8s → 16s → 32s → 60s (上限，之后固定 60s)
每次实际等待 = 基础间隔 × (0.5 + random(0.0..1.0) × 0.5)   // 抖动，避免惊群
```

抖动（jitter）的必要性：如果信令服务重启，所有被控端会在同一时刻掉线，不加抖动会在同一秒集体重连，把刚起来的服务再打挂。

**与现有启动流程的关系**：这个 task 必须是**完全非阻塞**的 —— 信令服务不可达时，被控端的本地 HTTPS 服务必须照常工作。实现上即 `tokio::spawn` 一个独立 task，内部错误只记日志，**绝不 propagate 到 `main()`**。这是 [8.7 节兼容承诺](#87-对现有直连模式的兼容承诺)的技术保障。

### 5.3 控制端如何发起

```
控制端浏览器                信令服务                  被控端
     │                         │                        │
     │                         │<═══ 已有长连接 ════════>│
     │                         │   (register 完成)       │
     │                         │                        │
     │── ws 连接 + hello ─────>│                        │
     │── connect(display_code) ─>│                       │
     │                         │─── incoming_call ─────>│
     │                         │   (含 call_id)          │
     │                         │                        │
     │                         │<── call_accepted ──────│
     │<── call_accepted ───────│   (含 call_id)          │
     │                         │                        │
     │     ┌───────────── SDP / ICE 交换（双向透传）─────────┐
     │<────┤  offer / answer / ice_candidate            ├───>│
     │     └────────────────────────────────────────────┘    │
     │                                                       │
     │<══════════ P2P DataChannel 建立 (DTLS 加密) ═════════>│
     │                                                       │
     │──── auth(argon2 密码) ───────────────────────────────>│
     │<─── auth_ok / auth_fail ─────────────────────────────│
     │                      (信令服务不参与，看不到密码)
```

**谁做 offer**：由**控制端（浏览器）发 offer**，被控端回 answer。理由：浏览器的 `createOffer()` 会自动生成完备的媒体/数据能力描述，而让 Rust 侧生成 offer 需要手工构造更多 SDP 细节。被控端作为 answerer 只需对浏览器的 offer 做应答，实现更简单。

### 5.4 信令消息格式草案

延续现有 `{"type": ...}` 的 snake_case 风格（与 `protocol.rs` 的 `#[serde(tag = "type", rename_all = "snake_case")]` 一致），便于复用同一套序列化约定。

**被控端 ⇄ 信令服务**

```jsonc
// → 被控端注册（出站连接建立后第一条）
{"type":"register","device_id":"550e8400-...","device_name":"办公室台式机",
 "token":"<register_token>","version":"1.0"}

// ← 注册成功，下发短码
{"type":"registered","display_code":"472-915-308","expires_in":86400}

// ← 注册失败
{"type":"register_failed","reason":"invalid_token"}   // 或 device_id_conflict / rate_limited

// → 心跳 / ← 心跳响应
{"type":"heartbeat"}
{"type":"heartbeat_ack"}

// ← 有控制端呼入
{"type":"incoming_call","call_id":"c-7f3a...","peer_hint":"203.0.113.9"}

// → 被控端接受（可基于 exclusive_control 或 max_sessions 拒绝）
{"type":"call_accepted","call_id":"c-7f3a..."}
{"type":"call_rejected","call_id":"c-7f3a...","reason":"busy"}
```

`peer_hint` 的用途：给被控端日志记录「谁在尝试连接」，便于审计。**不作为认证依据**（可伪造，且过 TURN 时无意义）。

**控制端 ⇄ 信令服务**

```jsonc
// → 按短码发起呼叫
{"type":"connect","display_code":"472-915-308"}

// ← 对端已接受，可以开始 SDP 交换
{"type":"call_accepted","call_id":"c-7f3a...",
 "ice_servers":[                                    // 服务端下发，避免前端硬编码
   {"urls":["stun:stun.l.google.com:19302"]},
   {"urls":["turn:turn.example.com:3478?transport=udp"],
    "username":"1730000000:user","credential":"<hmac-base64>"}
 ]}

// ← 呼叫失败
{"type":"call_failed","reason":"device_offline"}  // 或 invalid_code / busy / rate_limited
```

**SDP / ICE 交换（双向，信令服务只做透传）**

```jsonc
{"type":"offer","call_id":"c-7f3a...","sdp":"v=0\r\no=- ..."}
{"type":"answer","call_id":"c-7f3a...","sdp":"v=0\r\no=- ..."}
{"type":"ice_candidate","call_id":"c-7f3a...",
 "candidate":"candidate:1 1 udp 2113937151 192.168.1.5 50001 typ host",
 "sdp_mid":"0","sdp_mline_index":0}
// ICE 候选收集完毕（可选，用于前端提示「正在连接」状态结束）
{"type":"ice_complete","call_id":"c-7f3a..."}
// 任一端主动结束
{"type":"bye","call_id":"c-7f3a...","reason":"user_closed"}
```

**设计要点**：
- 信令服务对 `sdp` / `candidate` 字段**只做长度校验与透传，不解析内容**。这降低了信令服务的攻击面（不用担心 SDP 解析漏洞），也让它对 WebRTC 版本变化免疫。
- 所有消息带 `call_id`，支持同一设备的多个并发会话（对应现有 `exclusive_control` 配置项，`config.rs:84`）。
- `register_token` 只在 register 时出现一次，之后的消息不带凭据 —— 靠 WebSocket 连接本身的状态。
- **TURN 凭据由信令服务动态下发**，而不是硬编码在前端或配置里。用 coturn 的 `use-auth-secret` 模式：`username = <过期时间戳>:<用户标识>`，`credential = base64(HMAC-SHA1(username, shared_secret))`。这样凭据有时效（建议 10 分钟），泄露了也用不久，且不用在客户端存长期密钥。这是 coturn 的标准 REST API 认证方案（RFC 7635 思路）。

### 5.5 与现有 argon2 认证的串联

**核心原则：信令服务绝不持有被控端的访问密码或其哈希。**

分层认证模型：

| 层 | 凭据 | 验证方 | 作用 |
| --- | --- | --- | --- |
| **第 1 层：注册权限** | `register_token` | 信令服务 | 防止任意人往信令服务塞垃圾设备。可以是预共享密钥、或信令服务签发的设备证书 |
| **第 2 层：呼叫权限** | `display_code` | 信令服务 | 只决定「允许把呼叫请求转给哪个设备」。**不是认证**，仅是寻址 |
| **第 3 层：传输加密** | DTLS 指纹（SDP 内） | WebRTC 协议栈 | 防中间人。指纹经信令传递，所以信令服务理论上能做 MITM（见下方风险） |
| **第 4 层：访问控制** | **argon2 密码**（现有） | **被控端自己** | **唯一真正的授权关口**。P2P 通道建立后，走现有 `ClientMsg::Auth` → `Authenticator::verify()` 流程 |

**第 4 层完全复用现有代码。** `session.rs:82` 的 `Authenticator::verify()` 和 `web.rs:341-435` 的认证状态机一字不改 —— 只是输入从 WebSocket 变成 DataChannel。这是方案 A「复用现有协议」的直接红利：**穿透能力的引入不削弱任何现有安全属性**。

**信令服务被攻破的影响面（诚实评估）**：
- 攻击者**能**：知道哪些设备在线、短码与设备的映射、发起呼叫、看到 IP 地址（srflx 候选里有公网 IP）。
- 攻击者**不能**：直接控制任何设备 —— 不知道 argon2 密码就过不了第 4 层。
- 攻击者**理论上能做 MITM**：因为 DTLS 指纹经信令传递，恶意信令可以替换双方的指纹，插在中间。**这是所有基于信令的 WebRTC 方案的固有弱点。**

  缓解措施（按成本排序）：
  1. **（低成本，建议 M2 即做）** 被控端把自己的 DTLS 指纹**也**通过「用 argon2 密码派生的密钥签名」的方式传递，控制端用密码验签。因为密码是端到端共享的秘密而信令服务不知道，验签能力就把 MITM 挡在外面。
  2. （中成本）给用户展示指纹短哈希（类似 SSH 的 host key fingerprint），首次连接时确认并记住（TOFU 模型）。
  3. （高成本）引入 PKI，被控端持设备证书。

  **M1 阶段可以接受风险 1 未实现**（因为相比当前「自签证书 + 浏览器点通过」的现状，安全性并未降低），但 M2 必须补上。

**暴力破解防护需要改造（已核实的关键问题）**：

现有 `BruteForceGuard` 按 `peer_addr.ip()` 计数封禁（`web.rs:346` / `web.rs:415`）。走 WebRTC 后：
- P2P 直连时，ICE 能拿到对端的真实地址，封禁仍有效；
- **走 TURN 中继时，所有连接在被控端看来都来自 TURN 服务器的同一个 IP** —— 按 IP 封禁会把「所有经 TURN 的用户」连坐封禁，而攻击者换个 TURN 分配就绕过了。

改造方案（必须做，否则是安全回退）：
1. 按 `call_id` / 会话维度独立计数，而非只按 IP；
2. 新增**全局失败速率限制**（如「全部来源合计每分钟最多 N 次失败认证」）作为兜底，这对中继场景是唯一有效的防线；
3. 信令服务侧对「同一控制端连接发起的呼叫次数」限流（见下节），把攻击在更外层拦住；
4. 保留现有 `AUTH_FAIL_DELAY`（800ms 固定延迟，`web.rs:27`）—— 这个机制与传输层无关，继续有效。

### 5.6 滥用风险与限流

信令服务是**公网暴露的无认证入口**（控制端侧在呼叫前没有凭据），必须按「会被扫、会被刷」来设计。

| 风险 | 后果 | 缓解措施 |
| --- | --- | --- |
| **短码枚举** | 扫出所有在线设备，为后续密码爆破铺路 | 按源 IP 限流 `connect`（如 **10 次/分钟**）；连续失败 N 次后指数延长冷却；**短码空间要足够大且稀疏分配**（9 位数字 = 10^9，在线设备数量级远小于此，命中率极低）；失败响应**统一延迟**，不区分「码不存在」和「设备离线」，避免通过响应时间差做存在性探测 |
| **虚假注册灌水** | 耗尽信令服务内存 / 挤占短码空间 | `register_token` 校验；按源 IP 限制注册数；注册总量硬上限；长时间无心跳自动清理 |
| **呼叫骚扰** | 被控端被持续呼叫打扰、日志刷满 | 被控端侧可配 `call_rejected`；信令侧按「目标设备」限流呼入频率；被控端可配置「仅接受已知来源」白名单（可选） |
| **放大攻击 / 流量滥用** | 信令服务被当作消息中继刷流量 | 消息体积硬上限（SDP 建议 **64 KB**，其他消息 **4 KB**）；单连接消息速率限制；**单次会话的 SDP/ICE 交换总字节数上限**（正常建连不超过几十 KB，超了必然是滥用） |
| **TURN 被白用** | 别人拿你的 TURN 当免费代理，带宽账单爆炸 | **必须用动态凭据**（上节的 HMAC 时效凭据），绝不开 `no-auth`；coturn 配 `denied-peer-ip` 屏蔽内网段（防 SSRF）；配带宽上限 `max-bps` 和总配额；监控流量告警 |
| **僵尸连接** | 文件描述符耗尽 | 心跳超时强制断开；单 IP 最大并发连接数限制；连接总数硬上限 |
| **信令服务单点故障** | 所有穿透连接不可用 | **已有连接不受影响**（信令只参与建连）；被控端自动重连；**局域网直连路径完全不依赖信令**，是天然的降级出口 |

一个容易遗漏的点：**信令服务必须记录可审计的日志**（注册、呼叫发起、呼叫接受/拒绝，带源 IP 和时间戳），但**绝不能记录 SDP 内容**（含内网 IP 等隐私信息）和任何凭据。

---

## 6. 降级阶梯与选路

### 6.1 四级阶梯

```
┌─ 第 0 级：直连 WSS（现状，零成本，最快）────────────────────────┐
│  条件：浏览器能直接访问被控端 IP:8443                           │
│  场景：同一局域网 / 被控端有公网 IP / 已做端口映射                │
│  路径：浏览器 ──TLS──> 被控端                                  │
│  延迟：最低（1 跳）  流量成本：0   依赖外部服务：无               │
└───────────────────────────────┬───────────────────────────────┘
                                │ 不通，或用户从信令页面进入
┌─ 第 1 级：WebRTC host 候选 ────▼──────────────────────────────┐
│  条件：两端在同一局域网（ICE 自动发现）                          │
│  路径：浏览器 ──DTLS──> 被控端（内网直达，不出网关）              │
│  延迟：极低   流量成本：0（仅信令几 KB）                         │
│  说明：和第 0 级物理路径相同，差别只是多一次信令会合              │
└───────────────────────────────┬───────────────────────────────┘
                                │ 跨网
┌─ 第 2 级：STUN 打洞（srflx 候选）▼───────────────────────────────┐
│  条件：至少一端不是对称型 NAT                                   │
│  路径：浏览器 ══P2P══> 被控端（直连，流量不过任何服务器）          │
│  延迟：接近理论最优   流量成本：0                                │
│  覆盖率：约 80%~90%（业界通行估计，本工程未实测）                 │
└───────────────────────────────┬───────────────────────────────┘
                                │ 打洞失败
┌─ 第 3 级：TURN 中继（relay 候选）▼───────────────────────────────┐
│  条件：TURN 服务可用且凭据有效                                  │
│  路径：浏览器 ──> TURN 服务器 ──> 被控端                        │
│  延迟：+1 跳（取决于 TURN 地理位置）                             │
│  流量成本：**全部视频流量过 TURN**，按带宽计费                    │
│  说明：中继只转发 DTLS 密文，看不到明文                          │
└───────────────────────────────────────────────────────────────┘
```

### 6.2 自动选路如何实现

**好消息：第 1~3 级的选路由 ICE 协议自动完成，不需要应用层写任何判断逻辑。**

机制：所有候选（host / srflx / relay）**同时**收集并交换，ICE 并行对所有候选对做连通性检查，按优先级公式（RFC 8445 §5.1.2，host > srflx > relay）选出能通且优先级最高的一对。打洞成功就不会用 relay，打洞失败自动落到 relay。

应用层只需要做一件事：**配置好 `ice_servers`**（STUN + TURN 地址，由信令服务在 `call_accepted` 里下发，见 [5.4 节](#54-信令消息格式草案)）。

**第 0 级 ↔ 第 1~3 级的切换需要应用层决策**，建议策略：

```
前端加载后：
  if (从被控端自己的页面进入，即 location.host 就是被控端) {
      → 直接走现有 WSS 路径（第 0 级），不碰 WebRTC
        // 保证局域网用户体验和现在完全一致，零回归风险
  } else {
      → 从信令服务页面进入，走 WebRTC 路径（第 1~3 级）
  }
```

即**用「前端从哪里加载」来区分模式**。这个判断零成本、无歧义，且天然保证了现有直连用户不受任何影响。

**可选的增强（M3 以后再考虑）**：从信令页面也可以先尝试直连（填 IP 快速连接），失败再退 WebRTC。但这会引入「尝试直连的超时等待」，反而拖慢体验，**优先级低**。

### 6.3 前端如何感知当前走哪条路

必须让用户看见，否则「为什么今天特别卡」无法自助排查，支持成本会很高。

**数据来源**：`RTCPeerConnection.getStats()` 返回的 `candidate-pair`（`state === 'succeeded'` 且 `nominated`）里有 `localCandidateId` / `remoteCandidateId`，顺着查 `local-candidate` / `remote-candidate` 的 `candidateType` 字段，取值为 `host` / `srflx` / `prflx` / `relay`。

**UI 建议**（延续现有前端的简洁风格，放在状态栏一角）：

| 实际路径 | 显示 | 颜色 | 悬浮提示 |
| --- | --- | --- | --- |
| 第 0 级 WSS | `直连` | 绿 | 「局域网直连，延迟最低」 |
| host / host | `局域网直连` | 绿 | 同上 |
| srflx 参与 | `P2P 直连` | 绿 | 「已穿透 NAT，点对点直连」 |
| 任一端 relay | `中继` | 黄 | 「网络环境受限，正在通过中继服务器转发，延迟和流量成本较高」 |
| 建连中 | `连接中…` | 灰 | 显示 ICE 状态（checking / connected / failed） |

同时建议一并展示 `getStats()` 里现成的 RTT（`candidate-pair.currentRoundTripTime`）和实际码率 —— 这些数据浏览器免费提供，对排查问题帮助极大。

另外：`iceConnectionState` 变为 `failed` 时要给出**可操作的**提示，而不是只说「连接失败」。例如「NAT 穿透失败，且未配置中继服务器。请联系管理员配置 TURN，或在同一局域网内使用直连模式」。

---

## 7. 横向对比（诚实评估）

### 7.1 候选方案

| 方案 | 工作原理 | 数据路径 |
| --- | --- | --- |
| **WebRTC**（本方案） | 信令会合 + ICE 打洞 | P2P 直连（80~90%）/ TURN 中继 |
| **自建 WebSocket 反向隧道** | 被控端出站连中继，中继转发浏览器请求 | 全程过中继 |
| **frp / nps** | 被控端跑 frpc 出站连 frps，frps 开端口对外 | 全程过 frps |
| **SSH 反向隧道** | `ssh -R 8443:localhost:8443 user@vps` | 全程过 VPS |
| **Cloudflare Tunnel** | 被控端跑 cloudflared 出站连 CF 边缘 | 全程过 Cloudflare |
| **Tailscale / ZeroTier** | 组虚拟局域网（本身就用了打洞 + DERP 中继） | P2P 直连 / DERP 中继 |

### 7.2 逐维度对比

| 维度 | WebRTC | WS 反向隧道 | frp/nps | SSH -R | Cloudflare Tunnel | Tailscale |
| --- | --- | --- | --- | --- | --- | --- |
| **带宽成本** | **打洞成功时 0** | 全部流量 | 全部流量 | 全部流量 | 全部流量（免费额度内 0） | 打洞成功时 0 |
| **延迟** | **最优**（直连） | +中继跳 | +中继跳 | +VPS 跳 | +CF 边缘跳（但 CF 网络优化好） | 最优 |
| **需固定公网端口** | 否 | 否 | 是（frps 侧） | 否 | 否 | 否 |
| **必需公网组件** | 信令（轻）+ TURN（兜底） | 中继（重，吃带宽） | frps（重） | VPS（重） | **无需自建** | 无需自建 |
| **工程改动量** | **大**（新增 PeerConnection、信令、传输抽象） | 中（新增出站 + 中继服务） | **极小**（零代码，装个 frpc） | **零代码** | **零代码** | **零代码** |
| **新增依赖体量** | +60~90 crate | +少量 | 0（外部进程） | 0 | 0（外部二进制） | 0 |
| **调试难度** | **高**（ICE 失败原因多、UDP 抓包难、跨浏览器差异） | 低 | 低 | 低 | 低 | 中 |
| **中继能否看到明文** | **不能**（DTLS 端到端） | **能**（除非额外加密） | **能** | 不能（SSH 加密，但 VPS root 能看） | **能**（CF 终止 TLS） | 不能 |
| **UDP 被封时** | 退 TURN/TCP（需配 `transport=tcp`） | TCP，不受影响 | TCP | TCP | TCP | 退 DERP |
| **运维负担** | **高**（信令 + TURN 两套服务 + 监控） | 中 | 中 | 低 | **极低** | **极低** |
| **部署复杂度（对终端用户）** | 低（内置，配个信令地址） | 低 | 中（要懂 frp 配置） | 高（要有 VPS 和 SSH 知识） | 中（要注册 CF 账号） | 中（要装客户端+登录） |
| **是否依赖第三方商业服务** | 否（可全自建） | 否 | 否 | 否 | **是**（CF） | **是**（Tailscale 协调服务器） |

### 7.3 WebRTC 的收益与代价（本工程语境）

**收益**：
1. **流量成本**。这是最大的收益，且随规模放大。本工程码率 4 Mbps 量级（`config.rs` 默认自动估算），纯中继方案下 10 个并发会话就是 **40 Mbps 持续上行** —— 这个带宽在国内云厂商按量计费下成本可观。WebRTC 下 80~90% 的会话走 P2P，中继成本只有剩下那 10~20%。
2. **延迟**。远程控制对延迟极度敏感（鼠标跟手感），P2P 直连省掉的中继跳在跨地域场景可能是几十毫秒的差距。
3. **安全模型更好**。DTLS 端到端，中继看不到屏幕内容。相比之下 WS 中继 / frp / Cloudflare 的中继节点都能看到明文（除非再套一层加密）。对「远程控制」这种高敏感场景，这不是小事。
4. **无需任何固定公网端口**，CGNAT 下也能用。
5. **不依赖第三方商业服务**，可完全自主部署（相比 Cloudflare Tunnel / Tailscale）。

**代价**：
1. **工程改动量最大**。所有候选方案里，只有 WebRTC 需要改应用代码（其他都是「装个代理」）。需要新增 PeerConnection 管理、信令客户端、独立信令服务 crate、传输层抽象、分片层，并改造 `web.rs` 的核心会话循环。
2. **运维两套服务**。信令（轻，但必须高可用 —— 挂了新连接全部建不起来）+ TURN（重，吃带宽，要防白用）。
3. **调试难度质变**。ICE 失败的原因有十几种（候选收集超时、NAT 类型不兼容、UDP 被封、TURN 凭据过期、SDP 协商不匹配、防火墙拦 DTLS…），且 UDP 流量抓包分析门槛高。虽然 `chrome://webrtc-internals` 很强，但 Rust 侧的问题定位依然困难。**这会显著拉长 M1 的实际工期**，要有心理准备。
4. **依赖体量 +60~90 crate**，pre-1.0 库，minor 版本可能 breaking。
5. **对本工程 10% 的用户仍然需要 TURN** —— 那部分的流量成本和纯中继方案一样，并没有省掉。

### 7.4 推荐结论：现在不做 WebRTC

> 本节结论与 [0.1 节](#01-推荐做什么)一致。**完整的决策树见 [0.2 节](#02-决策树)**，这里只补充「为什么 WebRTC 排在最后」的论证。

**结论：以本工程当前状态和实际需求，建议先用 [frp 反向隧道](#9-路线-zfrp-反向隧道-当前推荐本工程零改动)把双向控制跑通验证需求，WebRTC 作为后续可选演进。**

#### 为什么 WebRTC 的三个优势都不足以支撑「现在做」

**优势 1：省带宽** —— 只在打洞成功时成立。而打洞成功率**缺乏可引用的权威实测数据**：我实查了常被引用的几个来源，发现 Tailscale 博客的「90%+」原文**自述是估计而非实测**，libp2p 的 Protocol Labs 论文页与 punchr 仓库 README **均未给出成功率数字**（见 [附录 A.3](#a3-推测--待实测--不可作为决策依据)）。**在成功率不明的前提下，为「可能省下的带宽」付 23~38 人日是不理性的。** 更何况打洞失败的那部分仍需 TURN，成本与纯中继完全相同。

**优势 2：低延迟** —— P2P 少一跳。但实测表明[延迟的主要决定因素是服务器地理位置](#96-性能开销)，而非跳数本身。如果用户的公网服务器与两端都在同一地域，中继增加的延迟可能只有几十毫秒 —— 对远程控制的跟手感影响有限。**这个优势需要先用 frp 实测验证是否构成问题，再决定是否值得消除。**

**优势 3：端到端加密（中继看不到明文）** —— 这是**唯一确定成立**的优势（已核实，见 [11.2 节](#112-webrtc-的-dtls-在这点上确实天然更优已核实)），我在 7.3 节的论证不变。**但本次调研有一个关键发现削弱了它的独特性**：[frp 姿势 A（tcp 透传 + 保留本工程自签证书）](#姿势-atcp-透传--保留本工程自签证书-推荐)用**零改动**达到了同样的效果 —— 浏览器与本工程端到端 TLS，frps 只转发密文，服务器没有私钥。

> **所以 WebRTC 在加密上的优势，是相对于「应用层中继」成立，而不是相对于「tcp 透传隧道」成立。** 这一条原本是决策树最后一个分支的触发条件，现在 frp 姿势 A 已经满足了它。

#### 什么时候才该做 WebRTC

满足以下任一条件时，重新评估：

1. **带宽成本实际成为瓶颈** —— 按[带宽表](#124-带宽成本精确计算)，服务器带宽确实不够且无法通过[降质](#125-中继模式下建议下调画质)解决，同时设备数量足够多使得 P2P 省下的带宽有经济意义；
2. **延迟经实测不可接受** —— 用 frp 跑通后实测确认中继跳带来的延迟影响了可用性；
3. **必须用公网正式证书（姿势 B）又要求中继看不到明文** —— 此时才需要比较「WebRTC」与「中继 + [应用层 E2E 加密](#113-如果要做到中继看不见内容)」，后者成本更低（4~6 人日 vs 23~38 人日）；
4. 要做产品级体验且已经完成了[路线 0](#12-路线-0自研应用层中继) —— 因为 WebRTC 与自研中继**共享全部基础设施**（N0 传输层抽象、信令协议、设备模型），届时只是增量加一条 P2P 数据面。

**第 1~6 节的 WebRTC 完整设计保留为储备**，事实已核实、设计已成型，需要时可直接进入实施，不会因为「现在不做」而作废。

---

## 8. 分阶段实施计划

延续 `docs/05-实施计划.md` 的里程碑风格，编号从 **N**（NAT traversal）起，与现有 M0~M6 区分。

> **工作量为量级估计**（人日，单人），基于代码阅读的判断，**非精确排期**。WebRTC 联调的不确定性很大，N1 尤其可能超出估计。

### 8.1 N0 — 前置准备与传输层抽象（不引入 WebRTC）

**目标**：把 `web.rs` 的会话循环与具体传输解耦，为后续接入 DataChannel 铺路。**这个阶段完全不碰网络，纯重构，可独立验证、独立合并。**

| 改动 | 文件 | 说明 |
| --- | --- | --- |
| 抽出传输 trait | `crates/server/src/transport.rs`（新增） | 定义 `Transport` trait：`recv()` / `send_text()` / `send_binary()` / `close()` |
| WebSocket 实现 | `transport.rs` | 为 `axum::extract::ws::WebSocket` 实现该 trait |
| 会话循环泛型化 | `web.rs:168-539` | `handle_socket()` 改为接受 `impl Transport`，`tokio::select!` 内的 `socket.recv()` / `socket.send()` 换成 trait 方法 |
| 分片层 | `transport.rs` 或新增 `framing.rs` | 用现有帧头的 2 字节 `reserved`（`pipeline.rs:239`）承载分片信息；发送端按阈值切片，接收端重组。**阈值可配，WebSocket 模式下设为极大值即等效不分片** |
| 前端分片重组 | `web/app.js` | 在 `processVideoPacket()`（app.js:425）之前加一层重组；不分片时直通 |
| 丢帧后请求关键帧 | `web.rs:233-236` | 现有 `Lagged` 分支只记日志，补一个 `keyframe_tx.send(())`。**这是对现有直连模式的独立改进，与穿透无关** |

**验证**：局域网直连全流程回归 —— 认证、视频、鼠标键盘、剪切板、特殊键、分辨率变化、光标形状，逐项与改造前对比。分片层用小阈值（如 8 KB）强制触发分片路径，验证重组正确。

**工作量**：3~5 人日。**风险**：中（动核心循环，但无新依赖，回归范围明确）。

---

### 8.2 N1 — 信令服务 + WebRTC DataChannel 打通（核心里程碑）

**目标**：被控端在 NAT 内，控制端在外网，能完成一次完整的远程控制会话。

| 改动 | 文件 | 说明 |
| --- | --- | --- |
| 信令服务 | `crates/signal/`（新 crate） | axum 0.7：`/register`（被控端长连接）、`/connect`（控制端）、`ServeDir` 托管 `web/`。设备表、短码分配、消息透传、基础限流 |
| 根 workspace | `Cargo.toml` | `members = ["crates/server", "crates/signal"]` |
| 信令协议定义 | `crates/server/src/protocol.rs` 新增枚举，或提取共享模块 | `SignalMsg` 的 `#[serde(tag="type", rename_all="snake_case")]` 定义，与 [5.4 节](#54-信令消息格式草案)一致 |
| 信令客户端 | `crates/server/src/signaling.rs`（新增） | 出站 WebSocket 长连接、心跳、指数退避重连、device_id 持久化。**必须是独立 tokio task，错误不影响本地服务** |
| WebRTC 集成 | `crates/server/src/webrtc.rs`（新增） | PeerConnection 创建、answer 生成、ICE 候选处理、DataChannel 建立；为 DataChannel 实现 N0 的 `Transport` trait |
| 依赖 | `crates/server/Cargo.toml` | `webrtc = { version = "0.21", default-features = false, features = ["runtime-tokio", "crypto-ring"] }` |
| 前端 WebRTC 路径 | `web/app.js` | 信令连接、`createOffer`、ICE 处理、DataChannel 收发；按 [6.2 节](#62-自动选路如何实现)判断走直连还是 WebRTC |
| 配置项 | `crates/server/src/config.rs` | 新增 `[signaling]` 段（见 [8.6 节](#86-新增配置项草案)） |

**分两步验证（务必分开，否则出问题无法定位）**：
1. **先排除打洞变量**：信令服务和两端都放在同一局域网，WebRTC 走 host 候选。此时如果画面正常，说明「DataChannel 承载 + 分片 + 传输抽象」全部正确。
2. **再验证打洞**：信令服务上公网，被控端放到真实 NAT 内（家宽），控制端用 4G 热点（确保不同 NAT）。配 Google STUN，验证 srflx 打洞。

**工作量**：10~15 人日。**风险：高。** 这是整个方案不确定性最集中的阶段：webrtc 0.21 的新 API 无网上参考（只能看官方 39 个示例）、ICE 调试困难、Rust 侧 SCTP 实现成熟度待验证。**建议留足缓冲，不要和其他需求并行排期。**

---

### 8.3 N2 — TURN 中继与安全加固

**目标**：覆盖打洞失败的场景，并补齐 N1 阶段欠下的安全债。

| 改动 | 文件 | 说明 |
| --- | --- | --- |
| coturn 部署 | `docs/` + 部署脚本 | coturn 配置模板：`use-auth-secret`、`static-auth-secret`、`denied-peer-ip`（屏蔽内网段防 SSRF）、`max-bps` 带宽限制、`total-quota` |
| 动态凭据下发 | `crates/signal/` | HMAC-SHA1 时效凭据生成（`username = <ts>:<id>`），在 `call_accepted` 里下发 |
| 暴力破解防护改造 | `crates/server/src/session.rs`、`web.rs:346/415` | 按 `call_id` 维度计数 + **新增全局失败速率限制**。见 [5.5 节](#55-与现有-argon2-认证的串联)，**这是修安全回退，不是优化** |
| DTLS 指纹防 MITM | `webrtc.rs` + `app.js` | 用 argon2 密码派生密钥对指纹签名，控制端验签。见 [5.5 节](#55-与现有-argon2-认证的串联)缓解措施 1 |
| 信令限流完善 | `crates/signal/` | 按 [5.6 节](#56-滥用风险与限流)表格逐项实现 |
| 连接状态可视化 | `web/app.js` | `getStats()` 读 `candidateType`，按 [6.3 节](#63-前端如何感知当前走哪条路)展示 |

**验证**：在 UDP 被封的环境（可用 iptables 在测试机上 `DROP` UDP 模拟）验证自动退到 TURN；短码枚举限流生效；中继场景下单个 IP 的失败认证不会连坐封禁其他用户。

**工作量**：5~8 人日。**风险**：中（coturn 配置有坑但文档成熟；安全改造逻辑明确）。

---

### 8.4 N3 — 稳定性与体验优化

| 改动 | 说明 |
| --- | --- |
| 断线重连 | ICE 断开后自动重连（利用 ICE restart 能力，`rtc` 已声明支持）；DataChannel 重建后恢复会话 |
| 背压调优 | 用 `bufferedAmount` 做丢帧决策（见 [3.1 节](#31-路线-artcdatachannel-承载现有帧)(4)），替代/补充现有 broadcast Lagged 机制；`broadcast` 容量 8（`web.rs:82`）改为可配 |
| 分片大小调优 | 实测不同分片阈值下的吞吐与延迟，确定默认值 |
| 多设备管理 | 信令服务加简单 Web UI 列出在线设备（需要给信令服务加管理员认证） |
| 诊断能力 | 一键导出连接诊断信息（ICE 候选、选中路径、RTT、丢包率），降低支持成本 |

**工作量**：5~10 人日。**风险**：低（都是增量优化，可按需裁剪）。

---

### 8.5 N4（可选）— 媒体轨路线

仅在 [3.4 节](#34-判断依据与推荐)列出的触发信号出现时启动。前置条件是 N1~N3 已稳定运行。

| 改动 | 说明 |
| --- | --- |
| H.264 媒体轨 | `TrackLocalStaticSample` + `MIME_TYPE_H264`（已核实可用） |
| 编码器动态码率 | `encoder.rs` 新增 `set_bitrate()`（**待核实** openh264 crate 0.6 是否暴露 `ENCODER_OPTION_BITRATE`） |
| PLI → 关键帧 | 对接现有 `keyframe_tx`（`web.rs:491-497` 路径已存在） |
| 带宽估计对接 | 从 `getStats()` / 库的 BWE 接口读目标码率，驱动编码器 |
| 前端改造 | WebCodecs 路径（app.js 约 100 行）换成 `<video srcObject>` |
| profile 对齐 | 确保 openh264 输出的 profile 能被浏览器硬解（**待实测**） |

**工作量**：8~12 人日。**风险**：中高（H.264 媒体轨无官方示例参考）。

---

### 8.6 新增配置项草案

延续 `docs/05-实施计划.md` 第 3 节的 config.toml 风格，**全部新增项都有默认值，不填即保持现有行为**：

```toml
# ===== 以下为现有配置，不变 =====
bind = "0.0.0.0:8443"
password = "change-me"
[video]
fps = 30
# ... 略，见 docs/05-实施计划.md

# ===== 新增：信令与穿透 =====
[signaling]
# 是否启用穿透。默认 false —— 不配置时行为与当前版本完全一致。
enabled = false

# 信令服务地址。enabled = true 时必填。
url = "wss://signal.example.com/register"

# 被控端向信令服务注册的凭据。与访问密码(password)无关，不要填同一个值。
register_token = ""

# 设备展示名，仅用于控制端界面展示。留空则用主机名。
device_name = ""

# 设备 ID 持久化路径。留空则用配置文件同级目录的 device_id。
# 首次启动生成 UUID v4 并写入，之后复用。
# device_id_file = "device_id"

# 心跳间隔（秒）与心跳超时（秒）。
heartbeat_interval = 30
heartbeat_timeout = 10

# 重连退避上限（秒）。
reconnect_max_backoff = 60

[webrtc]
# ICE 服务器。被控端侧配置；控制端侧由信令服务下发（见 5.4 节），
# 两边一般应保持一致。
ice_servers = [
    { urls = ["stun:stun.l.google.com:19302"] },
    # TURN 示例（凭据建议由信令服务动态下发，不要长期写死在这里）：
    # { urls = ["turn:turn.example.com:3478?transport=udp",
    #           "turn:turn.example.com:3478?transport=tcp"],
    #   username = "", credential = "" },
]

# DataChannel 分片阈值（字节）。0 表示不分片。
# 16384 是社区实测吞吐较优值；Chrome 硬上限实测为 262144。
chunk_size = 16384

# 视频通道的 SCTP 可靠性配置。
# ordered = false + max_retransmits = 0 即「宁丢不等」，实时流应当如此。
video_ordered = false
video_max_retransmits = 0

# 发送缓冲区高水位（字节）。超过则丢帧而非排队，防止延迟雪崩。
send_buffer_high_watermark = 1048576

# ICE 建连超时（秒）。超时后前端提示失败。
ice_timeout = 30

# ===== 新增：安全（扩展现有 [security] 段）=====
[security]
# 现有项不变
max_auth_failures = 5
auth_ban_seconds = 300
session_idle_timeout = 300
exclusive_control = false

# 新增：全局认证失败速率上限（次/分钟，所有来源合计）。
# 中继场景下按 IP 封禁会失效，这是兜底防线。0 表示不限制。
global_auth_failure_rate = 30
```

信令服务自己的配置（`crates/signal/signal.toml`）另行设计，大致需要：监听地址、TLS 证书、`register_token` 校验方式、TURN `static-auth-secret`、各项限流阈值、静态资源目录。

### 8.7 对现有直连模式的兼容承诺

**这是本方案的硬约束，列为验收标准：**

| 承诺 | 技术保障 |
| --- | --- |
| `[signaling] enabled = false`（默认）时，**行为与当前版本逐字节一致** | 信令客户端和 PeerConnection 都在 `enabled = true` 时才初始化；配置默认值即 false |
| 不填新配置项的旧 `config.toml` **直接可用** | 所有新增配置项有默认值，`AppConfig` 已用 `#[serde(default)]`（`config.rs:7`），新增段落自动走默认 |
| 被控端的 HTTPS/WSS 服务**继续监听**，`ServeDir` 继续托管前端 | `web::serve()` 的逻辑不因穿透功能改变；信令是**并行**的额外能力，不是替代 |
| 信令服务不可达时，**本地直连功能完全不受影响** | 信令客户端是独立 tokio task，错误只记日志，不 propagate 到 `main()`；无重试风暴 |
| 局域网用户**不需要任何公网服务** | 从被控端页面进入即走第 0 级直连，完全不碰信令和 ICE（见 [6.2 节](#62-自动选路如何实现)） |
| 现有 argon2 认证、暴力破解防护、剪切板、输入注入**语义不变** | N0 只做传输层抽象，不改业务逻辑；N2 的防护改造是**增强**（新增全局限流），不削弱现有 per-IP 机制 |
| 龙芯 / 交叉编译**不引入新的 C 依赖** | `crypto-ring` 路线已核实全链路无 openssl / aws-lc-sys（见 [4.2 节](#42-加密后端与本工程-ring-路线的兼容性核心风险点已核实)）。**唯一新约束是 rustc ≥ 1.85** |

**回归测试清单**（每个阶段合并前必跑，按 `docs/05-实施计划.md` 第 6 节的验证策略执行）：

1. 局域网 HTTPS 直连，密码认证通过/失败各一次；
2. 视频流正常（关键帧、P 帧、分辨率变化、光标形状）；
3. 鼠标移动/点击/滚轮、键盘输入、特殊快捷键；
4. 剪切板双向同步（含超长文本截断）；
5. 暴力破解封禁触发与解封；
6. 三平台后端各跑一遍（x11 / Windows / Wayland-via-Xwayland）；
7. 旧版 `config.toml`（不含新字段）能正常启动。

### 8.8 风险汇总

| 风险 | 影响 | 概率 | 应对 |
| --- | --- | --- | --- |
| **rustc ≥ 1.85 门槛**（edition 2024） | 龙芯等交叉编译链可能达不到 | 中 | **N1 开工前先确认各目标平台的 rustc 版本**。若不满足：降级用 webrtc 0.17.2（edition 2021，同样是 ring 后端，已核实），代价是旧 API + 该线维护力度较弱 |
| webrtc-rs SCTP 实现性能/稳定性不足 | 画面卡顿、吞吐不够 | 中 | N1 第一步（局域网 host 候选）即可暴露；兜底是转方案 B 或 str0m + rust-crypto |
| webrtc 0.21 新 API 无参考资料 | N1 工期显著超出估计 | **高** | 只参考官方 39 个示例；预留缓冲；考虑先用官方 data-channel 示例跑通最小闭环再接入本工程 |
| pre-1.0 库 minor 版本 breaking | 升级成本 | 中 | `Cargo.lock` 锁死版本；升级作为独立任务评估，不随业务改动顺手升 |
| ICE 打洞失败率高于预期 | 用户体验差 | 中 | N2 的 TURN 是必选不是可选；同时文档给出 frp/Cloudflare Tunnel 替代指引 |
| TURN 流量成本超预算 | 运营成本 | 中 | coturn 配 `max-bps` + `total-quota`；监控告警；必要时限制中继会话的码率（降质保通） |
| 信令服务成为单点 | 新连接全部建不起来 | 中 | 已建连接不受影响；可多实例 + 共享状态（Redis）；局域网直连是天然降级出口 |
| `web.rs` 核心循环重构引入回归 | 现有功能破坏 | 中 | N0 独立成阶段、独立验证、独立合并；严格执行 [8.7 节](#87-对现有直连模式的兼容承诺)回归清单 |
| 1080p IDR NAL 超 DataChannel 上限 | 关键帧发不出，画面不出来 | **高（若不处理）** | N0 的分片层是**必做项**，不是优化项 |

**总工作量估计：N0~N3 共 23~38 人日**（单人，含调试，不含 N4）。其中 N1 占比最大且不确定性最高。

---

## 9. 路线 Z：frp 反向隧道（★ 当前推荐，本工程零改动）

> **为什么这条路可行**：现有被控端已经是标准 WSS 服务（`web.rs:141` listen 在 `0.0.0.0:8443`）。frpc 跑在被控端机器上，主动出站连到公网的 frps —— **frpc 替被控端完成了「主动出站」这件唯一缺失的能力**，而这正是[第 12 节](#12-路线-0自研应用层中继)要自己写 10~15 人日的东西。本工程不改一行代码。

### 9.1 拓扑

```
控制端浏览器            公网服务器(你的)              被控端(NAT 内)
                       ┌─────────────┐
                       │    frps     │
                       │ :7000 (控制) │
   ──── https ────────>│ :8443 (对外) │
                       └──────▲──────┘
                              │ ① frpc 主动出站连 :7000
                              │   (NAT 友好，必然成功)
                       ┌──────┴──────┐
                       │    frpc     │
                       │      ↓      │
                       │ 127.0.0.1   │
                       │   :8443     │  ← 本工程，完全不知道
                       │ (本工程原样) │     自己被隧道了
                       └─────────────┘
```

### 9.2 已核实的 frp 事实

| 事实 | 来源 |
| --- | --- |
| **v0.52.0 起支持 TOML / YAML / JSON**；INI 已废弃（"deprecated and will be removed in future releases"），新特性只在新格式实现 | frp 官方 README |
| frps 键名：`bindPort`、`allowPorts`、`auth.token`、`auth.method`、`webServer.addr/port/user/password`、`subDomainHost`、`maxPortsPerClient`（默认 0 = 无限制）、`transport.heartbeatTimeout`（默认 90）、`transport.maxPoolCount`（示例值 5）、`userConnTimeout`（示例 10） | frp README + `conf/frps_full_example.toml` |
| frpc 代理键名：`[[proxies]]` 数组，含 `name` / `type` / `localIP` / `localPort` / `remotePort` | frp README |
| frpc TLS 键名：`transport.tls.enable`、`transport.tls.certFile`、`keyFile`、`trustedCaFile`、`useSystemCa`、`disableCustomTLSFirstByte` | frp README |
| frps TLS 键名：`transport.tls.force`、`certFile`、`keyFile`、`trustedCaFile` | frp README |
| **v0.50.0 起 `transport.tls.enable` 与 `disableCustomTLSFirstByte` 默认为 true** —— 即 frpc↔frps 之间默认已加密 | frp README |
| **不配 `trustedCaFile` 或 `useSystemCa` 时，frpc 不校验服务端证书**（原文："does not verify the server certificate"） | frp README |
| `auth.method` 支持 `token`（默认）与 `oidc`；可用 `auth.additionalScopes = ["HeartBeats"]` / `["NewWorkConns"]` 扩展鉴权范围 | frp README |
| 最新版本号 | **未能核实** —— GitHub Releases 区块未渲染出版本号。请以 [Releases 页](https://github.com/fatedier/frp/releases) 实际显示为准，只需 ≥ v0.52.0 即可用 TOML |

### 9.3 配置示例（TOML，frp ≥ v0.52）

**公网服务器 `frps.toml`**：

```toml
# ===== 基础 =====
bindPort = 7000                      # frpc 来连的控制端口

# ===== 鉴权（必须配，否则任何人都能连你的 frps 建隧道）=====
[auth]
method = "token"
token = "换成一个长随机串，如 openssl rand -base64 32 的输出"
# 把心跳也纳入鉴权校验，提高伪造成本
additionalScopes = ["HeartBeats"]

# ===== 加固：只允许开放指定端口，防止被白用成任意端口代理 =====
allowPorts = [
    { single = 8443 },               # 只放本工程这一个端口
]
maxPortsPerClient = 1                # 每个客户端最多占 1 个端口

# ===== 加固：frpc↔frps 链路强制 TLS =====
[transport]
heartbeatTimeout = 90
[transport.tls]
force = true                         # 拒绝非 TLS 的 frpc 连接

# ===== dashboard：默认只监听 127.0.0.1，不要改成 0.0.0.0 =====
# 需要远程看时，用 SSH 端口转发访问，别直接暴露公网。
# [webServer]
# addr = "127.0.0.1"
# port = 7500
# user = "admin"
# password = "换成强密码"
```

**被控端 `frpc.toml`**：

```toml
serverAddr = "你的公网服务器IP或域名"
serverPort = 7000

[auth]
method = "token"
token = "与 frps 完全相同的那个随机串"
additionalScopes = ["HeartBeats"]

# frpc↔frps 链路 TLS。v0.50+ 默认已是 true，这里显式写出以免将来默认值变化。
[transport.tls]
enable = true
# 强烈建议补上证书校验，否则 frpc 不验证服务端证书（见 9.2 已核实事实），
# 存在中间人风险。二选一：
# useSystemCa = true                 # frps 用正式证书（Let's Encrypt）时用这个
# trustedCaFile = "/path/to/ca.crt"  # frps 用自签证书时，把 CA 放进来

# ===== 把本机 8443 反向隧道到公网 8443 =====
[[proxies]]
name = "remotecontrol"
type = "tcp"                         # 字节透传，见 9.5
localIP = "127.0.0.1"
localPort = 8443
remotePort = 8443
```

启动后，外网浏览器访问 `https://你的公网服务器:8443` 即可控制 NAT 内的被控端。**本工程的 `config.toml` 一字不改。**

一个细节：`localIP = "127.0.0.1"` 配合把本工程的 `bind` 改成 `127.0.0.1:8443`（可选），能让被控端**不再监听局域网**，只接受 frpc 的本地连接，减小暴露面。但这样就失去了局域网直连能力 —— 如果两种场景都要，保持 `bind = "0.0.0.0:8443"` 即可，frp 不影响局域网直连同时工作。

### 9.4 TLS 嵌套：最容易卡住的一关

这里有**三层**可能的 TLS，必须理清：

```
① 浏览器 ←──TLS?──→ frps        (对外，浏览器看到的证书)
② frps   ←──TLS ──→ frpc        (frp 自身的隧道加密，默认已开)
③ frpc   ←──TLS?──→ 本工程:8443  (本工程自己的 WSS)
```

关键认知：**`type = "tcp"` 模式下 frp 是纯字节转发，它不解析也不终止 TLS。** 所以 ① 和 ③ 实际是**同一条 TLS 会话**（浏览器直接和本工程握手，frp 只是搬运密文），而 ② 是套在外面的另一层加密。

#### 姿势 A：tcp 透传 + 保留本工程自签证书（★ 推荐）

**配置**：就是上面 [9.3 节](#93-配置示例tomlfrp--v052)的原样，本工程 `tls.enabled = true`（默认值）不动。

**效果**：
- 浏览器与本工程**端到端 TLS**，frps 所在服务器**看不到明文**（密文经过它，但它没有私钥）。
- 浏览器会报证书警告（自签证书 + IP 访问），需要点「继续访问」—— **和现在局域网直连的体验完全一样**，用户已经熟悉。

**代价**：证书告警。但换来的是**连你自己的服务器都看不到屏幕内容**，这个安全性是白拿的。

> 注意一个坑：本工程的自签证书（`tls.rs` 用 rcgen 生成）的 SAN 大概率不含公网 IP/域名，所以浏览器除了「证书不受信任」还会报「证书与域名不匹配」。两者都是点一下继续就能过，但如果想消掉后者，需要在生成自签证书时把公网域名加进 SAN —— 这**需要改 `tls.rs`**，就不再是零改动了。**建议先接受告警。**

#### 姿势 B：公网侧统一上正式证书

**配置**：
1. 本工程 `config.toml` 设 `[tls] enabled = false`（降级为明文 HTTP/WS，只在 `127.0.0.1` 上听）；
2. 公网服务器上用 Caddy / nginx 做 TLS 终止，配 Let's Encrypt 正式证书，反代到 frps 暴露的端口；
3. **frpc↔frps 之间必须开 `transport.tls.enable = true`**（这步绝不能省，否则被控端到服务器之间是明文穿越公网）。

**效果**：浏览器无证书告警，体验最干净。

**代价（必须明确告知）**：**TLS 在公网服务器上终止，服务器能看到完整的屏幕画面明文和键盘输入。** 第 2 段（frpc↔frps）虽有 frp 的 TLS 保护，但那是 frps 解密后再加密的，服务器本身处于明文位置。

#### 选哪个

| | 姿势 A（透传自签） | 姿势 B（公网终止正式证书） |
| --- | --- | --- |
| 本工程改动 | **零** | 改一个配置项（`tls.enabled = false`） |
| 浏览器告警 | 有，需点继续 | 无 |
| **服务器能否看到明文** | **不能** | **能** |
| 额外部署 | 无 | Caddy/nginx + 域名 + 证书 |

**推荐姿势 A。** 理由：零改动、服务器看不到明文，代价只是一个一次性的证书告警点击。姿势 B 适合「要给多人用、不想解释证书告警」且「信任自己服务器」的场景。

### 9.5 frp 对 WebSocket 的支持

**`type = "tcp"` 模式下无需任何特殊处理** —— frp 在 TCP 层做字节转发，不理解上层协议，WebSocket 的 Upgrade 握手、二进制帧、长连接全部原样透传。这也是推荐 tcp 模式的另一个理由。

**（推测，未实查）** 如果用 `type = "http"` / `"https"` 模式，frp 会参与 HTTP 层处理，WebSocket 升级理论上支持（frp 有相关实现），但会引入「是否正确转发 `Upgrade`/`Connection` 头」、「HTTP 超时是否掐断长连接」这类不确定性。**本场景没有任何理由用 http 模式** —— 它的价值是按域名复用 80/443 端口，而本工程用独立端口更简单。

### 9.6 性能开销

**（量级估计，未实测）**

| 维度 | 影响 |
| --- | --- |
| **延迟** | 增加一跳：浏览器→服务器→被控端。实际增量 ≈ 两段 RTT 之和 − 直连 RTT。若服务器与两端地理位置都不远，典型增量**几十毫秒**；若服务器在境外而两端在境内，可能增加 100ms+ ——**服务器地理位置是延迟的主要决定因素**，远比 frp 本身的转发开销重要 |
| **吞吐** | frp 的转发是用户态 copy，单路 6 Mbps 对现代服务器可忽略。瓶颈几乎必然是**带宽**而非 CPU |
| **CPU** | frpc 在被控端的开销很小（纯转发）。注意被控端的主要 CPU 开销是 `encoder.rs` 的逐像素 BGRA→YUV 转换，frp 不改变这一点 |
| **抖动** | 多一跳意味着多一处排队点。服务器带宽跑满时抖动会明显上升 —— 这是[带宽表](#124-带宽成本精确计算)要认真看的原因 |

### 9.7 带宽成本：与自研中继完全相同

**已确认：frp、自研应用层中继、Cloudflare Tunnel、TURN 中继，这四者的带宽成本完全一致。** 原因很简单 —— 它们都是「全部视频流量经过服务器」，差别只在实现方式，不在流量路径。

所以[第 12.4 节的带宽表](#124-带宽成本精确计算)对 frp 同样适用，请务必据此判断服务器带宽是否够用。

### 9.8 安全边界（必须知晓）

| 项 | 说明 |
| --- | --- |
| **姿势 A 下服务器看不到明文** | 浏览器与本工程端到端 TLS，frps 只转发密文。**这是姿势 A 的核心优势** |
| **姿势 B 下服务器能看到明文屏幕与键盘输入** | TLS 在服务器终止。服务器是你自己的，这个风险可能可以接受 —— **但必须是知情的选择，而不是不知道** |
| frps 必须配 `auth.token` | 不配等于开放隧道服务，任何人都能用你的服务器建隧道（白用带宽、甚至当跳板） |
| frps 必须配 `allowPorts` | 否则客户端可以申请开放任意端口，把你的服务器变成通用代理 |
| dashboard 不要暴露公网 | `webServer.addr` 默认 `127.0.0.1`，**保持默认**。需要远程查看就用 SSH 端口转发 |
| frpc 应配证书校验 | 默认不校验服务端证书（已核实），存在中间人风险。配 `useSystemCa` 或 `trustedCaFile` |
| 本工程的 argon2 密码仍是最后防线 | 隧道只解决可达性，不提供访问控制。**公网可达后密码强度变得关键** —— 默认密码 `123456`（`config.rs:97`）必须改掉 |
| 暴力破解防护仍然有效（但要注意） | `BruteForceGuard` 按 `peer_addr.ip()` 封禁。tcp 透传模式下被控端看到的 peer IP 是 **frpc 的本地地址（127.0.0.1）**，所有外网请求会被归为同一个 IP → **封禁会连坐**。这是 frp 方案的一个实际缺陷，缓解办法是把 `max_auth_failures` 配大一些或接受连坐（反正只有你自己用），彻底解决需要 frp 的 Proxy Protocol 支持 + 本工程适配（**那就不是零改动了**） |

> 最后这条值得单独强调：**零改动方案也有零改动的代价** —— 失去了按真实来源 IP 做防护的能力。如果公网暴露且担心爆破，务必用强密码（这比 IP 封禁重要得多）。

---

## 10. Cloudflare Tunnel 评估：技术可行，但条款有明确风险

**结论：不推荐作为长期方案。** 技术上能跑，但 Cloudflare 的服务条款明确要求「通过 CDN 传视频必须购买相应付费产品」，而本工程传的**就是**视频流。

### 10.1 已核实的条款原文（关键）

查 Cloudflare 官方 [Service-Specific Terms](https://www.cloudflare.com/service-specific-terms-application-services/)，在 **"Content Delivery Network (Free, Pro, or Business)"** 标题下（该文档此处用无编号标题，无条款号可引）：

> 非 Enterprise 客户若要通过 CDN 提供视频和其他大文件，**必须使用**特定付费服务（原文点名 the Developer Platform、Images、Stream）。

且 Cloudflare 保留处置权：

> 在 CDN 被用于（或被怀疑被用于）在未购买上述付费服务的情况下提供视频，或提供「不成比例的图片、音频文件或其他大文件」时，Cloudflare 可以**停用或限制你对 CDN 的访问**。原文附带一句 "We will use reasonable efforts to provide you with notice of such action."

**一个需要澄清的点**：网上广泛流传的「2.8 条 Limitation on Serving Non-HTML Content」**在现行协议里已不存在**。我实查了 [cloudflare.com/terms](https://www.cloudflare.com/terms/) 的 Self-Serve Subscription Agreement（2025-09-12 版本），**全文没有这个标题，也没有提到 video / audio / streaming / caching 限制**。该限制现在位于上述 Service-Specific Terms 文档中。引用时请用现行位置，不要再引「2.8 条」这个过时编号。

### 10.2 WebSocket 支持

**（部分核实）** WebSocket 在 Cloudflare 各套餐（含免费版）均可用，也能通过 `cloudflared` 隧道工作。两个运维要点：
- 已建立的 WebSocket **不受** 100 秒源站超时（HTTP 524）约束 —— 那个限制只作用于初始 HTTP 请求。
- 但空闲连接会被回收，**需要应用层心跳**。好消息：本工程前端已有 ping/pong 机制（`protocol.rs` 的 `ClientMsg::Ping` / `ServerMsg::Pong`，`app.js` 中有调用），这一点天然满足。

> 坦白标注：WebSocket 的这些细节我**未能从 Cloudflare 官方文档页逐条实查**（抓取尝试返回错误）。上述内容与广泛的社区实践一致，但**请以官方 WebSocket 文档为准**。相比之下，10.1 节的条款限制是我**实查官方条款页面确认**的 —— 这才是决定「要不要用」的关键，而它的答案是否定的。

### 10.3 为什么说「不推荐」而非「不能用」

| 维度 | 评估 |
| --- | --- |
| 技术可行性 | **可以跑通**。免费、自带 TLS 和域名（`*.trycloudflare.com` 或自有域名）、不需要自己的服务器、不需要公网 IP |
| 零改动程度 | 与 frp 同级 —— `cloudflared` 也是替被控端完成出站 |
| **条款合规** | **明确有问题**。本工程的核心流量就是 H.264 视频流，正好落在条款禁止的范围内 |
| 实际风险 | 低流量短时使用很可能长期无事；但**持续高带宽**（按[带宽表](#124-带宽成本精确计算)，1080p@30 是 6.22 Mbps 持续）正是最容易触发处置的模式 |
| **加密边界** | **Cloudflare 终止 TLS，能看到明文屏幕内容和键盘输入**。这比 frp 姿势 A（服务器看不到明文）差，且这次看到明文的是**第三方公司**，不是你自己的服务器 |

**两条独立的否决理由**：条款不允许；以及**第三方能看到你的屏幕明文**。后者对远程桌面这类高敏感场景尤其不可接受。

**可接受的用法**：临时演示、快速验证（跑几分钟十几分钟），用 `cloudflared tunnel --url` 零配置起一个临时隧道很方便。**但不要作为日常方案。**

用户既然已有公网服务器，[frp 路线](#9-路线-zfrp-反向隧道-当前推荐本工程零改动)在每个维度上都更优（无条款风险、服务器自己掌控、姿势 A 下看不到明文），**没有理由选 Cloudflare Tunnel**。

---

## 11. 加密的真实边界

> 用户问的是「两端都通过公网服务器进行加密通信能否实现」。答案是**能，但"加密"这个词在这里有两种完全不同的含义**，差别关系到服务器能否看到你的屏幕。

### 11.1 两段 TLS ≠ 端到端加密

```
【传输加密（两段 TLS）】—— 中继能看到明文
  浏览器 ──TLS①──> 服务器 ──TLS②──> 被控端
                     ▲
                     └── TLS 在这里终止、解密、再加密
                         此处内存中是明文：屏幕画面、键盘输入、剪切板

【端到端加密】—— 中继看不到明文
  浏览器 ══════════ 密文 ══════════> 被控端
            服务器只搬运密文，无密钥
```

**两段 TLS 保护的是「链路不被窃听」，不保护「中继节点本身」。** 这是设计上的必然 —— 中继要在两条连接间转发，就必须先解密再转发。

受影响的方案：

| 方案 | 服务器能否看到明文 |
| --- | --- |
| **frp 姿势 A（tcp 透传 + 本工程自签证书）** | **不能** —— 浏览器与本工程直接握手，frps 只转发密文 |
| frp 姿势 B（公网终止正式证书） | **能** |
| 自研应用层中继（[第 12 节](#12-路线-0自研应用层中继)） | **能**（除非额外做 E2E） |
| Cloudflare Tunnel | **能**（且是第三方） |
| WebRTC + TURN | **不能** —— DTLS 端到端（见 [11.2](#112-webrtc-的-dtls-在这点上确实天然更优已核实)） |

> **一个重要发现**：frp 姿势 A 在加密边界上**和 WebRTC 一样好** —— 都做到了「中继看不到明文」，而成本是零改动 vs 20~40 人日。这实质性削弱了 WebRTC 在这一点上的独特优势。

### 11.2 WebRTC 的 DTLS 在这点上确实天然更优（已核实）

**核实结论：是的，这是 WebRTC 的真实优势，不是宣传话术。**

机制：WebRTC 规范强制加密，DataChannel 走 SCTP over **DTLS**，媒体轨走 **SRTP**（密钥由 DTLS-SRTP 协商，RFC 5764）。而 **TURN 的职责仅仅是「转发 UDP 包」** —— 它工作在 DTLS 之下，拿到的就是密文，**协议上没有任何途径解密**。

对比的关键差异：

| | TURN 中继 | 应用层中继（frp 姿势 B / 自研） |
| --- | --- | --- |
| 中继工作的层次 | **UDP 包转发**，在加密层之下 | **应用层转发**，必须先解密 |
| 中继是否持有密钥 | **否**（DTLS 密钥由两端协商，TURN 不参与） | **是**（TLS 在它上面终止） |
| 能否看到明文 | **协议上不可能** | **能** |

所以 WebRTC 即使退化到全程走 TURN 中继（最坏情况），**安全性仍优于应用层中继** —— 这是它架构上的内在属性。

但要注意两个限定：
1. **DTLS 指纹经信令传递，所以恶意信令服务理论上可做 MITM**（替换双方指纹插在中间）。这是所有基于信令的 WebRTC 方案的固有弱点，缓解措施见 [5.5 节](#55-与现有-argon2-认证的串联)。如果信令服务是用户自己部署的，这条风险大幅下降。
2. 如上节所述，**frp 姿势 A 用零成本达到了同样的效果**。WebRTC 的这个优势相对于「应用层中继」成立，相对于「tcp 透传隧道」则不成立。

### 11.3 如果要做到中继看不见内容

前提：选了会看到明文的方案（frp 姿势 B / 自研中继 / Cloudflare），又要求中继看不到内容。

**首选结论：别做 E2E，改用 frp 姿势 A。** 零成本达到同样效果（见 [11.1 节](#111-两段-tls--端到端加密)）。下面的方案仅在「必须用公网正式证书 + 又要 E2E」时才需要。

#### 可行的技术路线（已核实可行性）

**浏览器侧能力（已核实 MDN SubtleCrypto 算法表）**：

| 算法 | 浏览器支持 | 用途 |
| --- | --- | --- |
| **X25519** | **支持**（`deriveKey` / `deriveBits`，Web Crypto Level 2 新增） | 密钥协商 |
| **AES-GCM** | **支持**（`encrypt` / `decrypt`） | **AEAD 加密，这是浏览器端唯一实际可用的选择** |
| **HKDF** | 支持（`deriveKey` / `deriveBits`） | 从共享密钥派生会话密钥 |
| **PBKDF2** | 支持 | 从密码派生密钥 |
| ChaCha20-Poly1305 | **不支持** | — |
| Argon2 | **不支持** | — |

两个关键约束（直接影响设计）：
1. **浏览器没有 ChaCha20-Poly1305** → 必须用 **AES-GCM**。所以「Noise 协议 + ChaCha20」这个常见组合在浏览器侧行不通，除非上 WASM（不值得）。
2. **浏览器没有 Argon2** → **不能在浏览器端从密码直接派生出与被控端一致的密钥**。只能用 PBKDF2（浏览器有）或者改用 X25519 密钥协商（推荐）。

**Rust 侧**：`snow` crate（Noise 协议实现，已核实最新 0.10.0 / 2025-07-19，下载量 2980 万，活跃维护，**有 `ring-resolver` / `ring-accelerated` feature 可复用本工程的 ring**，无 aws-lc 依赖）。但注意 snow 0.10.0 是 **edition 2024 / MSRV 1.85** —— 和 webrtc 0.21 一样的工具链门槛。

#### 推荐的最简设计

**不要用 Noise**（浏览器侧没有现成实现，要自己对接握手状态机，复杂度高）。推荐手写一个极简的密钥协商：

```
1. 双方各生成 X25519 临时密钥对（浏览器用 SubtleCrypto，Rust 用 ring 的 agreement）
2. 通过中继交换公钥（中继看得到公钥，但看不到私钥 —— 这是 DH 的基本性质）
3. 各自算出共享密钥，HKDF 派生出收发两个方向的 AES-256-GCM 密钥
4. 后续所有应用层消息（0xAB 帧 + JSON）用 AES-GCM 加密，nonce 用递增计数器
5. 防 MITM：用 argon2 密码派生一个 PSK，把它混进 HKDF 的 salt/info
   —— 中继不知道密码，所以无法伪造出双方都接受的共享密钥
```

第 5 步是**必须的**：没有它，中继可以分别和两端做 DH（经典 MITM）。注意浏览器没有 Argon2，所以这个 PSK 的派生要么用 PBKDF2（两端都改用 PBKDF2），要么让被控端把 argon2 的 salt 发给浏览器 —— **这部分需要仔细设计，是整个 E2E 方案最容易出错的地方**。

#### 改动量与性能评估

| 项 | 评估 |
| --- | --- |
| 改动量 | **中等**：Rust 侧 +1 个加解密层（约 200~300 行）；前端 +密钥协商与解密（约 150~200 行）；协议层要加握手消息。**但不需要动 `pipeline.rs` / `encoder.rs` / 解码逻辑** |
| 浏览器性能 | **AES-GCM 有硬件加速（AES-NI），SubtleCrypto 走原生实现。** 6 Mbps 的解密量对现代 CPU 可忽略（AES-NI 吞吐在 GB/s 量级）。**真正的开销是 `await` 的异步调用次数** —— SubtleCrypto 全是 Promise，逐 NAL 解密会产生大量微任务。**建议按帧而非按 NAL 批量解密**以减少调用次数 |
| 喂给 WebCodecs | **可行**。解密后得到的仍是 Annex-B 字节，原样走现有 `processVideoPacket()`（`app.js:425`）即可。**解码路径不用改**，只是在它前面多一步解密 |
| 风险 | **自己实现加密协议风险高**。密钥复用、nonce 重复、缺少重放保护都是经典翻车点。如果要做，必须做代码审查 |

#### 是否建议做、什么阶段做

**不建议现在做。** 三条理由：

1. **frp 姿势 A 零成本解决了同一个问题**（中继看不到明文）。先用它。
2. 如果服务器是用户自己完全掌控的，「服务器能看到明文」的威胁模型其实**不包含真实攻击者** —— 能看到的只有用户自己。
3. 自研加密协议的翻车风险，可能比它要防的威胁更大。

**什么情况下才做**：中继服务器不完全可信（托管在第三方、多人共用、合规要求），且因为某些原因必须用姿势 B。届时作为[第 12 节](#12-路线-0自研应用层中继)自研中继的一个独立里程碑，放在中继本身跑通之后。

---

## 12. 路线 0：自研应用层中继

> **定位**：带宽成本与 frp **完全相同**，但换来产品化能力（设备管理、短码呼叫、统一入口、不要求用户装第二个程序）。**如果 frp 已能满足需求，这一节暂时不必实施。**

### 12.1 与 frp 的本质区别

两者在网络拓扑上**完全一样**（被控端出站 → 服务器配对转发 → 控制端接入），差别只在「谁实现出站」：

| | frp | 自研中继 |
| --- | --- | --- |
| 出站由谁实现 | frpc（额外进程） | **被控端自己**（新增 tokio task） |
| 部署 | 被控端要装 frpc + 配 frpc.toml | **单二进制，改个配置项即可** |
| 设备管理 | 无（每个设备手工分配一个 remotePort） | **设备注册表、短码呼叫、在线状态** |
| 多设备 | 每多一台要占一个公网端口 + 改 frps 配置 | **一个端口服务所有设备**，自动路由 |
| 前端入口 | 每台设备一个 `https://server:端口` | **统一入口**，列表里选设备 |
| 真实来源 IP | 丢失（见 [9.8 节](#98-安全边界必须知晓)的封禁连坐问题） | **保留**，`BruteForceGuard` 可正常工作 |
| 工作量 | **0** | 10~15 人日 |

**结论：设备数 ≤ 3 且只有你自己用 → frp 够了。设备多、要给别人用、要产品化 → 自研中继的价值才显现。**

### 12.2 设计要点（复用已有设计）

绝大部分设计与 WebRTC 路线**完全共享**，直接引用，不重复：

| 组件 | 设计位置 | 说明 |
| --- | --- | --- |
| 设备 ID / 短码模型 | [5.1 节](#51-设备-id-模型) | 原样适用 |
| 出站长连接保活与重连 | [5.2 节](#52-被控端出站长连接保活与重连) | 原样适用（30s 心跳、10s 超时、指数退避 + 抖动） |
| 控制端发起流程 | [5.3 节](#53-控制端如何发起) | 去掉 SDP/ICE 交换，配对成功后直接进入字节转发 |
| 消息格式 | [5.4 节](#54-信令消息格式草案) | 复用 register / registered / heartbeat / incoming_call / connect 等，**删掉 offer / answer / ice_candidate** |
| 认证分层 | [5.5 节](#55-与现有-argon2-认证的串联) | 原样适用 —— **中继不持有被控端密码，argon2 仍由被控端校验** |
| 滥用风险与限流 | [5.6 节](#56-滥用风险与限流) | 原样适用（短码枚举、虚假注册、放大攻击等） |
| 中继服务用什么写 | [4.8 节](#48-信令服务用什么写) | 同样推荐独立 crate + axum 0.7 |

**自研中继 = 信令服务 + 字节转发**。相比 WebRTC 路线，它**少了** PeerConnection 管理、ICE、STUN/TURN、分片层（WebSocket 无消息大小限制，不需要分片）；**多了**转发循环与带宽管理。

### 12.3 改动面：现有协议能否完全不改

**能，这是本路线最大的优势。** 逐项核实：

| 现有资产 | 是否需要改 |
| --- | --- |
| `pipeline.rs` 的 0xAB 二进制帧（16 字节头 + NAL） | **零改动** |
| `encoder.rs` H.264 编码 | **零改动** |
| `protocol.rs` 的 `ServerMsg` / `ClientMsg` JSON | **零改动**（只新增中继信令枚举，互不干扰） |
| `capture/`、`input/`、`clipboard.rs` | **零改动** |
| `web/app.js` 的 WebCodecs 解码路径（`app.js:425-500`） | **零改动** |
| `session.rs` argon2 认证与状态机 | **零改动** |
| `web.rs` 会话循环 | **需要传输层抽象**（见 [8.1 节 N0](#81-n0--前置准备与传输层抽象不引入-webrtc)）—— 这是唯一的核心改动 |
| 新增 | `signaling.rs`（出站客户端）、`crates/relay/`（中继服务） |

**中继服务只做不透明字节转发**：它把一条连接收到的字节原样写到配对的另一条连接，**不解析 0xAB 帧、不解析 JSON**。这样做有三个好处：攻击面小（没有解析逻辑就没有解析漏洞）、对协议变化免疫（将来改帧格式不用动中继）、实现简单（一个双向 `tokio::io::copy` 循环即可）。

**与 WebRTC 路线的共享度**：N0（传输层抽象）、信令协议、设备模型、中继服务骨架**全部共用**。所以「先做自研中继，将来再上 WebRTC」不是走弯路 —— WebRTC 只是在这套基础设施上多加一条 P2P 数据面。

### 12.4 带宽成本精确计算

> **这张表对所有过中继的方案通用**：frp、自研中继、Cloudflare Tunnel、WebRTC 的 TURN 中继。

**码率公式（已读代码核实，`encoder.rs:173-184`）**：

```rust
fn auto_bitrate(width: u32, height: u32, fps: u32) -> u32 {
    const BITS_PER_PIXEL: f64 = 0.10;
    const MIN_BITRATE: f64 = 2_000_000.0;      // 下限 2 Mbps
    const MAX_BITRATE: f64 = 25_000_000.0;     // 上限 25 Mbps
    let pixels = (width as f64) * (height as f64);
    let fps_factor = (fps.max(1) as f64 / 30.0).sqrt();
    let raw = pixels * 30.0 * BITS_PER_PIXEL * fps_factor;
    raw.clamp(MIN_BITRATE, MAX_BITRATE) as u32
}
```

默认配置是 `fps = 30`、`bitrate = -1`（自动），所以**实际码率由上式决定**。按此公式精确计算：

| 分辨率@帧率 | 实际码率 | 单向 GB/小时 | 中继双向合计 GB/小时 |
| --- | --- | --- | --- |
| 720p@15fps | 2.00 Mbps（触下限） | 0.90 | 1.80 |
| **720p@30fps** | **2.76 Mbps** | **1.24** | **2.49** |
| 720p@60fps | 3.91 Mbps | 1.76 | 3.52 |
| 1080p@15fps | 4.40 Mbps | 1.98 | 3.96 |
| **1080p@30fps（默认）** | **6.22 Mbps** | **2.80** | **5.60** |
| 1080p@60fps | 8.80 Mbps | 3.96 | 7.92 |
| 1440p@30fps | 11.06 Mbps | 4.98 | 9.95 |
| 4K@30fps | 24.88 Mbps | 11.20 | 22.39 |
| 4K@60fps | 25.00 Mbps（触上限） | 11.25 | 22.50 |

> GB 按十进制（1 GB = 10⁹ 字节）换算，与云厂商计费口径一致。

**并发会话的带宽需求（以默认 1080p@30fps = 6.22 Mbps 为基准）**：

| 并发会话 | 中继入向 | 中继出向 | 总吞吐 | 出向流量 GB/小时 |
| --- | --- | --- | --- | --- |
| 1 | 6.22 Mbps | 6.22 Mbps | 12.44 Mbps | 2.80 |
| 2 | 12.44 Mbps | 12.44 Mbps | 24.88 Mbps | 5.60 |
| 5 | 31.10 Mbps | 31.10 Mbps | 62.21 Mbps | 14.00 |
| **10** | **62.21 Mbps** | **62.21 Mbps** | **124.42 Mbps** | **27.99** |
| 20 | 124.42 Mbps | 124.42 Mbps | 248.83 Mbps | 55.99 |
| 50 | 311.04 Mbps | 311.04 Mbps | 622.08 Mbps | 139.97 |

**注意「进出双份」**：中继既要接收被控端推来的流，又要转发给控制端，所以**同一份视频在服务器上既占入向又占出向带宽**。云厂商通常只对**出向**计费，但**入向会占用网卡总吞吐**，带宽受限的小规格实例要按总吞吐判断。

**一台服务器能跑几路（按出向带宽）**：

| 服务器出向带宽 | 1080p@30 (6.22M) | 1080p@15 (4.40M) | 720p@30 (2.76M) |
| --- | --- | --- | --- |
| 5 Mbps | **0.8 路（跑不动 1 路！）** | 1.1 路 | 1.8 路 |
| 10 Mbps | 1.6 路 | 2.3 路 | 3.6 路 |
| 30 Mbps | 4.8 路 | 6.8 路 | 10.9 路 |
| 100 Mbps | 16.1 路 | 22.7 路 | 36.2 路 |
| 1 Gbps | 160.8 路 | 227.3 路 | 361.7 路 |

**关键警示：常见的入门云主机（1~5 Mbps 带宽）连一路 1080p@30 都跑不动。** 这是选中继方案前必须先确认的事 —— **请先查清服务器的实际出向带宽**，再看上表判断够不够。

### 12.5 中继模式下建议下调画质

**建议：是的，中继模式下应当默认降质。**

理由：带宽是中继方案唯一的硬成本，而降质的收益非常显著 —— 从 1080p@30 降到 720p@30，码率从 6.22 降到 2.76 Mbps，**省 56%**；降到 720p@15 则是 2.00 Mbps，**省 68%**。而远程控制的主要场景（看文字、点菜单、改配置）对分辨率的敏感度远低于对**延迟和流畅度**的敏感度。

具体建议（按优先级）：

1. **优先降帧率而非分辨率**。1080p@15 = 4.40 Mbps（省 29%），文字依然清晰可读，只是鼠标拖动略顿。如果用户主要是看和点，15fps 完全够用。
2. **分辨率敏感的场景保持 1080p**，因为降分辨率会让小字模糊，直接影响可用性。
3. **显式配置而非自动**：中继模式下建议把 `video.bitrate` 从 `-1`（自动）改为显式值。手工设 2 Mbps 时每路只占 0.90 GB/小时，10 路出向只需 9.0 GB/小时 —— 可控性强得多。

配置示例（中继/隧道场景推荐起点）：

```toml
[video]
fps = 15          # 从 30 降到 15，省约 29% 码率
bitrate = 2       # 单位 Mbps，显式指定而非自动估算（2 Mbps 约 0.90 GB/时）
codec = "h264"
```

> 注意：这两个配置项**现在就可以调**（`config.rs:70-78` 已支持），不需要等任何开发。用 frp 跑通后如果觉得卡或流量大，**第一件事就是调这两个值**。

### 12.6 中继服务的新增配置项

中继服务自身配置（`crates/relay/relay.toml`，草案）：

```toml
bind = "0.0.0.0:9443"
register_token = "与被控端一致的长随机串"

[tls]
cert = "/etc/letsencrypt/live/xxx/fullchain.pem"
key  = "/etc/letsencrypt/live/xxx/privkey.pem"

[limits]
max_devices = 100                  # 注册设备上限
max_sessions = 20                  # 并发会话上限（按 12.4 节的带宽表设定）
max_bandwidth_mbps = 100           # 总带宽上限，超出拒绝新会话
session_timeout_secs = 3600
connect_rate_per_min = 10          # 单 IP 呼叫频率限制（防短码枚举）
max_message_bytes = 4194304        # 单消息上限，防放大攻击
```

被控端侧配置复用 [8.6 节](#86-新增配置项草案)的 `[signaling]` 段（把 `url` 指向中继服务即可），字段含义相同。

### 12.7 工作量与里程碑

复用 [8.1 节 N0](#81-n0--前置准备与传输层抽象不引入-webrtc) 的传输层抽象，之后：

| 阶段 | 内容 | 工作量 |
| --- | --- | --- |
| **R0** | 传输层抽象（= N0，但**不需要分片层**，WebSocket 无消息大小限制） | 2~4 人日 |
| **R1** | 中继服务 crate（注册、短码、配对、字节转发、基础限流）+ 被控端出站客户端 | 5~8 人日 |
| **R2** | 限流完善、带宽管理、设备列表前端、审计日志 | 3~5 人日 |
| R3（可选） | 应用层 E2E 加密（见 [11.3 节](#113-如果要做到中继看不见内容)） | 4~6 人日 |

**合计 10~17 人日**（不含 R3），明显低于 WebRTC 路线的 23~38 人日，且**没有 edition 2024 / rustc 1.85 的工具链门槛**（不引入 webrtc crate）—— 这对龙芯等国产环境是实质优势。

---

## 13. 成熟开源方案：什么时候该用现成的

### 13.1 两个主要候选

**RustDesk**（[rustdesk.com](https://rustdesk.com)）
- Rust 写的，与本工程同语言同生态。
- **自带中继方案**：`hbbs`（ID/注册服务器）+ `hbbr`（中继服务器），**支持完全自建**，不依赖官方服务器。架构思路与[第 12 节](#12-路线-0自研应用层中继)高度相似（设备 ID + 中继），某种意义上它就是「路线 0 的成熟实现」。
- 客户端覆盖 Windows / Linux / macOS / Android / iOS / Web。
- 功能完整：文件传输、多显示器、TCP 隧道、权限管理。

**MeshCentral**（[meshcentral.com](https://meshcentral.com)）
- Node.js 写的，Web 端管理为主。
- 偏**批量设备运维管理**（资产清单、批量命令、Intel AMT 支持），远程桌面只是其中一个功能。
- 适合管理几十上百台机器的 IT 运维场景。

> 坦白标注：以上为这两个项目的**公认特性描述**，本轮**未逐条实查官方文档**（本次调研的核实预算优先投给了 frp 配置、Cloudflare 条款、crate 版本这些直接影响推荐结论的事实）。选用前请自行核实功能细节与许可证条款。

### 13.2 本工程继续存在的价值（客观评估）

这不是客套 —— 本工程有几个成熟方案给不了的实质优势：

| 价值 | 说明 |
| --- | --- |
| **已适配国产环境** | commit `572478a` 做了龙芯架构适配，`c0e8682`/`9ee4972` 实现了 Wayland 支持（Xwayland 桥接）。**这类适配在上游项目里往往排不上优先级**，自己掌控代码才能及时解决 |
| **单二进制、依赖可控** | 当前 `Cargo.lock` 249 个 package，刻意统一 ring 后端避开 aws-lc-sys 的 C 编译负担（`Cargo.toml` 注释明确记录了这个决策）。**这种对依赖链的精确控制，是交叉编译/信创环境能否顺利落地的关键** |
| **代码量小、可审计** | `crates/server/src` 共 4386 行（已核实 `wc -l`）。远程控制软件拿到的是用户桌面的完全控制权，**能把全部代码读完审一遍，这个性质在高安全要求场景下价值很高** |
| **无第三方服务依赖** | 现状完全自包含（自签证书 + 本地 listen），不需要任何外部服务。采用 frp 后也只依赖用户自己的服务器 |
| **按需裁剪** | 不需要文件传输、不需要移动端、不需要设备管理 —— 没有的功能就没有对应的攻击面和维护负担 |
| **完整的设计文档** | `docs/` 下已有 01~05 + Windows 专项，本文又补齐了穿透方案。**这是可持续维护的基础** |

### 13.3 判断：什么场景该自建，什么场景该用现成的

**该用现成方案（RustDesk）的场景**：
- 目标就是「尽快有个能用的远程控制」，功能需求是通用的（看屏幕、控鼠标键盘、传文件）；
- 需要移动端、多显示器、文件传输等本工程暂未实现的功能；
- 设备数量多、要做权限管理；
- 没有持续投入开发维护的人力。

**该继续自建（本工程）的场景**：
- **目标平台特殊**（龙芯、麒麟、统信等信创环境），上游支持不及时或不完整 —— **这正是本工程已经投入并解决了的事**；
- **有审计/合规要求**，需要能读完全部代码、能控制每一个依赖；
- 需求是**裁剪过的子集**，不想引入用不到的功能和攻击面；
- 需要深度定制（特定的输入注入行为、特定的编码参数、嵌入到自有系统）；
- 已有的技术积累值得延续（本工程已完成 M0~M4 + Windows + Wayland + 龙芯适配，**沉没成本不是沉没的，是资产**）。

**一个务实的建议**：这两者不互斥。**可以先用 RustDesk 顶上日常需求，同时本工程按自己的节奏继续演进** —— 尤其是本工程在信创环境上的适配工作，是 RustDesk 替代不了的。反过来说，**如果发现本工程的需求其实完全被 RustDesk 覆盖、且不涉及特殊平台，那么停止重复开发、转为「调研并采用」确实是更理性的选择** —— 这正是用户「没必要重复造轮子」那句话的合理之处。

> 判断的分水岭是一个具体问题：**你的目标平台 RustDesk 支持得好吗？** 如果是标准 x86 Windows/Linux，答案大概是「支持得很好，用它」。如果是龙芯 + Wayland，答案可能是「本工程已经跑通了，继续」。

---

## 附录 A：已核实事实与推测的边界

### A.1 已核实 —— 读过本仓库代码

| 结论 | 代码位置 |
| --- | --- |
| 被控端只 listen，无任何 outbound | `web.rs:141`（`bind_rustls`）、`web.rs:145`（`TcpListener::bind`）；全工程无 `connect` 调用，`Cargo.toml` 无 HTTP 客户端依赖 |
| 前端固定回连当前 host | `web/app.js:53-54` |
| 帧头 16 字节，含 2 字节未用 `reserved` | `pipeline.rs:233-247` |
| **逐 NAL 下发**（非整帧） | `pipeline.rs:184-189` 对每个 `EncodedPacket` 单独序列化发送；`web.rs:227` 逐个 `Message::Binary` |
| broadcast 容量硬编码 8，Lagged 丢旧帧且**未请求关键帧** | `web.rs:82`（容量）、`web.rs:233-236`（Lagged 分支只有 debug 日志） |
| argon2 认证在连接建立后由被控端校验 | `web.rs:360` → `session.rs:82` |
| **BruteForceGuard 按 peer IP 封禁**（中继场景会失效） | `web.rs:346`（`is_banned`）、`web.rs:415`（`record_failure`），均传 `peer_addr.ip()` |
| 认证失败固定延迟 800ms | `web.rs:27` |
| 刻意统一 ring、避开 aws-lc-sys | `crates/server/Cargo.toml` TLS 段注释原文 |
| 编码器码率在 `new()` 时固定，无运行期调整接口 | `encoder.rs:36-78` |
| `AppConfig` 已用 `#[serde(default)]`，新增字段向后兼容 | `config.rs:7` |
| 前端 WebCodecs 解码逻辑（SPS 触发 configure、IDR 内联 SPS/PPS） | `web/app.js:425-500` |
| 根 Cargo.toml 有 `[patch.crates-io]` 指向 vendor/openh264-sys2 | `Cargo.toml` |
| `Cargo.lock` 共 249 个 package | `grep -c '^\[\[package\]\]' Cargo.lock` |
| lock 中 rustls 0.23.40 / ring 0.17.14 / rcgen 0.13.2 / tokio 1.52.3 | `Cargo.lock` |
| **`docs/design/` 目录此前不存在**（本文为该目录首个文档） | `ls docs/` 只有 01~05 + README + windows/ |

### A.2 已核实 —— 查过 crates.io / docs.rs / GitHub（2026-10 实查）

| 结论 | 来源 |
| --- | --- |
| `webrtc` 最新 0.21.1，发布于 2026-10-11，共 72 个版本，MIT/Apache-2.0 | crates.io API `/crates/webrtc` |
| `default = ["runtime-tokio", "crypto-ring"]`；`crypto-aws-lc-rs` 为 opt-in | 同上（feature 列表） |
| webrtc 0.21.1 仅 11 个直接依赖，协议栈全在 `rtc` crate | crates.io API `/crates/webrtc/0.21.1/dependencies` |
| `rtc` 0.21.1 依赖 `rustls ^0.23.27`（仅 std）、`rcgen ^0.14.8`（仅 pem）+ 18 个 rtc-* 子 crate | crates.io API `/crates/rtc/0.21.1/dependencies` |
| `rtc-crypto` 0.21.1 的 `ring ^0.17.14` 与 `aws-lc-rs ^1.17.3` **均为 optional** | crates.io API `/crates/rtc-crypto/0.21.1/dependencies` |
| 全链路**无 openssl/openssl-sys 非可选依赖**；openssl 仅为 rcgen 的 unix dev-dependency | 同上 + `/crates/rcgen/0.14.8/dependencies` |
| rcgen 0.14.8 的 `ring`、`aws-lc-rs` 均为 optional | crates.io API `/crates/rcgen/0.14.8/dependencies` |
| `crypto-ring` / `crypto-aws-lc-rs` 是 **additive**，ring 仍为默认，传递依赖无法悄悄替换；后端按 PeerConnection 粒度选 | GitHub `webrtc-rs/webrtc` README |
| 0.17.x → 0.20 是重写级改动：edition 2021→2024，crate 代码量 ~20783 → ~3235 行 | crates.io 版本元数据对比 |
| 0.21.x 是 **edition 2024**（即 rustc ≥ 1.85） | crates.io API `/crates/webrtc`（`edition` 字段） |
| README 自述 pre-1.0，「minor version bump may include breaking changes」，1.0 工作跟踪于 issue #836 | GitHub README |
| 社区热度 5.2k stars / 531 forks / 3820 commits | GitHub README |
| 官方带 39 个可运行示例（data channel、media playback、simulcast、ICE restart、insertable streams） | GitHub README |
| 0.21 修复：可靠 channel 用背压代替静默丢消息；SCTP T1 RTO 修正为 1s | GitHub README |
| **`MIME_TYPE_H264` 存在**，`register_default_codecs()` 默认集含 H264 | docs.rs `/rtc/0.21.1/.../media_engine/index.html` |
| `media_engine` 共 15 个 MIME 常量（含 AV1、HEVC） | 同上 |
| `rtc::data_channel` 提供 `RTCDataChannel` / `RTCDataChannelInit` / `RTCDataChannelMessage` | docs.rs `/rtc/0.21.1/rtc/` |
| webrtc 0.21.1 的 `data_channel` 模块含 send-buffer back-pressure | docs.rs `/webrtc/0.21.1/webrtc/` |
| 0.21 顶层**已无 `api` 模块**（`api::media_engine` 那套旧 API 不再适用） | 同上 |
| `media_stream` 提供 `TrackLocalStaticSample` / `TrackLocalStaticRTP` | 同上 |
| `rtc` 声明支持 STUN/TURN、Trickle ICE、ICE restart；`rtc-turn` 是实际依赖 | docs.rs `/rtc/0.21.1/rtc/` |
| webrtc 0.17.2 是 edition 2021，依赖 `ring ^0.17.14`（required）、`rcgen ^0.13`，**无 rustls/aws-lc** | crates.io API `/crates/webrtc/0.17.2/dependencies` |
| str0m 最新 0.24.1（2026-10-03），38 个版本，下载 278 万 | crates.io API `/crates/str0m` |
| str0m `default = ["aws-lc-rs", "examples"]`，可选后端 openssl/rust-crypto/apple-crypto/wincrypto，**无 ring 后端** | 同上（feature 列表） |
| Google 提供公共 STUN（`stun.l.google.com:19302`），**不提供公共 TURN**；官方态度是「开发可用、无 SLA、商业部署应自建」 | [discuss-webrtc 邮件组](https://groups.google.com/g/discuss-webrtc/c/shcPIaPxwo8/m/F8Vc8QlIyn0J)、[StackOverflow](https://stackoverflow.com/questions/65660261/) |
| coturn 是自建 TURN 的事实标准 | [github.com/coturn/coturn](https://github.com/coturn/coturn)；多个部署讨论佐证 |
| Chrome DataChannel 上限实测 **262144 字节**（报错原文 "can send a maximum of 262144 bytes"） | [StackOverflow](https://stackoverflow.com/questions/60647381/how-to-increase-channelmessage-max-size-with-simplewebrtc-sdk) |
| 规范建议互操作安全值 64 KiB；社区实测 16 KiB 分片吞吐最优 | [Mozilla WebRTC 博客](https://blog.mozilla.org/webrtc/large-data-channel-messages/)、[StackOverflow](https://stackoverflow.com/q/35381583) |

### A.2.1 已核实 —— 本轮新增（frp / Cloudflare / 加密 / 码率）

| 结论 | 来源 |
| --- | --- |
| `auto_bitrate()` 公式：`pixels × 30 × 0.10 × √(fps/30)`，clamp 到 2~25 Mbps | `encoder.rs:173-184`（读代码） |
| 1080p@30fps 实际码率 **6.22 Mbps**；720p@30 为 2.76 Mbps；1080p@15 为 4.40 Mbps | 按上式计算（[12.4 节](#124-带宽成本精确计算)完整表格） |
| frp **v0.52.0 起支持 TOML/YAML/JSON**，INI 已废弃且新特性不再支持 | frp 官方 README |
| frp 配置键名（frps: `bindPort`/`allowPorts`/`auth.token`/`auth.method`/`webServer.*`/`maxPortsPerClient`/`transport.heartbeatTimeout`/`transport.tls.force`；frpc: `[[proxies]]` 的 `name`/`type`/`localIP`/`localPort`/`remotePort`、`transport.tls.enable`/`certFile`/`trustedCaFile`/`useSystemCa`） | frp README + `conf/frps_full_example.toml` |
| frp **v0.50.0 起 `transport.tls.enable` 默认 true** | frp README |
| **frpc 默认不校验服务端证书**（"does not verify the server certificate"），需配 `useSystemCa` 或 `trustedCaFile` | frp README |
| frps 默认值：`maxPortsPerClient = 0`（无限制）、`transport.heartbeatTimeout = 90`、`transport.maxPoolCount = 5`、`webServer.addr` 默认 `127.0.0.1` | `conf/frps_full_example.toml` |
| `auth.method` 支持 `token`（默认）/ `oidc`，可加 `auth.additionalScopes` | frp README |
| **Cloudflare 条款：非 Enterprise 客户通过 CDN 提供视频必须购买付费服务**（点名 Developer Platform / Images / Stream），Cloudflare 可停用或限制 CDN 访问 | [Service-Specific Terms](https://www.cloudflare.com/service-specific-terms-application-services/) 的 "Content Delivery Network (Free, Pro, or Business)" 标题下（该处无条款编号） |
| **网传的「2.8 条 Limitation on Serving Non-HTML Content」在现行协议中已不存在** —— 现行 Self-Serve Subscription Agreement（2025-09-12 版）全文无此标题，也不提 video/audio/streaming | [cloudflare.com/terms](https://www.cloudflare.com/terms/) 实查 |
| 浏览器 Web Crypto **支持 X25519 / AES-GCM / HKDF / PBKDF2**；**不支持 ChaCha20-Poly1305，不支持 Argon2** | [MDN SubtleCrypto](https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto) 算法表 |
| `snow` crate（Noise 实现）最新 0.10.0（2025-07-19），下载 2980 万，活跃维护；**有 `ring-resolver`/`ring-accelerated` feature，无 aws-lc 依赖**；edition 2024 / MSRV 1.85 | crates.io API `/crates/snow` |
| `crates/server/src` 共 **4386 行** Rust | `wc -l`（读代码） |
| TURN 是 RFC 8656 定义的**协议**，coturn 是最常用的**开源实现** | RFC + coturn 仓库 |

### A.3 推测 / 待实测 —— 不可作为决策依据

| 项 | 为什么没核实 | 如何验证 |
| --- | --- | --- |
| 引入 webrtc 后新增 60~90 个 package | 需要 `cargo tree`，本轮按规则不执行构建 | `cargo add webrtc --dry-run` 或 `cargo tree` |
| 各目标平台（尤其龙芯）rustc 是否 ≥ 1.85 | 无该环境 | 各构建机 `rustc --version` |
| webrtc-rs SCTP 的实际吞吐与延迟表现 | 需联调 | N1 第一步局域网实测 |
| 1080p IDR NAL 的实际大小分布 | 需运行 | 在 `pipeline.rs` 加临时日志统计 NAL 长度分布 |
| openh264 crate 0.6 是否暴露动态码率接口（`ENCODER_OPTION_BITRATE`） | 影响 N4，当前不阻塞 | 查 openh264 0.6 的 docs.rs |
| openh264 输出的 H.264 profile 能否被浏览器媒体轨硬解 | 影响 N4 | N4 实测 |
| `TrackLocalStaticSample` 对 Annex-B 输入的处理方式 | 影响 N4 | 读 rtc-media 源码或实测 |
| `RTCIceServer` 的 TURN 凭据字段名 | docs.rs 示例只给了 `urls` + `..Default::default()` | 查 `RTCIceServer` 结构体定义 |
| **STUN 打洞成功率 80%~90%** | **本轮尝试核实但未找到可引用的权威实测数据**，详见下方说明。文中所有成功率数字均为业界流传的估计值 | 自行用 `getStats()` 统计 `candidateType == "relay"` 被选中的比例 |
| frp 的 `type = "http"` 模式对 WebSocket 升级的支持 | 本场景无需用 http 模式（tcp 透传已足够），未深入核实 | 如确需用，查 frp 官方 http 代理文档 |
| Cloudflare WebSocket 的具体限制（空闲回收时间、并发上限） | 官方文档页抓取失败 | 查 Cloudflare 官方 WebSocket 文档 |
| frp 最新版本号 | GitHub Releases 区块未渲染出版本号 | 看 [Releases 页](https://github.com/fatedier/frp/releases)，只需 ≥ v0.52.0 |
| frp 转发的延迟/吞吐具体数值 | 需实测 | 用 frp 跑通后对比直连的 RTT |
| RustDesk / MeshCentral 的功能细节与许可证 | 本轮核实预算优先给了直接影响结论的事实 | 选用前查各自官方文档 |

#### 关于打洞成功率：一次失败的核实，以及它对结论的影响

这个数字本来是判断「WebRTC 值不值得做」的关键依据，所以我专门去查了常被引用的几个来源，结果如下：

| 来源 | 实查结果 |
| --- | --- |
| [Tailscale "How NAT traversal works"](https://tailscale.com/blog/how-nat-traversal-works) | 文中确实有「direct connection over 90% of the time」，但**原文明确标注这是估计（a guess）而非实测数据**，且其方案用了端口预测、UPnP/NAT-PMP/PCP 等超出标准 ICE 的技术，不能直接套用到 WebRTC |
| [Protocol Labs "Decentralized hole punching" 论文页](https://research.protocol.ai/publications/decentralized-hole-punching/) | **页面只有摘要，无任何成功率数字、无 QUIC/TCP 分项、无样本量** |
| [punchr 仓库 README](https://github.com/dennis-tra/punchr)（libp2p 打洞测量工具） | **只有工具说明和结果分类定义，无实际测量结果**。原文自述目的是 "to measure the hole punching success rate" —— 是目标，不是结论 |
| [probelab.io](https://probelab.io/) | 无打洞测量数据 |
| [libp2p 打洞博客](https://blog.ipfs.tech/2022-01-20-libp2p-hole-punching/) | 概念说明文，无测量数据 |

**所以文中出现的「80%~90%」「10%~20% 需要 TURN」这类数字，请一律视为业界流传的估计值，不要当作可靠依据做决策。** 这个核实失败本身是支撑 [7.4 节](#74-推荐结论现在不做-webrtc)结论的理由之一 —— **在收益大小不明的情况下，先用零成本方案（frp）验证需求，比先投入 23~38 人日更稳妥。** 真要这个数字，唯一可靠的办法是自己用 `getStats()` 在真实用户网络上统计。
| 2 字节 `reserved` 是否够放分片信息 | 设计判断：若用 12 bit 做 chunk_index（最大 4095 片 × 16 KB = 64 MB）+ 4 bit 标志位，对本场景充裕 | N0 设计评审时确认 |

---

## 附录 B：需要决策的点

按需要决策的紧迫程度排序：

> **说明**：下面 B.1~B.6 是**自研路线（WebRTC / 应用层中继）的决策点**。按 [0.1 节](#01-推荐做什么)的推荐，当前阶段只需关注 B.0；B.1~B.6 等到真要启动自研时再决策。

### B.0 [当前唯一需要决策的事] 先用 frp 验证需求，还是直接投入自研

**我的建议：先用 frp。** 本工程零改动，一下午能跑通，然后用真实体验回答三个问题：

1. 外网控制的**延迟**能接受吗？（决定是否需要 WebRTC 的 P2P）
2. 服务器**带宽**够吗？（对照[带宽表](#124-带宽成本精确计算)，先查清服务器实际出向带宽）
3. **设备数**会增长吗？要给别人用吗？（决定是否需要[自研中继](#12-路线-0自研应用层中继)的设备管理能力）

这三个问题现在都只能靠猜，而 frp 能用一下午的成本把它们变成事实。**在此之前投入 10~38 人日自研，是在为未经验证的需求付费。**

需要你确认的具体事项：
- **服务器的实际出向带宽是多少？**（这是最关键的一个数字，决定能跑几路，也决定要不要[降质](#125-中继模式下建议下调画质)）
- 接受 frp 姿势 A 的**证书告警**吗？（接受 → 零改动且服务器看不到明文；不接受 → 走姿势 B，但服务器能看到明文）
- 预计有几台被控端设备？（≤ 3 台 frp 足够；更多则[自研中继](#121-与-frp-的本质区别)的价值开始显现）

### B.1 [阻塞 N1] 各目标平台的 rustc 版本是否 ≥ 1.85

webrtc 0.21.x 是 edition 2024，这是硬门槛。**N1 开工前必须确认**，尤其是龙芯交叉编译链（本工程 commit `572478a` 做过龙芯适配）。

- 若满足 → 用 0.21.1，按本文方案执行。
- 若不满足 → 两个选项：(a) 升级工具链；(b) 降级用 webrtc 0.17.2（edition 2021，同样 ring 后端，已核实），代价是旧 API + 该版本线维护力度较弱、未来升级断档。

### B.2 [影响方案取舍] 目标用户规模与运维能力

这直接决定「WebRTC 是否值得这 23~38 人日」（见 [7.4 节](#74-推荐结论现在不做-webrtc)）：

- **如果是少量设备 + 用户有运维能力** → **先不做 WebRTC**，用[第 9 节的 frp 方案](#9-路线-zfrp-反向隧道-当前推荐本工程零改动)，零代码零风险。**这也是本文的最终推荐。**
- **如果要做成产品级、用户不应碰网络配置、设备数量会增长** → 先做[路线 0 自研中继](#12-路线-0自研应用层中继)（10~17 人日），它已能提供设备管理与统一入口；只有在带宽成本确实成为瓶颈时才追加 WebRTC。

**这个问题需要产品侧输入，不是技术能单方面决定的。** 判断依据见 [B.0](#b0-当前唯一需要决策的事-先用-frp-验证需求还是直接投入自研)。

### B.3 [影响 N2 预算] TURN 服务的部署形态与成本上限

- 自建 coturn（云主机 + 带宽费）还是买托管 TURN？
- 带宽预算上限是多少？这决定 `max-bps` / `total-quota` 怎么配，以及是否需要「中继时自动降码率」。
- 部署在哪个地域？TURN 的地理位置直接影响中继延迟。

### B.4 [影响 N1 设计] 信令服务的部署归属

- 由项目方统一提供一个公共信令服务，还是让用户自己部署？
  - 统一提供：用户体验好，但项目方承担运维和滥用风险，且涉及隐私（能看到所有设备的在线状态和 IP）。
  - 用户自部署：无隐私顾虑，但提高使用门槛。
  - **建议：代码支持自部署（配置项填 URL），同时项目方可选地提供一个公共实例。**
- `register_token` 的分发机制？（预共享密钥最简单；设备证书更安全但要做 PKI）

### B.5 [影响 N2 排期] DTLS 指纹防 MITM 是否必须在 N2 完成

[5.5 节](#55-与现有-argon2-认证的串联)指出信令服务理论上能做 MITM。我的判断是：
- N1 可以暂不实现（相比现状「自签证书 + 浏览器点通过」，安全性没有降低）；
- **N2 必须补上**（用 argon2 密码派生密钥给指纹签名，成本不高）。

如果信令服务由用户自部署（B.4 选自部署），这条风险等级会下降 —— 因为信令服务就是用户自己的，不存在「不可信的第三方信令」。**两个决策相关联。**

### B.6 [可独立决策] 是否顺带把 rcgen 从 0.13 升到 0.14

好处：消除两份 rcgen 并存编译。
代价：API 有小变动，需要改 `tls.rs`。
**建议：作为独立的小改动单独评估，不要和穿透功能混在一个 PR 里**（便于回滚和定位问题）。

---

## 附录 C：术语对照

| 缩写 | 全称 | 本文含义 |
| --- | --- | --- |
| **STUN** | Session Traversal Utilities for NAT | 问「我的公网地址是什么」，用于打洞。Google 提供公共服务 |
| **TURN** | Traversal Using Relays around NAT | 中继转发，打洞失败时的兜底。**必须自建**（coturn） |
| **ICE** | Interactive Connectivity Establishment | 候选收集 + 连通性检查 + 自动择优的框架 |
| **SDP** | Session Description Protocol | 描述媒体/数据能力的文本格式，经信令交换 |
| **DTLS** | Datagram Transport Layer Security | UDP 上的 TLS，WebRTC 强制使用 |
| **SRTP** | Secure Real-time Transport Protocol | 加密的 RTP，用于媒体轨 |
| **SCTP** | Stream Control Transmission Protocol | DataChannel 的底层传输，跑在 DTLS 上 |
| **srflx** | server reflexive | STUN 问到的公网候选 |
| **relay** | — | TURN 分配的中继候选 |
| **PLI** | Picture Loss Indication | 接收端「我丢帧了，请发关键帧」的 RTCP 反馈 |
| **NACK** | Negative Acknowledgement | 「某个包我没收到，请重传」 |
| **TWCC** | Transport-wide Congestion Control | WebRTC 的带宽估计反馈机制 |
| **CGNAT** | Carrier-Grade NAT | 运营商级 NAT，用户无法做端口映射 |
| **TOFU** | Trust On First Use | 首次连接时确认并记住对端身份（类似 SSH host key） |

