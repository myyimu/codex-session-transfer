# Codex 会话迁移

一个本地优先的 Codex 桌面工具，用来在不同电脑之间导出、导入、同步和恢复 Codex 项目任务会话。

它会扫描当前电脑里的 Codex 本地任务历史，按项目和任务名展示，支持勾选后打成 ZIP 压缩包；在另一台电脑上打开同一个应用，即可预览压缩包，把选中的项目任务会话导入到当前 Codex 数据目录。所有处理都发生在本机，不会上传会话内容。

[English README](./README.en.md)

![导出任务总览](./docs/screenshots/readme/export-overview.png)

## About

Codex Session Transfer 适合这些场景：

- 换电脑时，把旧电脑上的 Codex 项目任务会话带到新电脑。
- 在多台电脑之间同步选中的 Codex 历史任务，而不是复制整个 `.codex` 目录。
- 备份重要任务会话，保留项目路径、任务标题、原始对话记录和浏览器会话配置。
- 本地已经存在会话文件，但 Codex 侧边栏看不到时，把本地历史重新恢复到 Codex 任务列表。

## 功能说明

- **导出任务**：扫描本机 Codex 任务历史，按任务勾选后导出为标准 ZIP。
- **按项目分组**：搜索框右侧提供项目下拉框，支持按 Codex 项目分组筛选任务。
- **任务搜索**：支持按任务名、项目路径和会话内容关键词搜索。
- **项目状态标记**：项目路径不存在或未在 Codex 侧栏显示时，会在列表和项目下拉框中标记，并放到靠后位置。
- **导入任务**：在新电脑预览 ZIP 内容后导入当前 Codex 数据目录。
- **重复任务处理**：默认跳过本机已存在的任务；也可以选择“从本地历史恢复”，让本机已有会话重新出现在 Codex 列表。
- **选择目标项目**：导入或恢复时可以选择放入哪个本机项目，也可以保留压缩包中记录的原项目路径。
- **路径适配**：旧电脑项目路径不存在时，可自动尝试匹配本机 `~/work`、`~/Projects`、`~/Documents` 下的同名项目。
- **运行中保护**：导入前会检测 Codex/ChatGPT 桌面端主进程。如果 Codex 正在运行，会提示先退出，避免 Codex 用内存状态覆盖恢复结果。
- **本地备份**：导入前会备份 Codex SQLite 数据库和全局状态文件，降低误操作风险。

## 页面教程

### 1. 导出任务

打开应用后默认进入“导出任务”。列表中会显示任务标题、更新时间、项目路径和对话轮数。勾选任务后，可以点击右上角或底部的“导出压缩包”。

![导出任务](./docs/screenshots/readme/export-overview.png)

### 2. 按项目筛选

点击搜索框右侧的项目下拉框，可以按项目筛选任务。已删除、缺失或未在 Codex 侧栏显示的项目会带有状态标记，方便判断是否需要迁移或恢复。

![项目筛选](./docs/screenshots/readme/project-dropdown.png)

### 3. 导入压缩包

切换到“导入任务”，拖入或选择由本应用导出的 `.zip` 压缩包。应用会先预览压缩包，不会立刻写入 Codex 数据。

![导入压缩包](./docs/screenshots/readme/import-empty.png)

### 4. 处理重复任务和目标项目

选择压缩包后，可以看到每个任务的导入状态：

- `可导入`：本机没有这个任务，会作为新任务导入。
- `已存在，将跳过`：本机已有同 ID 任务，默认不覆盖。
- `从本地历史恢复`：本机已有会话文件，但需要重新注册到 Codex 任务列表时使用。

你还可以在“导入到项目”中选择目标项目，让迁移来的任务出现在指定 Codex 项目分组下。

![导入设置](./docs/screenshots/readme/import-settings.png)

## 使用步骤

### 从旧电脑导出

1. 打开 `Codex 会话迁移`。
2. 进入“导出任务”。
3. 使用搜索框或项目下拉框找到要带走的任务。
4. 勾选任务，或点击“全选当前结果”。
5. 点击“导出压缩包”，保存生成的 `codex-tasks-YYYY-MM-DD.zip`。

### 在新电脑导入

1. 把 ZIP 压缩包复制到新电脑。
2. 完全退出 Codex/ChatGPT 桌面端，建议使用 `Cmd + Q`。
3. 打开 `Codex 会话迁移`，进入“导入任务”。
4. 选择 ZIP 压缩包并检查任务预览。
5. 按需要选择重复任务处理方式、目标项目和路径适配。
6. 点击“导入到 Codex”。
7. 重新打开 Codex/ChatGPT 桌面端，在侧边栏查看恢复后的项目和任务。

## 重复与恢复规则

导入默认不会覆盖已有任务。应用使用 Codex 任务 ID 判断重复：

- 本机不存在的任务会写入 Codex 会话目录。
- 本机已存在的任务默认跳过，不改原始会话文件。
- 选择“从本地历史恢复”时，应用会把本机已有会话重新注册到 Codex 任务列表，并取消归档/隐藏状态。
- 选择目标项目时，应用会把导入或恢复的任务绑定到该项目。
- 如果 Codex/ChatGPT 桌面端正在运行，应用会阻止导入，避免侧边栏状态被运行中的客户端覆盖。

## 下载

在 GitHub Releases 中下载最新版本：

**最新下载地址：** [github.com/myyimu/codex-session-transfer/releases/latest](https://github.com/myyimu/codex-session-transfer/releases/latest)

- macOS: `Codex-Session-Transfer-*-arm64.zip`
- Windows: `Codex-Session-Transfer-*-Windows-portable.zip`

macOS 解压后直接打开 `.app`。如果系统提示无法打开，可在 Finder 中右键应用并选择“打开”。

### macOS 授权打开

如果 macOS 提示“无法打开，因为无法验证开发者”，可以先尝试：

1. 在 Finder 中右键 `Codex 会话迁移.app`。
2. 选择“打开”。
3. 在系统确认弹窗中再次点击“打开”。

如果仍然无法打开，并且你确认应用来自本仓库 Releases，可使用命令移除隔离标记：

```bash
xattr -dr com.apple.quarantine "/Applications/Codex 会话迁移.app"
open "/Applications/Codex 会话迁移.app"
```

如果应用还在下载目录，把路径换成实际位置，例如：

```bash
xattr -dr com.apple.quarantine "$HOME/Downloads/Codex 会话迁移.app"
open "$HOME/Downloads/Codex 会话迁移.app"
```

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

## 开源协议

[MIT](./LICENSE)

## Star

如果这个小工具帮你把 Codex 会话顺利搬家了，可以顺手点一个 Star。它会乖乖继续变好，也会让作者开心很久。
