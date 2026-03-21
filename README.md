# screen-capture

一个面向 Linux 的 Rust 截图工具 MVP workspace。当前阶段只实现：

- 命令行启动
- 全屏截图
- 区域截图
- 保存为 PNG
- 复制到系统剪贴板
- 明确错误处理
- 基础日志
- 可测试的核心抽象

当前可运行实现支持：

- **X11**：通过 X11 协议直接抓取全屏
- **Wayland**：通过 `xdg-desktop-portal` 请求截图
- **剪贴板**：通过 `wl-copy` 或 `xclip` 复制 PNG 数据

Wayland 路径依赖桌面环境提供可用的 portal screenshot 接口，通常会弹出系统级确认对话框。这个路径在不同桌面环境上的行为可能存在差异。

默认情况下，截图会优先保存到 `$HOME/Pictures`；如果该目录不存在，则回退到 `$HOME`，最后回退到当前工作目录。

## Workspace 结构

```text
screen-capture/
├─ Cargo.toml
├─ README.md
├─ crates/
│  ├─ capture-core/
│  ├─ capture-platform-linux/
│  ├─ capture-cli/
│  ├─ capture-editor/
│  ├─ capture-tray/
│  └─ capture-utils/
└─ .gitignore
```

### crate 职责

- `capture-core`
  - 核心模型
  - 截图抽象 trait
  - PNG 编码器
  - 输出目标抽象
  - 统一错误类型
- `capture-platform-linux`
  - Linux 平台后端选择
  - X11 全屏截图实现
  - Wayland / portal 截图实现
- `capture-utils`
  - 日志初始化
  - 默认输出文件名和路径生成
- `capture-cli`
  - 命令行参数解析
  - 依赖编排
  - 执行 capture -> encode -> write 流程
- `capture-editor`
  - 截图后编辑库
  - 当前支持矩形框、箭头和文本标注
  - 供 `capture-tray` 在截图后直接拉起编辑器
- `capture-tray`
  - Linux 系统托盘入口
  - 常驻后台运行
  - 当前提供 Wayland 交互选区入口、X11 热键和退出入口
  - 用户侧默认只需要这一个可执行文件

## 架构设计

核心抽象拆分为三层：

1. `ScreenCaptureBackend`
   - 负责“根据 `CaptureRequest` 从哪里拿到像素数据”
   - 平台差异集中在这里
2. `ImageEncoder`
   - 负责“如何把像素编码成目标格式”
   - 当前 MVP 只提供 PNG
3. `OutputTarget`
   - 负责“根据 `SaveOptions` 把编码结果写到哪里”
   - 当前提供文件系统输出

这样拆分有几个好处：

- 平台采集和文件输出解耦，后续接剪贴板、内存管道时不需要改采集逻辑
- 编码策略独立，后续支持 JPEG、WebP 或直接原始缓冲区都容易扩展
- CLI 只把命令行参数映射为 `CaptureRequest` 和 `SaveOptions`，核心流程通过统一 pipeline 执行

## Linux 现实限制

- 当前支持全屏截图、固定区域裁剪，以及 Wayland 下基于 portal 的交互选区
- Wayland 路径依赖 `xdg-desktop-portal` 和桌面环境后端，通常会出现用户确认 UI
- 当前 portal 实现假设返回的是本地可读的 `file://` URI；如果桌面环境返回其他 URI 形式，MVP 会明确报错
- X11 目前还没有最终版的半透明 overlay，但已经提供轻量矩形选区交互
- `Ctrl+Alt+A` 的全局热键和“按鼠标所在窗口截图”当前只在 X11 下实现
- Wayland 下这类能力通常需要 compositor 或桌面环境额外支持，当前不会假装可用
- 当前不包含最终版 overlay、标注、全局快捷键

## 标注原型状态

- `capture-editor` 当前是一个可运行原型，不是完整编辑器
- 当前支持：
  - 矩形框
  - 箭头
  - 文本标注
  - 保存为 PNG
  - 复制到剪贴板
  - 中文界面
- 项目已内置 [`assets/fonts/DroidSansFallbackFull.ttf`](/home/ryk/Documents/code/screen-capture/assets/fonts/DroidSansFallbackFull.ttf)
- 对应许可证文本见 [`assets/licenses/Apache-2.0.txt`](/home/ryk/Documents/code/screen-capture/assets/licenses/Apache-2.0.txt)
- 编辑器默认优先使用内置字体，因此即使目标机器缺少系统中文字体，也能正常显示和导出中文标注
- 当前还不支持：
  - 马赛克
  - 多级撤销栈
  - 字体族切换

