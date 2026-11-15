# XuanCi LaTeX 公式插件（可选）

主程序**不包含**这个插件，也不会自动安装它。安装后，结果窗里的 `$...$` 和 `$$...$$` 会转换成易读文本；未安装时显示 LaTeX 源码。

转换是文本级的，不生成图片：分式写成 `(分子)/(分母)`，根号写成 `√(...)`，上下标使用 RichEdit 的上下标样式。支持常见的分数、根号、上下标、希腊字母、运算符、函数名、`\text` 等；不认识的命令会原样显示命令名，便于发现遗漏。

## 构建

```powershell
cargo build --release --manifest-path plugins/latex/Cargo.toml
```

产物：`plugins/latex/target/release/xuanci-latex.exe`（约 0.3 MB），仅依赖 `serde` / `serde_json`，不含浏览器、JavaScript 或网络访问。

## 安装

1. 托盘右键 →「LaTeX 公式」→「安装插件…」；
2. 选择 `xuanci-latex.exe`；
3. 插件会复制到 `%LOCALAPPDATA%\XuanCi\plugins\latex\`；
4. 「启用公式渲染」默认勾选。

卸载：同一菜单 →「卸载插件…」。

## 协议

插件通过 stdin/stdout 通信，不创建临时文件：

```text
xuanci-latex.exe --version
echo '<request-json>' | xuanci-latex.exe
```

请求：

```json
{
  "tex": "\\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}",
  "display": true
}
```

stdout 返回一行 JSON，`segments` 按顺序描述文本片段，`style` 可为 `normal`、`super`、`sub`：

```json
{"ok":true,"segments":[
  {"style":"normal","text":"("},
  {"style":"normal","text":"-b ± √(b"},
  {"style":"super","text":"2"},
  {"style":"normal","text":"-4ac))/("},
  {"style":"normal","text":"2a)"}
]}
```

失败时输出 `{"ok":false,"error":"..."}` 并以非零状态退出。宿主超时（15 秒）、输出非法或插件缺失时自动回退为 LaTeX 源码。单条公式上限 20,000 字符，片段合计上限 4,000 个。

## 安全

- 独立进程运行，只接收公式文本；
- 不读取选中文本、API Key 或设置；
- 不发起网络请求；
- 不写入任何文件。

插件是可选组件，请只安装你信任来源的二进制文件。
