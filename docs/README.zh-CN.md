<p align="center">
  <img src="https://i.ibb.co/KxH6pYFd/logo.png" width="80" height="80" alt="LumeTrace 标志">
</p>

<h1 align="center">每个文件，都有自己的时间线。</h1>

<p align="center">LumeTrace 是一款 macOS 本地文件管理器，<br>以自动版本记录、文件时间线和可视化差异对比为核心。</p>

<p align="center">整理文件，继续使用你习惯的编辑器，随时追踪、对比和恢复历史版本。</p>

<p align="center">
  <a href="media/lumetrace-overview.gif">
    <img src="media/lumetrace-overview.gif" width="960" alt="LumeTrace 功能演示：拖入 Launch Plan.md，通过正文中的 onboarding 搜索文件，再让 Lumie 总结文件目标并展示引用来源。演示经过剪辑，时长 12 秒，等待时间已压缩。">
  </a>
</p>

<p align="center">
  <a href="../README.md">English</a> ·
  <a href="media/lumetrace-overview.gif">查看完整演示</a> ·
  <a href="DEVELOPMENT.md">开发文档</a>
</p>

## 看清每一次修改

自动记录每一次保存，直观对比每一处修改。需要回到之前时，随时恢复历史版本。

<p align="center">
  <a href="https://i.ibb.co/67XxPbbT/1.gif">
    <img src="https://i.ibb.co/67XxPbbT/1.gif" width="960" alt="LumeTrace 操作演示：将 Markdown 文件从 V1 修改为 V2，在时间线中查看两个版本，再通过行内与并排 Diff 对比修改。">
  </a>
</p>

## 如何使用

1. **照常编辑文件。** 在 LumeTrace 或你习惯的编辑器里修改并保存。
2. **时间线自动记录。** 每次修改成为同一个文件的新版本，不必另存一份。
3. **对比任意两个版本。** 在时间线中选择两个版本，通过 Diff 看清具体改了什么。

<p align="center">
  <img src="https://i.ibb.co/prMRVcND/download.png" width="960" alt="工作流程示意：同一个 Launch Plan.md 文件从 V1 逐步形成 V4，选择不相邻的 V1 和 V4 即可对比修改，不再需要 final_final_v2_really_final.md。">
</p>

一个文件，一段完整历史。不再需要 `final_final_v2_really_final.md`。

## 围绕文件展开

分开管理不同项目，找回之前的修改，找到需要的内容。需要一起分析时，再让 AI 帮忙。

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="https://i.ibb.co/p6b4MzW0/image.png" width="420" alt="文件空间示意：写作、研究与个人资料分别保存在独立空间，当前选中写作空间。">
      <h3>文件空间</h3>
      <p>从空文件夹开始，也可以接入已有文件夹。自由命名、切换不同空间，文件记录、历史版本、搜索索引、废纸篓和 AI 对话各自独立。</p>
      <p>实体文件始终保存在你选择的目录里。</p>
    </td>
    <td width="50%" valign="top">
      <img src="https://i.ibb.co/zVLBjFJb/image.png" width="420" alt="版本恢复示意：把 V2 设为当前版本，时间线仍保留 V1 和 V3。">
      <h3>找回旧版本</h3>
      <p>预览历史版本，再将它设为当前版本，其他历史记录依然保留。误删文件可以从废纸篓恢复，也可以导出和恢复 LumeTrace 备份。</p>
      <p>想回到之前，总有迹可循。</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="https://i.ibb.co/cKvqzfNY/image.png" width="420" alt="搜索示意：输入 launch，通过文件名、已索引的正文和标签找到相关文件。">
      <h3>不止搜索文件名</h3>
      <p>按下 <kbd>⌘ K</kbd>，搜索文件名、提取的正文和标签。即使忘了文件叫什么，也能通过里面写过的内容找到它。</p>
      <p>全文搜索无需配置 AI 服务。</p>
    </td>
    <td width="50%" valign="top">
      <img src="https://i.ibb.co/WWBdXy8t/AI.png" width="420" alt="AI 问答示意：Lumie 解释状态从 Draft 改为 Ready，并引用 Launch Plan.md 的 V1 至 V2 版本差异。">
      <h3>Lumie，你的 AI 文件助手</h3>
      <p>用自然语言提问、连续追问，或总结两个版本之间的修改。回答附带来源，可以打开文件核对。</p>
      <p>可接入 Ollama、LM Studio、兼容 OpenAI 的云端 API 或受支持的 Agent CLI。AI 不是必需项。</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="https://i.ibb.co/Dg1MXJf0/E5.png" width="420" alt="语义索引示意：Multilingual E5 Small 将提取的正文转换为向量，并在本地建立索引。">
      <h3>在本地按语义检索</h3>
      <p>从 Hugging Face 下载可选的 Multilingual E5 Small 模型，为搜索和 AI 检索增加语义相关性。建立索引时，正文与向量都留在你的 Mac 上。</p>
      <p>随时查看索引进度、暂停处理，或重试失败的任务。</p>
    </td>
    <td width="50%" valign="top">
      <img src="https://i.ibb.co/HfG8kKbH/image.png" width="420" alt="语言与外观示意：支持八种界面语言，以及浅色、深色和跟随系统的外观。">
      <h3>你的语言，你的外观</h3>
      <p>支持简体中文、繁体中文、英文、日文、韩文、德文、法文和西班牙文，共八种界面语言。</p>
      <p>选择浅色、深色或跟随系统外观，再搭配喜欢的强调色。</p>
    </td>
  </tr>
