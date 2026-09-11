export interface HelpArticle { id: string; title: string; summary: string; sections: { title: string; steps: string[] }[] }
export const zhHelpArticles: HelpArticle[] = [
  {
    "id": "start",
    "title": "从这里开始",
    "summary": "先管理文件，再按需开启搜索与 AI。帮助内容保存在应用内，离线也能阅读。",
    "sections": [
      {
        "title": "第一次使用",
        "steps": [
          "创建文件空间：为它命名，选择一个空文件夹，或导入已有文件夹。实体文件保存在你选择的位置。",
          "先导入一个 Markdown 或文本文件，打开预览，再保存一次修改。文件记录完成后即可使用，正文和向量索引会继续在后台处理。",
          "按 ⌘K 搜索文件名或正文；点击顶部 Lumie 图标可以配置并使用 AI。AI 服务不是文件管理的前提。"
        ]
      },
      {
        "title": "在哪里找到设置",
        "steps": [
          "左下角齿轮打开设置；偏好设置中有外观、语言、AI 服务和后台任务与状态。语义搜索、备份及文件空间管理也从齿轮菜单进入。",
          "左下角文件空间名称用于切换空间。右下角问号随时打开本手册；Esc 关闭帮助，重新打开会保留当前阅读位置。"
        ]
      },
      {
        "title": "这份手册的范围",
        "steps": [
          "本手册介绍社区版与专业版共有的本地文件功能。专业版外部空间的可用操作取决于当前连接与权限，请以界面实际启用的功能为准。"
        ]
      }
    ]
  },
  {
    "id": "workspaces",
    "title": "创建与切换文件空间",
    "summary": "把不同项目放在独立空间中，文件记录、历史和索引分别管理。",
    "sections": [
      {
        "title": "创建或导入",
        "steps": [
          "首次启动时选择新建空间或导入已有文件夹，并填写名称、选择目录。已有文件较多时，等待文件登记完成。",
          "已有空间可从左下角齿轮的“文件空间设置”管理；点击左下角空间名称切换。",
          "确认选择的是需要管理的文件夹。导入已有空间会使用原目录，不会把文件变成仅存在于应用内的副本。"
        ]
      },
      {
        "title": "目录不可用时",
        "steps": [
          "外置磁盘断开或目录移动后，先恢复磁盘连接或目录位置，再重试读取。不要为了恢复目录而反复建立空空间。",
          "移除空间与删除实体目录是不同操作；操作前阅读确认说明。历史记录也不等于独立备份。"
        ]
      }
    ]
  },
  {
    "id": "import",
    "title": "拖入与导入文件",
    "summary": "从 Finder 拖入文件或文件夹，也可以从文件区菜单选择导入。",
    "sections": [
      {
        "title": "拖入文件",
        "steps": [
          "先打开目标空间和文件夹，再把 Finder 中的文件拖到文件区。留意拖放提示，确认目标后松开。",
          "也可以在文件区空白处打开右键菜单，选择导入文件或文件夹。导入过程中观察进度，完成后正文索引继续在后台处理。",
          "把已有文件夹作为整个空间管理时，使用“导入已有文件夹”的空间流程。导入普通文件与导入整个空间用途不同。"
        ]
      },
      {
        "title": "遇到同名文件",
        "steps": [
          "根据提示选择“添加为最新版本”或保留为新名称。合并版本会更新目标文件，先核对路径和文件名。",
          "相同内容可能被跳过，可定位已有文件。不要把重复导入当作创建新版本的保证。",
          "递归初始化会跳过以点开头的隐藏目录；不能通过导入来迁移其他软件的专用数据库。"
        ]
      }
    ]
  },
  {
    "id": "organize",
    "title": "新建、整理与标签",
    "summary": "使用文件夹、标签和视图控制来整理实体文件。",
    "sections": [
      {
        "title": "新建与移动",
        "steps": [
          "使用侧栏“+”创建文件夹；文件区空白处右键可新建 Markdown 或文本文件。",
          "右键文件或文件夹可查看重命名、移动、删除等当前可用操作。移动前核对目标目录。",
          "可把文件拖到侧栏目标文件夹；拖出应用可交给 Finder 或其他支持接收文件的应用。拖放结果以目标提示和确认界面为准。"
        ]
      },
      {
        "title": "选择与视图",
        "steps": [
          "单击选择文件，⌘ 单击选择多个文件，Shift 可用于范围选择。先确认选中的项目再执行批量操作。",
          "顶部视图控制可调整文件展示密度；使用排序和筛选缩小列表。手动排序只在允许手动排序的视图条件下可用。",
          "文件右键菜单中的标签操作可添加或调整标签，之后可以用标签搜索或筛选。"
        ]
      }
    ]
  },
  {
    "id": "preview",
    "title": "预览、编辑与外部应用",
    "summary": "选中文件后查看内容，支持的文本可以在应用内编辑。",
    "sections": [
      {
        "title": "打开与预览",
        "steps": [
          "选中文件后按空格预览；也可使用右键菜单的预览或系统应用打开操作。不同格式会使用对应的预览方式。",
          "图片支持缩放查看；PDF 可以翻页。Markdown 和支持的文本格式可查看正文或进入编辑。",
          "内置预览无法读取的格式，可交给系统应用打开。扫描 PDF 和图片不提供 OCR 正文识别。"
        ]
      },
      {
        "title": "保存修改",
        "steps": [
          "编辑文本后使用编辑器的保存操作或 ⌘S。关闭前处理未保存提示，不要把预览历史版本误认为当前文件。",
          "可以继续使用自己熟悉的外部编辑器。保存到同一实体文件后，LumeTrace 在后台检测修改并记录版本。",
          "若看不到新版本，先检查文件路径、磁盘连接及后台文件监听状态。"
        ]
      }
    ]
  },
  {
    "id": "search",
    "title": "搜索文件名、正文与标签",
    "summary": "全文搜索不需要 API Key，也不需要先配置 AI。",
    "sections": [
      {
        "title": "开始搜索",
        "steps": [
          "点击顶部搜索框或按 ⌘K，输入文件名、正文中的词或标签。非 macOS 键盘对应 Alt+K。",
          "查看搜索结果与匹配信息，选择结果定位或打开文件。必要时调整范围、标签或文件类型筛选。",
          "新导入文件可先按文件名找到；正文匹配需要等待文本提取与全文索引完成。"
        ]
      },
      {
        "title": "搜索不到时",
        "steps": [
          "到“偏好设置 → 后台任务与状态”检查是否暂停、有待处理任务或失败项，继续处理或重试。",
          "PDF、DOCX、XLSX、PPTX 和支持的文本格式可以提取可读正文；图片、扫描件或超出提取限制的文件不一定有正文索引。",
          "想增加内容含义的相关性，可安装本地语义模型。语义相关性不等于所有文件都能被完整理解或找到。"
        ]
      }
    ]
  },
  {
    "id": "cloud-ai",
    "title": "配置云端 AI",
    "summary": "使用你自己的 OpenAI 兼容 API 服务、密钥与模型。",
    "sections": [
      {
        "title": "连接并保存",
        "steps": [
          "打开左下角齿轮 → 偏好设置 → AI 服务，选择“云端 API”。",
          "填写供应商给出的 OpenAI 兼容 Base URL 和 API Key。通常地址包含 /v1；不要粘贴聊天网页地址或完整 /chat/completions 路径。",
          "点击“测试连接”，连接成功后选择模型，再点击“保存”。更换地址、密钥或服务时重新测试。"
        ]
      },
      {
        "title": "常见问题与数据范围",
        "steps": [
          "认证失败先检查密钥、账户权限和额度；连接失败检查地址、网络和供应商的兼容接口。模型不可用时检查该账户是否有调用权限。",
          "连接测试不发送工作空间文件。主动提问时，问题、有限上下文、文件信息及相关片段或版本 Diff 可发送给所选服务。",
          "API Key 存在本机配置中，当前未加密，且不包含在导出备份内。不要把密钥放进截图或问题反馈。调用费用由你的服务商收取。"
        ]
      }
    ]
  },
  {
    "id": "local-ai",
    "title": "配置 Ollama / LM Studio",
    "summary": "本地聊天模型需要先由对应服务运行，和 E5 向量模型分开配置。",
    "sections": [
      {
        "title": "Ollama",
        "steps": [
          "先安装并运行 Ollama，准备一个可用的聊天模型。",
          "打开“偏好设置 → AI 服务 → 本地模型”，选择 Ollama，输入服务地址，例如 http://127.0.0.1:11434。",
          "测试连接，选择返回的模型，然后保存。若没有模型，先在 Ollama 中下载模型，再重新测试。"
        ]
      },
      {
        "title": "LM Studio",
        "steps": [
          "在 LM Studio 中下载并加载聊天模型，启动其本地 OpenAI 兼容服务器。",
          "在 LumeTrace 的本地模型服务类型中选择 LM Studio，填写例如 http://127.0.0.1:1234/v1，测试连接后选模型并保存。",
          "端口以你本机实际设置为准。连接不上时检查服务是否启动、模型是否加载、地址是否正确。"
        ]
      },
      {
        "title": "本地与远程的区别",
        "steps": [
          "127.0.0.1 指当前电脑。填写其他机器的地址时，请求会发往那台机器；“本地模型”这一选项本身不保证数据不离开本机。",
          "E5 只负责本地语义索引，不能代替聊天模型，也不会自动开启 Lumie 对话。"
        ]
      }
    ]
  },
  {
    "id": "agent-cli",
    "title": "使用 Agent CLI",
    "summary": "连接本机已安装并完成登录的命令行 AI 工具。",
    "sections": [
      {
        "title": "配置流程",
        "steps": [
          "先在终端完成所选 CLI 的安装与登录，并确认它能正常运行。",
          "进入“偏好设置 → AI 服务 → Agent CLI”，选择工具，检测可用状态，选择文件权限并保存。",
          "Hermes CLI 和 Codex CLI 已接入；Claude Code 与 OpenCode 标记为实验性，日常可用性尚未充分验证。"
        ]
      },
      {
        "title": "权限与故障",
        "steps": [
          "优先使用只读权限。允许读写会赋予工具修改实体文件的能力，实际写入还需明确确认；执行前核对修改目标。",
          "LumeTrace 使用 CLI 自身的登录与模型设置，不读取其登录凭证。检测失败时检查 CLI 安装、登录和可执行命令是否可被应用找到。"
        ]
      }
    ]
  },
  {
    "id": "lumie",
    "title": "用 Lumie 提问与查看来源",
    "summary": "围绕文件内容提问、总结，或比较版本变化。",
    "sections": [
      {
        "title": "提问步骤",
        "steps": [
          "先配置并保存一个可用 AI 服务，再点击顶部 Lumie 图标打开对话。",
          "输入具体问题，例如“帮我找到提到 onboarding 的文件”“总结发布方案的目标”或“这个文件 V1 到 V2 改了什么”。",
          "等待检索和生成，必要时停止请求。回答后可继续追问；点击来源引用查看对应文件或版本。"
        ]
      },
      {
        "title": "让回答更可靠",
        "steps": [
          "尽量给出文件名、关键词、版本号和任务目标，减少同名文件或模糊描述造成的歧义。",
          "把引用与原文核对后再采用结论。AI 可能遗漏内容或给出错误解释，不能代替文件本身。",
          "找不到内容时先检查正文索引、搜索结果、当前空间和服务配置。对话不会一次性发送整个文件空间。"
        ]
      }
    ]
  },
  {
    "id": "vectors",
    "title": "配置向量与语义搜索",
    "summary": "下载本地 E5 Embedding 模型，给搜索和 AI 检索增加内容含义相关性。",
    "sections": [
      {
        "title": "安装与建立索引",
        "steps": [
          "点击左下角齿轮 → 语义搜索，查看本地 Embedding 模型状态。当前使用 Multilingual E5 Small。",
          "点击“下载模型”。模型从 Hugging Face 下载，随后验证文件并为可读取的正文建立语义索引。",
          "等待状态变为可用。新增和修改文件会在后台继续处理，可到“后台任务与状态”查看进度。无需填写云端 Embedding API Key。"
        ]
      },
      {
        "title": "使用与管理",
        "steps": [
          "安装后继续使用普通搜索和 Lumie 检索，不需要手动把文件转换成向量。向量由可提取的正文生成，图片和扫描件不因此获得 OCR。",
          "下载失败检查网络后重试；索引失败查看失败项再重试。下载和索引期间仍可管理文件。",
          "移除模型会关闭语义搜索并删除本地向量，文件名与全文搜索仍可用。模型、文本、分块和向量在索引过程中保留在本机。"
        ]
      },
      {
        "title": "与聊天模型的区别",
        "steps": [
          "Embedding 用于计算内容相关性；聊天模型用于生成回答。两者分别配置，安装 E5 不会自动启用 AI 服务。"
        ]
      }
    ]
  },
  {
    "id": "history",
    "title": "时间线、Diff 与恢复版本",
    "summary": "保留每次已记录的变化，比较内容，并在需要时回到旧版本。",
    "sections": [
      {
        "title": "查看与比较",
        "steps": [
          "选中文件，使用版本记录入口打开时间线。初次登记是一个版本，后续保存或明确合并同名文件可形成新版本。",
          "选择历史版本查看当时内容；选择两个版本查看 Diff。支持的文本可做行级、词级比较，并切换并排或行内视图。",
          "可为版本添加备注或标记里程碑，说明修改原因。备注和标记不修改文件正文。"
        ]
      },
      {
        "title": "恢复旧内容",
        "steps": [
          "先预览目标历史版本，确认文件与内容，再使用设为当前版本的操作。它会影响当前实体文件，执行前处理外部编辑器里未保存的修改。",
          "已有历史仍保留；恢复不是删除后续全部历史。图片和二进制格式不提供文本 Diff。",
          "时间线在本地保存，不能替代独立备份，也不能保证应用管理前的历史已经存在。"
        ]
      }
    ]
  },
  {
    "id": "trash-backup",
    "title": "废纸篓与备份",
    "summary": "误删后恢复文件；定期导出独立备份。",
    "sections": [
      {
        "title": "废纸篓",
        "steps": [
          "删除文件后到侧栏“废纸篓”查看，选择需要的项目并使用恢复操作。路径冲突时阅读提示后处理。",
          "项目最多保留 30 天，也可能被你提前永久删除。清空或永久删除前核对选择，不能承诺再次恢复。"
        ]
      },
      {
        "title": "导出和恢复备份",
        "steps": [
          "点击齿轮 → 导出备份，选择保存位置并等待成功。备份包括当前文件、废纸篓、版本记录和本地索引。",
          "齿轮 → 从备份恢复，选择 LumeTrace 备份文件，查看检查结果，再选恢复目录并确认。恢复会切换空间并替换相应的受管理记录，务必阅读确认说明。",
          "备份不加密，不包含云端 API Key。把备份另存到可靠位置；恢复后按需重新配置 AI 凭证。"
        ]
      }
    ]
  },
  {
    "id": "background",
    "title": "后台任务与故障排查",
    "summary": "区分文件登记、正文提取、向量索引与文件监听。",
    "sections": [
      {
        "title": "查看处理状态",
        "steps": [
          "打开“偏好设置 → 后台任务与状态”，或点击顶部后台状态入口，查看各类任务的进度、待处理数量与失败项。",
          "正文提取与索引未完成时，文件仍可浏览和管理；全文搜索、语义检索可能暂时缺少结果。",
          "暂停只影响正文和索引处理，文件监听仍继续运行。点击继续恢复队列，失败项可单独重试。"
        ]
      },
      {
        "title": "按现象排查",
        "steps": [
          "文件不见了：确认当前空间、文件夹、筛选条件和废纸篓；检查磁盘是否在线。",
          "文件有但搜不到正文：确认格式能提取文字，检查后台队列。扫描件不支持 OCR。",
          "AI 无法回答：检查 AI 服务测试结果、模型权限、额度和当前网络；索引成功不代表聊天服务已配置。",
          "从齿轮 → 功能反馈进入 GitHub Issues 或 Discord，提供版本、复现步骤和脱敏截图，不要附 API Key 或私人文件正文。"
        ]
      }
    ]
  },
  {
    "id": "appearance",
    "title": "外观、语言与快捷键",
    "summary": "让阅读和操作适合你的习惯。",
    "sections": [
      {
        "title": "外观与语言",
        "steps": [
          "打开齿轮 → 偏好设置 → 外观，选择浅色、深色或跟随系统，并调整可用的强调色与字体选项。",
          "在偏好设置的语言页面选择界面语言。应用支持八种语言；本手册完整内容目前提供中文与英文，其他界面语言显示英文手册。"
        ]
      },
      {
        "title": "常用快捷键",
        "steps": [
          "⌘K：打开文件搜索。空格：预览选中的文件。⌘S：保存正在编辑的文本。",
          "⌘ 单击：多选。Shift：用于范围选择。Esc：关闭当前弹层或预览；在帮助中心内按 Esc 关闭帮助。",
          "使用 Tab 在按钮和输入框间移动，Enter 或空格激活按钮。输入正文时，按键以当前编辑器的行为为准。"
        ]
      }
    ]
  }
];
export const enHelpArticles: HelpArticle[] = [
  {
    "id": "start",
    "title": "Start here",
    "summary": "Organize your files first, then enable search and AI as needed. This guide is bundled with the app and works offline.",
    "sections": [
      {
        "title": "Your first workspace",
        "steps": [
          "Create a workspace, give it a name, and choose an empty folder or import an existing folder. Physical files remain in the location you choose.",
          "Import a Markdown or text file, preview it, and save an edit. Files become available before background text extraction and semantic indexing finish.",
          "Press ⌘K to search names or content. Open Lumie from the top toolbar to configure and use AI. File management does not require AI."
        ]
      },
      {
        "title": "Find the controls",
        "steps": [
          "The bottom-left gear opens settings. Preferences contains Appearance, Language, AI service, and Background tasks and status. Semantic search, backups, and workspace management are also in the gear menu.",
          "Use the workspace name at bottom left to switch workspaces. The question mark opens this guide. Escape closes it; reopening keeps your place."
        ]
      },
      {
        "title": "Scope of this guide",
        "steps": [
          "This guide covers local-file features shared by Community and Pro. Operations in Pro external spaces depend on the connection and permissions; follow the controls actually available in that space."
        ]
      }
    ]
  },
  {
    "id": "workspaces",
    "title": "Create and switch workspaces",
    "summary": "Keep projects in separate spaces with separate records, history, and indexes.",
    "sections": [
      {
        "title": "Create or import",
        "steps": [
          "At first launch, create a workspace or import an existing folder, enter a name, and select its directory. Wait for file registration when importing many files.",
          "Manage spaces through Workspace settings in the bottom-left gear menu. Click the workspace name to switch spaces.",
          "Choose the folder you intend to manage. Importing an existing workspace uses its original directory rather than turning files into app-only copies."
        ]
      },
      {
        "title": "When a directory is unavailable",
        "steps": [
          "Reconnect an external disk or restore the directory location, then retry. Repeatedly creating empty spaces will not recover the missing directory.",
          "Removing a workspace and deleting its physical folder are different operations. Read the confirmation before proceeding. Local history is not an independent backup."
        ]
      }
    ]
  },
  {
    "id": "import",
    "title": "Drag in and import files",
    "summary": "Bring files or folders from Finder, or use the file-area import menu.",
    "sections": [
      {
        "title": "Import into a folder",
        "steps": [
          "Open the destination workspace and folder, then drag files from Finder into the file area. Check the drop feedback before releasing.",
          "Alternatively, right-click empty space in the file area and choose the file or folder import action. Watch progress; text indexing continues after registration.",
          "To manage an existing folder as an entire workspace, use the workspace import flow. Importing individual files and importing a workspace serve different purposes."
        ]
      },
      {
        "title": "Handle duplicate names",
        "steps": [
          "Choose to add the incoming file as the latest version or keep it under a new name. Merging updates the destination file; verify its name and path first.",
          "Identical content may be skipped, with an option to locate the existing file. Reimporting does not always create a new version.",
          "Recursive initialization skips dot-prefixed hidden directories. File import does not migrate another app’s internal database."
        ]
      }
    ]
  },
  {
    "id": "organize",
    "title": "Create, organize, and tag",
    "summary": "Use folders, tags, and view controls to organize physical files.",
    "sections": [
      {
        "title": "Create and move",
        "steps": [
          "Use the sidebar + to create a folder. Right-click empty space in the file area to create a Markdown or plain-text file.",
          "Right-click a file or folder for available rename, move, and delete actions. Check the destination before moving.",
          "Drag files to a sidebar folder, or drag out to Finder or another app that accepts files. Follow the destination feedback and any confirmation."
        ]
      },
      {
        "title": "Selection and views",
        "steps": [
          "Click to select; ⌘-click selects multiple files and Shift supports range selection. Check the selected items before a bulk action.",
          "Use the toolbar view controls to adjust file density, and sorting and filters to narrow the list. Manual ordering is available only under compatible view conditions.",
          "Use the file context menu to add or edit tags, then search or filter by those tags."
        ]
      }
    ]
  },
  {
    "id": "preview",
    "title": "Preview, edit, and open externally",
    "summary": "Inspect selected files and edit supported text inside the app.",
    "sections": [
      {
        "title": "Open a preview",
        "steps": [
          "Select a file and press Space to preview it, or use the context menu to preview or open it in the system app. The viewer depends on the format.",
          "Zoom images or browse PDF pages. Markdown and supported text formats can be read or edited in their respective views.",
          "Use the system app for formats without a readable built-in preview. Scanned PDFs and images do not have OCR text extraction."
        ]
      },
      {
        "title": "Save changes",
        "steps": [
          "Use the editor’s Save action or ⌘S. Handle any unsaved-change prompt before closing. A historical preview is not the current file.",
          "Keep using an external editor if you prefer. Saving to the same physical file lets LumeTrace detect the change and record a version in the background.",
          "If a version is missing, check the file path, disk connection, and background file-watcher status."
        ]
      }
    ]
  },
  {
    "id": "search",
    "title": "Search names, content, and tags",
    "summary": "Full-text search requires neither an API key nor an AI service.",
    "sections": [
      {
        "title": "Search your workspace",
        "steps": [
          "Click Search in the toolbar or press ⌘K. Enter a file name, words from its content, or a tag. The non-macOS shortcut is Alt+K.",
          "Inspect the results and matches, then select a result to locate or open it. Adjust scope, tags, or file-type filters when needed.",
          "Newly registered files can be found by name first. Content matches become available after text extraction and full-text indexing."
        ]
      },
      {
        "title": "When a file is missing",
        "steps": [
          "Open Preferences → Background tasks and status. Resume paused work or retry failed items, and allow pending extraction to finish.",
          "Readable text can be extracted from PDF, DOCX, XLSX, PPTX, and supported text formats. Images, scans, or files beyond extraction limits may have no indexed body.",
          "Install the local semantic model to add meaning-based relevance. Semantic relevance does not guarantee that every file can be understood or retrieved."
        ]
      }
    ]
  },
  {
    "id": "cloud-ai",
    "title": "Configure cloud AI",
    "summary": "Connect your own OpenAI-compatible API, key, and model.",
    "sections": [
      {
        "title": "Connect and save",
        "steps": [
          "Open the bottom-left gear → Preferences → AI service and select Cloud API.",
          "Enter the provider’s OpenAI-compatible Base URL and your API Key. The base often includes /v1; do not use a chat website URL or a complete /chat/completions path.",
          "Click Test connection, choose an available model, and Save. Test again after changing the address, key, or service."
        ]
      },
      {
        "title": "Troubleshooting and data",
        "steps": [
          "For authentication failures, check the key, account permissions, and quota. For connection failures, check the address, network, and API compatibility. Verify model access for your account.",
          "Connection tests do not send workspace files. When you ask a question, the chosen service may receive the question, limited context, file identities, relevant excerpts, or a requested version Diff.",
          "API keys are stored locally without encryption and excluded from exported backups. Keep keys out of screenshots and feedback. Your provider controls usage charges."
        ]
      }
    ]
  },
  {
    "id": "local-ai",
    "title": "Configure Ollama / LM Studio",
    "summary": "Run a chat model in its own service. Configure it separately from the E5 embedding model.",
    "sections": [
      {
        "title": "Ollama",
        "steps": [
          "Install and start Ollama, then prepare an available chat model.",
          "Open Preferences → AI service → Local model, choose Ollama, and enter its address, such as http://127.0.0.1:11434.",
          "Test the connection, select a returned model, and Save. If no models appear, download one in Ollama and test again."
        ]
      },
      {
        "title": "LM Studio",
        "steps": [
          "Download and load a chat model in LM Studio, then start its local OpenAI-compatible server.",
          "Choose LM Studio in LumeTrace’s Local model settings. Enter an address such as http://127.0.0.1:1234/v1, test, choose a model, and Save.",
          "Use the port configured on your machine. Check that the server is running and the model is loaded if the connection fails."
        ]
      },
      {
        "title": "Local versus remote",
        "steps": [
          "127.0.0.1 means this computer. An address on another machine sends requests there; choosing Local model alone does not guarantee that data stays on your Mac.",
          "E5 provides semantic indexing, not chat generation. Installing it does not configure Lumie."
        ]
      }
    ]
  },
  {
    "id": "agent-cli",
    "title": "Use an Agent CLI",
    "summary": "Connect a command-line AI tool installed and signed in on your Mac.",
    "sections": [
      {
        "title": "Configure the tool",
        "steps": [
          "Install and sign in to the chosen CLI in Terminal first, and confirm that it runs normally.",
          "Open Preferences → AI service → Agent CLI, select the tool, check availability, choose file permissions, and Save.",
          "Hermes CLI and Codex CLI are integrated. Claude Code and OpenCode are experimental and are not fully validated for everyday use."
        ]
      },
      {
        "title": "Permissions and failures",
        "steps": [
          "Prefer read-only access. Read/write permission enables physical-file changes, which still require explicit confirmation. Check the target before approving a change.",
          "The CLI uses its own login and model configuration; LumeTrace does not read its login credentials. If detection fails, check installation, login, and whether the app can find the executable."
        ]
      }
    ]
  },
  {
    "id": "lumie",
    "title": "Ask Lumie and inspect sources",
    "summary": "Ask about file contents, summarize documents, or compare versions.",
    "sections": [
      {
        "title": "Ask a question",
        "steps": [
          "Configure and save an available AI service, then open Lumie from the top toolbar.",
          "Ask a specific question, such as “Find files mentioning onboarding”, “Summarize the launch plan’s goals”, or “What changed between V1 and V2 of this file?”",
          "Wait for retrieval and generation, or stop the request if needed. Follow up in the conversation and open source references to inspect files or versions."
        ]
      },
      {
        "title": "Check the answer",
        "steps": [
          "Include a file name, keywords, version numbers, and your goal to reduce ambiguity.",
          "Verify references against the source before relying on a conclusion. AI can omit information or explain it incorrectly.",
          "If content is missing, check text indexing, search results, the current workspace, and service configuration. A conversation does not send the entire workspace in one request."
        ]
      }
    ]
  },
  {
    "id": "vectors",
    "title": "Set up vectors and semantic search",
    "summary": "Download the local E5 embedding model to add meaning-based relevance to search and AI retrieval.",
    "sections": [
      {
        "title": "Install and index",
        "steps": [
          "Open the bottom-left gear → Semantic search and check the local embedding model status. The current model is Multilingual E5 Small.",
          "Choose Download model. The app downloads from Hugging Face, validates the files, and indexes readable extracted text.",
          "Wait until the model is ready. New and changed files continue processing in the background; check Background tasks and status for progress. No cloud embedding API key is needed."
        ]
      },
      {
        "title": "Use and manage",
        "steps": [
          "Continue using normal search and Lumie retrieval; you do not manually convert files to vectors. Embeddings use extracted text and do not add OCR to images or scans.",
          "Retry failed downloads after checking the network. Inspect failed indexing tasks before retrying. File management remains available during downloads and indexing.",
          "Removing the model disables semantic search and deletes local vectors. Name and full-text search remain available. The model, extracted text, chunks, and vectors stay local during indexing."
        ]
      },
      {
        "title": "Embedding is not chat",
        "steps": [
          "Embeddings measure content relevance; chat models generate answers. Configure them separately. Installing E5 does not enable an AI service."
        ]
      }
    ]
  },
  {
    "id": "history",
    "title": "Timeline, Diff, and restore",
    "summary": "Inspect recorded changes, compare content, and return to an earlier version.",
    "sections": [
      {
        "title": "Inspect and compare",
        "steps": [
          "Select a file and open its version-history entry. Initial registration creates a version; later saves or an explicit same-name merge can create subsequent versions.",
          "Preview a historical version or select two versions for Diff. Supported text has line- and word-level comparisons with inline and side-by-side views.",
          "Add a version note or mark a milestone to explain a change. Notes and milestones do not modify the file’s content."
        ]
      },
      {
        "title": "Restore content",
        "steps": [
          "Preview the target version, verify the file and contents, then use the action to make it current. This affects the physical file; handle unsaved external-editor changes first.",
          "Other recorded history is retained. Restoring does not erase every later version. Images and binary formats do not have text Diff.",
          "Local history is not an independent backup and does not contain changes from before the file was managed."
        ]
      }
    ]
  },
  {
    "id": "trash-backup",
    "title": "Trash and backups",
    "summary": "Recover deleted files and keep independent backups.",
    "sections": [
      {
        "title": "Recover from Trash",
        "steps": [
          "After deleting a file, open Trash in the sidebar, select the item, and use Restore. Read the prompt if the destination conflicts.",
          "Items remain for up to 30 days unless permanently deleted sooner. Check your selection before emptying Trash; recovery after permanent deletion is not guaranteed."
        ]
      },
      {
        "title": "Export and restore a backup",
        "steps": [
          "Choose gear → Export backup, select a destination, and wait for success. The backup includes current files, Trash, version records, and local indexes.",
          "Choose gear → Restore from backup, select a LumeTrace archive, inspect it, then select a destination and confirm. Restoration switches the workspace and replaces the relevant managed records; read the confirmation.",
          "Backups are not encrypted and exclude cloud API keys. Store a separate copy somewhere reliable and reconfigure AI credentials after restoration as needed."
        ]
      }
    ]
  },
  {
    "id": "background",
    "title": "Background tasks and troubleshooting",
    "summary": "Distinguish registration, text extraction, semantic indexing, and file watching.",
    "sections": [
      {
        "title": "Inspect progress",
        "steps": [
          "Open Preferences → Background tasks and status, or the toolbar background-status control, to inspect progress, pending work, and failures.",
          "Files remain usable before extraction and indexing finish, but full-text and semantic results can be incomplete.",
          "Pausing affects text and index processing; file watching continues. Resume the queue or retry failed tasks."
        ]
      },
      {
        "title": "Diagnose the symptom",
        "steps": [
          "Missing file: check the active workspace, folder, filters, Trash, and disk connection.",
          "File exists but its text is missing from search: check extraction support and the background queue. Scans do not have OCR.",
          "AI cannot answer: check the service connection, model access, quota, and network. A ready index does not mean chat is configured.",
          "Open gear → Feedback for GitHub Issues or Discord. Include the version, reproduction steps, and sanitized screenshots, but no API keys or private file contents."
        ]
      }
    ]
  },
  {
    "id": "appearance",
    "title": "Appearance, language, and shortcuts",
    "summary": "Adjust reading and interaction to suit your workflow.",
    "sections": [
      {
        "title": "Appearance and language",
        "steps": [
          "Open gear → Preferences → Appearance for light, dark, or system appearance, plus available accent and font settings.",
          "Choose the interface language in Preferences → Language. The app supports eight languages. The full guide currently has Chinese and English editions; other interface languages use the English guide."
        ]
      },
      {
        "title": "Common shortcuts",
        "steps": [
          "⌘K opens file search. Space previews the selected file. ⌘S saves text being edited.",
          "⌘-click selects multiple items. Shift supports range selection. Escape closes the current overlay or preview; inside Help it closes this guide.",
          "Use Tab between controls and Enter or Space to activate buttons. While editing text, keys follow the active editor’s behavior."
        ]
      }
    ]
  }
];

export function helpArticles(language: string): HelpArticle[] {
  return language === "zh-CN" ? zhHelpArticles : enHelpArticles;
}
export function searchHelp(articles: HelpArticle[], query: string): HelpArticle[] {
  const words = query.trim().toLocaleLowerCase().split(/\s+/u).filter(Boolean);
  return articles.filter(article => {
    const text = [article.title, article.summary, ...article.sections.flatMap(s => [s.title, ...s.steps])].join(" ").toLocaleLowerCase();
    return words.every(word => text.includes(word));
  }).sort((a, b) => {
    const relevance = (article: HelpArticle) => words.filter(word => article.title.toLocaleLowerCase().includes(word)).length;
    return relevance(b) - relevance(a);
  });
}
