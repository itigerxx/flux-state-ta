# 🌊 FluxState-TA

**一个面向高频交易的流式技术分析库。**

`FluxState-TA` 专为“数据持续不断流入”的交易环境而设计。不同于传统批处理型指标库，它内部维护了一个 ** Stateful **，能够同时优雅处理实时价格波动与 K 线最终收盘，从根源上避免高波动市场中的状态污染问题。

---

## 🚀 为什么选择 FluxState-TA？

在高频交易或实时流式交易场景中，“**未完成 K 线（Unfinished Candle）**” 一直是一个核心痛点。  
`FluxState-TA` 在引擎层面区分 **瞬时更新（Transient Ticks）** 与 **最终确认收盘（Confirmed Closures）**，专门解决这一问题。

### 核心特性

* **💎 有状态架构**：内置增量状态管理，无需手动维护历史数据或 K 线周期切换逻辑。
* **⚡ 流式优先**：单次更新复杂度为 $O(1)$，非常适合 WASM 策略运行环境。
* **🛡️ 防污染机制**：自动处理同一根 K 线的多次更新，避免中间价格噪音污染历史状态。
* **🧩 幂等设计**：通过 `open_time` 校验机制，天然抵抗重复消息与乱序数据。

---

## 📦 安装方式

将以下内容添加到你的 `Cargo.toml`：

```toml
[dependencies]
flux-state-ta = { git = "https://github.com/itigerxx/flux-state-ta.git" }