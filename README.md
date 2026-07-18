# Codex 会话迁移

一个本地运行的桌面工具，用于搜索、勾选、打包和恢复 Codex 任务会话。所有会话内容只在本机处理。

## 功能

- 按 Codex 中的任务名、工作目录和内容搜索历史任务
- 多选任务并导出为标准 ZIP 压缩包
- 在新电脑预览压缩包内容并导入当前 Codex
- 自动跳过相同任务 ID，避免覆盖已有会话
- 原项目路径不存在时尝试匹配本机常见工作目录
- 导入前备份 Codex 的 SQLite 数据库

## 本地开发

```bash
npm install
npm run dev
```

浏览器界面预览：

```bash
npm run dev:web
```

## 打包

macOS 免安装 ZIP：

```bash
npm run build:mac
```

Windows 免安装 EXE（需在 Windows 或相应 CI 环境执行）：

```bash
npm run build:win
```

构建产物位于 `release/`。

## 压缩包格式

压缩包根目录包含 `manifest.json`，每个任务位于 `tasks/<task-id>/`，保留原始 `session.jsonl` 与可选的 `browser.toml`。当前格式版本为 `codex-session-transfer/v1`。
