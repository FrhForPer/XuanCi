# XuanCi 划词助手

Windows 全局划词工具：在任意软件中选中文字，自动弹出小工具条，一键翻译或交给 AI；也支持全局快捷键。

- 原生 Rust + Win32，无 WebView / 无浏览器内核 / 无遥测
- 兼容任意 OpenAI 格式接口（DeepSeek、OpenAI、硅基流动、通义、智谱、Ollama、LM Studio 等），SSE 流式输出
- API Key 使用 Windows DPAPI 加密保存在本机
- 声明式提示词模板，改 JSON 即可扩展动作

## 已实现

| 能力 | 说明 |
|---|---|
| 划词自动弹条 | 鼠标拖选或双击选词后，在光标附近弹出「翻译 / AI 问答 / 自定义模板」 |
| 全局快捷键 | `Ctrl+Alt+D` 翻译，`Ctrl+Alt+Q` 让 AI 解释（v1 固定，占用时会提示） |
| 取词 | UI Automation 优先；UIA 失败时用 Ctrl+C 剪贴板回退并恢复原剪贴板（热键与自动弹条均可，可在托盘关闭） |
| 安全策略 | 密码框拒绝、管理员程序提示、终端禁用 Ctrl+C、文件/图片剪贴板回退保护 |
| 流式结果窗 | 可滚动、复制、停止、关闭 |
| 模板扩展 | `%LOCALAPPDATA%\XuanCi\templates.json`，重启后出现在工具条 |
| 设置 | 托盘菜单中设置 API Key、模型、Base URL |
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

首次启动会要求输入 DeepSeek API Key（`sk-...`），随后可用托盘菜单修改。

调试构建会向 `%TEMP%\xuanci-debug.log` 写入不含正文的诊断信息（捕获耗时、字符数、错误原因）。

## 配置

数据目录：`%LOCALAPPDATA%\XuanCi\`

| 文件 | 内容 |
|---|---|
| `secrets.bin` | DPAPI 加密的 API Key |
| `settings.json` | Base URL、模型、翻译目标语言、是否允许自动弹条使用剪贴板回退（默认模型 `deepseek-flash`，官方另可选 `deepseek-v4-pro`） |
| `templates.json` | 提示词模板（见下） |

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

## 已知限制（v1）

- 结果窗暂以纯文本显示 Markdown（`**粗体**` 等标记原样显示）
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