## 构建

```bash
cargo build
```

## 运行

默认会在首选截图目录生成带时间戳的 PNG 文件：

```bash
cargo run -p capture-cli -- capture
```

只复制到剪贴板：

```bash
cargo run -p capture-cli -- capture --copy
```

只保存到文件：

```bash
cargo run -p capture-cli -- capture --save
```

同时保存并复制：

```bash
cargo run -p capture-cli -- capture --save --copy
```

显式指定全屏模式：

```bash
cargo run -p capture-cli -- capture --fullscreen
```

指定固定区域截图：

```bash
cargo run -p capture-cli -- capture --region 100,200,640,480
```

交互式选区截图：

```bash
cargo run -p capture-cli -- capture --select
```

指定输出路径：

```bash
cargo run -p capture-cli -- capture --output ./shot.png
```

处理同名文件：

```bash
cargo run -p capture-cli -- capture --on-conflict rename
cargo run -p capture-cli -- capture --on-conflict overwrite
cargo run -p capture-cli -- capture --on-conflict error
```

指定后端：

```bash
cargo run -p capture-cli -- capture --backend auto
cargo run -p capture-cli -- capture --backend x11
cargo run -p capture-cli -- capture --backend portal
```

延时截图：

```bash
cargo run -p capture-cli -- capture --delay 3
```

查看日志：

```bash
RUST_LOG=info cargo run -p capture-cli -- capture
```

环境诊断：

```bash
cargo run -p capture-cli -- doctor
```

系统托盘常驻：

```bash
cargo run -p capture-tray
```

用同一个可执行文件直接进入截图后编辑：

```bash
cargo run -p capture-tray -- --edit-region
```

编辑已有图片：

```bash
cargo run -p capture-tray -- --edit-input ./shot.png
```

Wayland 下如果要通过 GNOME 自定义快捷键触发区域截图，可以把同一个二进制绑定到：

```bash
/path/to/capture-tray --edit-region
```

## 测试

```bash
cargo test
```

## 输出策略

- 默认文件名形如 `screencap-YYYYMMDD-HHMMSS.png`
- 默认保存目录优先是 `$HOME/Pictures`
- 默认冲突策略是 `rename`，例如 `shot.png` 已存在时会写成 `shot-1.png`
- 可以显式选择 `overwrite` 或 `error`
- CLI 会把常见错误转换成更直接的提示，例如权限不足、会话不支持、backend 不可用、保存失败、图片数据为空

## 剪贴板说明

- Wayland 下优先使用 `wl-copy`
- X11 下使用 `xclip`
- 如果当前系统没有可用的剪贴板命令，`doctor` 会给出诊断结果，`--copy` 会返回明确错误

## 环境诊断

`doctor` 目前会输出：

- 当前会话类型
- `auto` 模式下检测到的 backend
- `xdg-desktop-portal` 是否可通过 session D-Bus 访问
- 默认输出目录是否可写
- 剪贴板支持状态
- 多显示器信息状态（目前仅占位）

## 托盘状态

- 当前已提供 Linux 系统托盘入口
- 托盘当前只保留右键菜单里的 `Quit`
- 不再提供独立 GUI 主窗口
- 左键点击托盘图标会触发一次交互式区域截图
- X11 下会在后台注册全局快捷键 `Ctrl+Alt+A`
- 触发后会按鼠标当前所在窗口的矩形截图，并保存到默认输出目录
- Wayland / GNOME 下建议把 `capture-tray --capture-region` 绑定到 GNOME 自定义快捷键

## GNOME Wayland 快捷键

如果你在 GNOME Wayland 下使用本项目，推荐把下面这个命令绑定到系统自定义快捷键：

```bash
/home/ryk/Documents/code/screen-capture/target/debug/capture-tray --capture-region
```

建议绑定到：

```text
Ctrl+Alt+A
```

GNOME 官方帮助文档说明，自定义快捷键可以直接绑定任意系统命令：
https://help.gnome.org/users/gnome-help/stable/keyboard-shortcuts-set.html

## 下一步建议

- 为 X11 增加真正的透明 overlay 选区层，而不是仅支持固定区域坐标
- 增加 `ClipboardTarget`
- 在 overlay 之上补区域调整、Esc 取消、Enter 确认等交互细节
- 为 X11 和 portal 路径补更多集成测试和格式兼容校验