</table>

*功能插图使用示例内容绘制，并非应用截图。*

<details>
<summary>AI 服务兼容性</summary>

在偏好设置中选择服务。文件浏览、版本追踪、Diff、全文搜索和本地语义索引均不需要 AI 服务。

| 服务 | 连接方式 | 状态 |
| --- | --- | --- |
| Ollama | Ollama 原生 API | 已支持 |
| LM Studio | 兼容 OpenAI 的 API | 已支持 |
| 云端 API | 兼容 OpenAI 的 API，使用你自己的 Key | 已支持 |
| Hermes CLI | 本机已安装的 Agent CLI | 已支持 |
| Codex CLI | 本机已安装的 Agent CLI | 已支持 |
| Claude Code | 本机已安装的 Agent CLI | **实验性** |
| OpenCode | 本机已安装的 Agent CLI | **实验性** |

Claude Code 和 OpenCode 可供尝试，但尚未完成日常使用场景的充分验证。连接检测确认的是服务可用性，不代表对回答质量的保证。

</details>

## 开始使用

LumeTrace 是一款面向单用户的免费 macOS 应用。[v1.0.0 源码版本](https://github.com/gurudin/lumetrace/releases/tag/v1.0.0)已发布，该 Release 暂无安装包附件。目前可以[从源码启动](#开发启动)。

1. **创建文件空间。** 为它命名，并选择新的或已有的文件夹。使用已有文件夹时，先等待文件登记完成。
2. **保存一次修改。** 创建或打开 Markdown、TXT 等文本文件，在 LumeTrace 或常用编辑器中修改并保存。
3. **查看文件历史。** 打开时间线，选择两个版本查看 Diff。需要恢复时，将较早的版本设为当前版本。

导入后，正文提取和可选的语义索引会在后台继续。你可以在偏好设置中查看进度；查看文件时间线不需要等待这些任务完成。

## 文件始终属于你

- **文件与历史保存在本地。** 实体文件留在你选择的目录，文件空间信息、版本快照、搜索索引、废纸篓记录和 AI 对话也都保存在本地。
- **语义索引在本机建立。** 可选的 E5 模型从 Hugging Face 下载，建立索引不会上传文件空间内容。
- **主动提问，才使用 AI。** 问题、有限近期上下文、文件标识，以及相关正文片段或指定版本的 Diff，可能发送给你选择的模型、云端 API 或 Agent CLI。LumeTrace 不会在单次请求中发送整个文件空间。
- **了解数据如何保存。** 云端 API Key 在本地未加密保存，并从导出的备份中排除。备份文件本身也没有加密，请妥善保管。

## 常见问题

**可以继续使用原来的编辑器吗？**

可以。其他应用保存的文件修改会在后台被检测并记录到时间线。删除文件空间也不会悄悄删除对应的实体文件夹。

**哪些文件可以对比或搜索？**

Markdown、纯文本、源代码等受支持的文本文件可进行逐行和词级 Diff。全文索引还可从 PDF、DOCX、XLSX、PPTX 中提取可读正文。目前不提供扫描文档或图片的 OCR。

**时间线可以代替备份吗？**

不能。本地历史方便撤销修改，但不是独立于这台 Mac 的备份。建议导出备份，并另外保存一份。删除的内容最多在废纸篓保留 30 天，你也可以提前永久删除。

**支持同步或迁移其他应用的资料库吗？**

v1.0.0 暂不支持。LumeTrace 是本地、单用户的文件空间，不提供团队分享、NAS／云同步或跨设备冲突处理。你可以导入实体文件夹、恢复 LumeTrace 备份，但不支持 Eagle 等应用的专用资料库迁移。从文件夹初始化时，以 `.` 开头的隐藏目录会被排除。详见[导入与备份说明](MIGRATION.md)。

目前不提供 Windows 和 Linux 安装包。

## 反馈与社区

- [GitHub Issues](https://github.com/gurudin/lumetrace/issues) — 报告问题、提出改进建议，或跟进处理进展。
- [Discord](https://discord.gg/6pJVMTJ5UG) — 提问交流，分享你的使用方式。

反馈问题时，请提供应用版本、macOS 版本和复现步骤，必要时附上截图。分享日志或截图前，请移除私人文件内容和 API Key。

## 开发启动

需要 Node.js/npm、Rust，以及 [macOS 上的 Tauri 2 平台依赖](https://v2.tauri.app/start/prerequisites/)。在本地项目目录中运行：

```bash
npm install
npm run tauri:dev
```

本地开发请使用开发模式，不要将开发构建安装到 `/Applications`。架构、AI 接入细节和测试命令见[开发文档](DEVELOPMENT.md)。

## 许可证

LumeTrace 使用 [GNU Affero General Public License v3.0](../LICENSE)（`AGPL-3.0-only`）许可。

第三方组件仍遵循各自的许可证。
