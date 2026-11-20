# XuanCi 扩展规范

## v1：声明式提示词模板（已实现）

模板文件：`%LOCALAPPDATA%\XuanCi\templates.json`

```json
[
  {
    "id": "polish",
    "name": "润色",
    "system_prompt": "你是一名中文编辑。在不改变原意的前提下改善表达。只输出润色后的文本。",
    "user_template": "请润色以下内容：\n{{text}}",
    "temperature": 0.4
  }
]
```

规则：

- `id`、`name`、`user_template` 必填；`user_template` 必须包含 `{{text}}`。
- `{{text}}` 会被替换为用 `<selected_text>...</selected_text>` 包裹的选中文本。
- 系统提示会自动追加数据边界声明，防止选中文本中的指令影响模型。
- 无效项被跳过；JSON 解析失败时回退到内置模板。
- 可通过托盘「动作模板」子菜单添加、编辑、删除，保存后立即重建工具条；直接修改 JSON 文件则需重启 XuanCi。
- 模板动作显示在工具条右侧，点击后直接发起流式请求。

安全边界：模板只能影响发给模型的文本，不能执行代码、访问文件或读取 API Key。

## v1.5：WASM 插件（规划）

选择 WASM 而不是原生 DLL 的原因：

- Rust 没有稳定 ABI，原生 DLL 插件需要不安全的 C ABI，升级宿主即可崩溃。
- WASM 可沙箱化：默认无文件、无网络、无环境变量访问。
- 可设置燃料/超时，插件死循环时能可靠终止。
- 插件可用 Rust/C/AssemblyScript 编写，跨语言。

### 目录与清单

```
%LOCALAPPDATA%\XuanCi\plugins\com.example.polish\
  plugin.json
  plugin.wasm
```

```json
{
  "id": "com.example.polish",
  "name": "润色",
  "version": "1.0.0",
  "apiVersion": "1.0",
  "entry": "plugin.wasm",
  "actions": [
    {
      "id": "polish",
      "name": "润色选中文本",
      "description": "改善表达，保持原意",
      "order": 200
    }
  ]
}
```

### 宿主 ABI（草案）

插件导出：

```wat
;; 返回 JSON 元数据
(export "xuanci_manifest" (func ...))
;; 执行动作：输入 JSON，返回最终文本
(export "xuanci_execute" (func (param i32 i32) (result i32)))
```

宿主通过 `wasmtime` 注入一个受限的 `host` 模块：

| 导入 | 能力 |
|---|---|
| `host.stream_chat(request_json) -> i32` | 发起 LLM 请求，返回句柄 |
| `host.poll_chunk(handle) -> i32` | 读取增量文本 |
| `host.log(level, message)` | 写入诊断日志（默认关闭） |

能力白名单：

- 默认无网络、无文件系统。
- 需要额外能力时由用户在设置页逐项授权。
- 插件拿不到 API Key，只能通过宿主代理发起请求。
- 单次执行默认 120 秒超时，超时即终止实例。

### 加载策略

- 启动时只读 `plugin.json`，不加载 wasm。
- 首次点击动作时创建 `wasmtime` 实例，按需编译并缓存。
- 执行结束、禁用或卸载时释放实例。
- 插件异常不会影响主进程。

## v2：Markdown 渲染器（已实现内置版，插件版规划）

内置渲染器位于 `crates/xuanci-core/src/markdown.rs`，把 CommonMark + GFM 转成 RTF，由结果窗通过 `EM_STREAMIN` 显示。第三方库只使用 `pulldown-cmark`（解析）和 `syntect`（代码高亮），不执行 HTML/JavaScript，不自动下载远程图片。

未来的渲染器插件建议采用以下边界：

- 插件只接收 Markdown 和渲染选项，返回**受限文档模型**（标题、段落、列表、代码块、表格、链接、图片占位等），不直接返回任意 RTF。
- 插件运行在独立进程或 WASM 沙箱中，拿不到 API Key、选中文本以外的内容、文件系统和网络。
- 宿主负责把文档模型转成 RichEdit RTF，插件异常或超时时自动回退到内置渲染器。
- 不采用原生 DLL 和 WebView 作为渲染器插件形态。

Markdown 中的图片不会被读取或下载，只显示 `[图片: alt]` 占位文本。

### LaTeX 公式插件（已实现，可选安装）

`plugins/latex` 是一个独立的可选插件，把 LaTeX 公式转换为带样式的文本片段。主程序**不包含、不自动安装**它；未安装时公式显示为源码。

- 插件协议：向 `xuanci-latex.exe` 的 stdin 写一行 JSON，从 stdout 读取 `{ok, segments:[{text, style}]}`，`style` 为 `normal` / `super` / `sub`；不创建临时文件。
- 宿主在解析 Markdown 前会把 `\(...\)` / `\[...\]` 统一转换为 `$...$` / `$$...$$`，代码围栏和行内代码中的内容不受影响。
- 不生成图片；分式、根号用括号文本表示，上下标使用 RichEdit 上下标样式。
- 宿主通过独立进程调用，15 秒超时；失败、超时、输出非法或插件缺失时回退源码。
- 插件只接收公式文本，不读取 API Key、选中文本或网络。
- 安装位置：`%LOCALAPPDATA%\XuanCi\plugins\latex\xuanci-latex.exe`。
- 安装/卸载/启用开关都在托盘「LaTeX 公式」子菜单中。

之前曾评估用 RaTeX 渲染为 PNG 的方案，但当前 Win32 结果窗的 RichEdit 控件无法可靠绘制图片对象，因此改为纯文本转换。

## 不采用的方案

- **原生 DLL**：无稳定 ABI、无沙箱、崩溃可拖垮主进程。
- **进程内脚本（Lua/JS）**：需要引入运行时，且同样缺少可靠沙箱。
- **WebView 插件面板**：违背“无浏览器内核、低占用”的目标。
