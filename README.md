# XuanCi 划词助手

Windows 全局划词工具：在任意软件中选中文字，自动弹出小工具条，一键翻译或交给 AI；也支持全局快捷键。

- 原生 Rust + Win32，无 WebView / 无浏览器内核 / 无遥测
- 兼容任意 OpenAI 格式接口（DeepSeek、OpenAI、硅基流动、通义、智谱、Moonshot、Ollama、LM Studio 等），SSE 流式输出
- 多服务商配置：根据 Base URL 自动识别服务商，每个服务商独立保存地址、模型和 Key，托盘一键切换
- API Key 使用 Windows DPAPI 加密保存在本机
- 声明式提示词模板，改 JSON 即可扩展动作

## 已实现

| 能力 | 说明 |
|---|---|
| 划词自动弹条 | 鼠标拖选或双击选词后，在光标附近弹出「翻译 / AI 问答 / 自定义模板」 |
| 全局快捷键 | `Ctrl+Alt+D` 翻译，`Ctrl+Alt+Q` 让 AI 解释（v1 固定，占用时会提示） |
| 取词 | UI Automation 优先；UIA 失败时用 Ctrl+C 剪贴板回退并恢复原剪贴板（热键与自动弹条均可，可在托盘关闭） |
| 安全策略 | 密码框拒绝、管理员程序提示、终端禁用 Ctrl+C、文件/图片剪贴板回退保护 |
| 流式结果窗 | 可滚动、复制纯文本、复制 Markdown、停止、关闭 |
| Markdown 渲染 | 流式阶段显示纯文本，完成后渲染为富文本；支持标题、列表、任务列表、表格、引用、代码块、脚注、删除线、行内代码、链接；可在托盘关闭并回退纯文本 |
| 代码高亮 | 代码块按语言高亮（Rust/Python/JS/TS/C/C++/JSON/Shell/SQL 等 syntect 内置语言），未知语言回退等宽文本；可在托盘关闭，关闭后不加载高亮数据（约省 8 MB） |
| 自定义动作 | 托盘「动作模板」子菜单：添加、编辑、删除按钮和提示词；保存后立即重建工具条，无需重启 |
| 模板文件 | 也可直接编辑 `%LOCALAPPDATA%\XuanCi\templates.json`，重启后生效 |
| LaTeX 公式（可选插件） | 默认不安装；从托盘安装官方 `xuanci-latex.exe` 后，`$...$` / `$$...$$` 转换为易读文本（分式、上下标、根号、希腊字母等）；未安装时显示源码 |
| 多服务商 | 托盘「服务商」子菜单：切换、添加、编辑、删除；每个服务商独立保存 Key |
| 自动识别 | 添加服务商时根据 Base URL 识别 DeepSeek/OpenAI/硅基流动/通义/智谱/Moonshot/Ollama/LM Studio，并预填名称与常用模型 |
| 模型列表 | 「获取模型列表…」调用 `GET {base_url}/models`，从返回结果中选择模型 |
| 单实例保护 | 重复启动会提示「已在运行」，不会争抢快捷键 |
| 开机自启 | 托盘菜单勾选，写入 `HKCU\...\Run`，使用 `--autostart` 静默启动 |

## 快捷键

| 功能 | 默认 |
|---|---|
| 翻译选中文本 | `Ctrl+Alt+D` |
| AI 解释选中文本 | `Ctrl+Alt+Q` |
| 调试：强制剪贴板回退 | `Ctrl+Alt+C`（仅 debug 构建） |

详细操作见 [使用说明.md](使用说明.md)。

已避开常见软件占用的组合：`Ctrl+Alt+Z`（QQ）、`Alt+A`（微信截图）、`Ctrl+Alt+A`（QQ 截图）、`Ctrl+Shift+A`（钉钉/飞书截图）。

## 构建与运行

前置：Rust（本机为 `x86_64-pc-windows-gnu`）+ MinGW gcc，无需 .NET。

```powershell
cd xuanci
cargo build --release
.\target\release\xuanci.exe
```

首次启动会要求输入当前服务商（默认 DeepSeek）的 API Key（`sk-...`），随后可在托盘的「服务商」子菜单里切换、添加其他服务商或修改配置。

调试构建会向 `%TEMP%\xuanci-debug.log` 写入不含正文的诊断信息（捕获耗时、字符数、错误原因）。

## 配置

