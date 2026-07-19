# Codex 会话迁移

一个轻量的本地桌面工具，用来在不同电脑之间迁移 Codex 任务会话历史。

它可以扫描当前电脑里的 Codex 任务，按任务名和项目筛选，勾选后导出为 ZIP 压缩包；在另一台电脑上打开同一个工具，即可预览压缩包并导入到当前 Codex 数据目录。所有数据只在本机处理，不会上传会话内容。

[English README](./README.en.md)

## 项目描述

Codex Session Transfer 是一个本地优先的 Codex 会话导入导出工具，适合更换电脑、备份重要任务、迁移项目上下文，或在多台机器之间同步选中的 Codex 历史任务。

建议 GitHub 仓库 About 描述：

```text
Local-first desktop utility for exporting and importing selected Codex task sessions across computers.
```

## 主要功能

- 扫描本机 Codex 任务历史，并显示与 Codex 相近的任务标题。
- 支持按任务名、路径、内容关键词搜索。
- 支持按项目分组筛选任务。
- 已删除或缺失的项目会标记为“项目已删除”，并排在项目下拉列表底部。
- 可多选任务并导出为标准 ZIP 压缩包。
- 可在新电脑预览压缩包内容并导入当前 Codex。
- 导入时默认按任务 ID 跳过重复任务，也可选择从本地历史恢复到 Codex 列表。
- 导入前自动备份 Codex SQLite 数据库。
- 导入时可尝试把旧电脑项目路径适配到本机常见目录。
- 导入或恢复时可选择目标项目，让任务出现在指定项目分组下。

## 系统页面使用教程

### 导出任务

1. 打开应用后停留在“导出任务”页面。
2. 点击右上角刷新按钮，重新扫描本机 Codex 任务。
3. 在搜索框输入任务名、项目路径或会话内容关键词。
4. 使用搜索框右侧的项目下拉框，按项目筛选任务。
5. 勾选需要迁移的任务，也可以点击“全选当前结果”。
6. 点击顶部或底部的“导出”按钮，选择保存位置。
7. 应用会生成 `codex-tasks-YYYY-MM-DD.zip` 压缩包。

### 导入任务

1. 切换到“导入任务”页面。
2. 点击选择 ZIP 压缩包。
3. 应用会预览压缩包中的任务，并标出本机已存在的重复任务。
4. 如果存在重复任务，可选择“跳过”或“从本地历史恢复”。恢复不会重新覆盖会话文件，会把本机已有历史重新写回 Codex 任务列表。
5. 在“导入到项目”里选择目标项目，或保持压缩包原本记录的项目路径。
6. 保持“自动适配项目路径”开启时，如果旧路径不存在，应用会尝试匹配本机 `~/work`、`~/Projects`、`~/Documents` 下的同名项目。
7. 点击“导入到 Codex”。
8. 导入完成后重启 Codex，即可看到新增或恢复的任务。

### 重复与覆盖规则

导入默认不会覆盖已有任务。应用使用 Codex 任务 ID 判断重复：

- 本机已存在的任务默认会跳过。
- 如果选择“从本地历史恢复”，应用会把本机已有会话重新注册到 Codex 任务列表，并取消归档隐藏状态。
- 新任务会写入 Codex 会话目录。
- 导入前会备份 Codex 数据库，降低误操作风险。

## 下载

在 GitHub Releases 中下载最新版本：

- macOS: `Codex-Session-Transfer-*-aarch64.zip`
- Windows: `Codex-Session-Transfer-*-x64-setup.exe`

macOS 解压后直接打开 `.app`。如果系统提示无法打开，可在 Finder 中右键应用并选择“打开”。

## 本地开发

```bash
npm install
npm run dev
```

只预览网页界面：

```bash
npm run dev:web
```

## 打包

macOS:

```bash
npm run build:mac
```

Windows:

```bash
npm run build:win
```

构建产物位于 `release/`。

## 压缩包格式

导出的 ZIP 根目录包含 `manifest.json`，每个任务位于 `tasks/<task-id>/`，保留原始 `session.jsonl` 与可选的 `browser.toml`。

当前格式版本为：

```text
codex-session-transfer/v1
```

## 隐私说明

应用只读取本机 Codex 数据目录，并在本机生成或导入 ZIP 文件。它不会上传会话内容，也不需要服务器。