数据目录：`%LOCALAPPDATA%\XuanCi\`

| 文件 | 内容 |
|---|---|
| `secrets.bin` | DPAPI 加密的「服务商 ID → API Key」映射；旧版单 Key 文件会自动迁移 |
| `settings.json` | 服务商列表、当前服务商、翻译目标语言、剪贴板回退开关、Markdown 渲染开关、LaTeX 插件开关（默认模型 `deepseek-flash`，官方另可选 `deepseek-v4-pro`） |
| `templates.json` | 提示词模板（见下） |

`settings.json` 中的服务商结构：

```json
{
  "providers": [
    {
      "id": "deepseek",
      "name": "DeepSeek",
      "base_url": "https://api.deepseek.com",
      "model": "deepseek-flash"
    }
  ],
  "active_provider": "deepseek"
}
```

顶层 `base_url` / `model` 是当前服务商的镜像，由程序自动维护。旧版只有 `base_url` / `model` 的配置会在启动时自动转成服务商列表。

Markdown 相关设置（均可在托盘菜单切换）：

```json
{
  "markdown_render": true,
  "code_highlight": true,
  "latex_plugin_enabled": true
}
```

- `markdown_render`：是否把 AI 回复渲染为富文本，默认 `true`；关闭后显示原始 Markdown。
- `code_highlight`：是否对代码块做语法高亮，默认 `true`；关闭后代码块仍以等宽纯文本显示，并避免加载 syntect（约 8 MB 私有内存）。
- `latex_plugin_enabled`：安装 LaTeX 插件后是否启用，默认 `true`；没有安装插件时无影响。

### 构建可选 LaTeX 插件

插件不在主程序的 Cargo workspace 中，默认不会被打包或安装：

```powershell
cargo build --release --manifest-path plugins/latex/Cargo.toml
```

生成 `plugins/latex/target/release/xuanci-latex.exe`（约 0.3 MB，纯文本转换，无第三方渲染库）。安装方式：托盘右键 →「LaTeX 公式」→「安装插件…」，选择这个 exe。

`templates.json` 示例：

```json
[
  {
    "id": "polish",
    "name": "润色",
    "system_prompt": "你是一名中文编辑……",
    "user_template": "请润色以下内容：\n{{text}}",
    "temperature": 0.4
  }
]
```

`{{text}}` 是必填占位符。无效模板会被跳过，坏 JSON 会回退到内置模板。

## 隐私与安全

- 仅 HTTPS（本地测试允许 `http://127.0.0.1`）
- 无遥测、无自动更新、空闲时不发起任何网络请求
- 日志不记录选中文本、提示词、模型回复、API Key
- 选中文本只存在于内存，不写历史
- 进程以普通权限运行，不自动提权
- Markdown 渲染不执行 HTML/JavaScript，不自动下载或读取任何图片
- 链接以蓝色下划线显示并附带 URL，当前版本不是可点击链接
- LaTeX 插件是独立进程，只有用户手动安装后才会运行；官方插件只读取请求中的公式文本并返回文本片段，不访问网络、不写文件

## 已知限制（v1）

- LaTeX 公式默认显示源码；安装可选插件后会转换为易读文本
- 公式为近似文本排版（分式用括号表示、上下标用 RichEdit 上下标），不追求与 LaTeX 完全一致的视觉效果
- Markdown 中的图片不显示，只保留 `[图片: alt]` 占位
- 任务列表的复选框是静态符号，不能点击切换
- 表格只支持 GFM 基本表格，复杂嵌套表格可能显示不理想
- 未实现安装包
- 未实现外部插件运行时（当前扩展方式为 JSON 模板）
- 不支持 OCR/截图取词、触控/触控笔划词
- 管理员权限程序、部分 Qt/Java/游戏窗口无法取词
- 未做代码签名，个别杀毒软件可能对全局钩子告警

## 许可证

- 开源使用：**GNU AGPL-3.0-only**，见 [LICENSE](LICENSE)。
- 商业授权：如果贵公司无法遵守 AGPL-3.0（例如需要闭源集成或运营网络服务但不公开修改），可以另行购买商业授权，见 [COMMERCIAL.md](COMMERCIAL.md)。

## 目录

```
crates/xuanci-core/   # 纯逻辑：模型、提示词、模板、DeepSeek SSE
crates/xuanci-win/    # Win32：UIA、剪贴板、钩子、热键、工具条、结果窗、DPAPI
crates/xuanci-app/    # 启动、消息循环、动作编排、存储
tools/mock_sse.py     # 本地 SSE mock，用于端到端测试
docs/plugin-spec.md   # 扩展机制与未来 WASM 插件规范
```
