# SFTP 文件管理器设计文档

**日期**: 2026-05-26
**状态**: 已批准

## 概述

为 Zap 终端添加原生 SFTP 文件管理器功能，使用 `ssh2` crate（libssh2 绑定）实现 SFTP 协议，提供完整的远程文件浏览、传输和管理能力。作为独立 Pane 面板实现，与现有 Server File Browser 并存，无需安装远程 daemon。

## 技术方案

使用 `ssh2` crate 直接实现 SFTP 协议，已在同类项目中验证稳定，SFTP 功能完备（目录遍历、流式传输、权限管理）。

依赖：`ssh2`（libssh2 绑定）、`smol`（异步运行时）、`thiserror`（错误处理）。Windows 上启用 `openssl-on-win32` feature 并 vendored openssl-sys。

## Crate 结构与模块组织

### 协议层 — `crates/warp_sftp/`（新 crate）

```
crates/warp_sftp/
  Cargo.toml
  build.rs                          # Windows 链接 advapi32
  src/
    lib.rs                          # 模块根，导出公开 API
    error.rs                        # SftpError / SftpChannelError
    types.rs                        # FileType / Metadata / DirEntry / OpenOptions 等
    session.rs                      # SftpSession（SSH 连接管理、认证）
    sftp.rs                         # Sftp（SFTP 通道，文件/目录操作）
    dir.rs                          # Dir（目录读取与排序）
    file.rs                         # File（文件读写）
```

### UI 层 — `app/src/sftp_manager/`（新模块）

```
app/src/sftp_manager/
  mod.rs                            # 模块根
  types.rs                          # UI 类型：FileEntry / TransferTask / Dialog / ConnectionState
  sftp_ops.rs                       # 高层操作桥接
  browser.rs                        # SftpBrowserView 主视图
  file_list.rs                      # 文件列表渲染
  breadcrumb.rs                     # 面包屑导航
  context_menu.rs                   # 右键菜单
  dialogs.rs                        # 对话框
  transfer_panel.rs                 # 传输进度面板
```

### Pane 集成

```
app/src/pane_group/pane/sftp_pane.rs（新增）
```

## 核心协议层设计

### session.rs — 连接管理

- `SftpSession`：内部持有 `Arc<ssh2::Session>` + `TcpStream`
- `connect(host, port, username, auth_method) -> Result<SftpSession>`：建立 TCP 连接 → SSH 握手 → 认证
- `sftp() -> Result<Sftp>`：在现有会话上打开 SFTP 子系统
- `disconnect()`：主动断开
- `Drop` 自动断开连接

`AuthMethod` 枚举：`Password(String)` | `PublicKey { path, passphrase }`

### sftp.rs — SFTP 通道操作

- `Sftp`：包装 `Arc<Mutex<ssh2::Sftp>>`，Clone + 线程安全
- 操作：`open`、`create_dir`、`remove_dir`、`remove_file`、`rename`、`stat`、`lstat`、`read_dir`、`symlink`、`readlink`

### dir.rs — 目录读取

- `Dir::read_dir() -> Result<Vec<DirEntry>>`
- 过滤 `.` 和 `..`，转换为 DirEntry
- 排序：目录优先，然后按字母序

### file.rs — 文件读写

- `File`：包装 `ssh2::File`
- 操作：`read_to_end`、`write_all`、`read`（32KB 分块）、`write`（32KB 分块）、`flush`、`stat`

### types.rs — 核心类型

- `FileType`：Dir | File | Symlink | Other
- `FilePermissions`：9-bit Unix 权限（rwxrwxrwx）
- `Metadata`：type, perms, size, uid, gid, atime, mtime
- `DirEntry`：name, path, metadata
- `OpenOptions`：read, write, append, create, truncate
- `WriteMode`：Overwrite | Append | Resume

### error.rs — 错误类型

- `SftpError`：IO | SSH2 | ConnectionFailed | AuthFailed | Timeout | NoSuchFile | PermissionDenied | General
- `SftpChannelError`：Sftp | SendFailed | RecvFailed

## UI 层设计

### browser.rs — SftpBrowserView 主视图

实现 `BackingView` + `TypedActionView` + `View` trait。

**状态**：

| 字段 | 类型 | 说明 |
|------|------|------|
| connection | ConnectionState | Connecting/Connected/Disconnected/Failed |
| _session | Option\<SftpSession\> | 保持 TCP 连接存活 |
| sftp | Option\<Sftp\> | SFTP 通道 |
| current_path | String | 当前目录路径 |
| entries | Vec\<FileEntry\> | 当前目录文件列表 |
| selection | Option\<usize\> | 选中项索引 |
| nav_history | NavHistory | 前进/后退历史 |
| transfers | Vec\<TransferTask\> | 传输队列 |
| dialog | Option\<Dialog\> | 当前弹窗状态 |
| search_filter | Option\<String\> | 搜索过滤 |

**Action 枚举**：

- Connect(node_id)、Disconnect
- NavigateTo(path)、GoBack、GoForward、GoUp、Refresh
- Upload、Download、Delete、Rename、CreateFolder
- Select(index)、Open(index)
- ShowContextMenu(index)
- CancelTransfer(task_id)
- Search(filter)

**渲染结构**（从上到下）：

1. 工具栏：后退/前进/上级/刷新按钮 + 上传按钮 + 新建文件夹按钮
2. 面包屑导航：可点击的路径分段
3. 文件列表：表格式（名称/大小/修改日期），点击选中、双击打开
4. 传输面板：底部可折叠，显示活跃传输任务及进度
5. 右键菜单：打开/下载/重命名/删除/详情
6. 对话框：删除确认、重命名输入、新建文件夹输入、文件详情展示

### sftp_ops.rs — 高层操作桥接

- `connect_from_server(server_info, secret_store) -> Result<(SftpSession, Sftp)>`：从 SSH 管理器读取配置 → 获取凭据 → 建立连接
- `list_dir(sftp, path) -> Result<Vec<FileEntry>>`
- `upload_file_streaming(sftp, local, remote, cancel_flag)`：32KB 分块，AtomicBool 支持取消
- `download_file_streaming(sftp, remote, local, cancel_flag)`：32KB 分块
- `upload_dir_recursive`、`download_dir_recursive`
- `delete_file`、`delete_dir_recursive`、`create_dir`、`rename`
- 并发控制：AtomicUsize CAS 限制最多 2 个并行传输

### 其他 UI 模块

| 模块 | 职责 |
|------|------|
| `file_list.rs` | 文件表头 + 行渲染，目录/文件图标，悬停效果，选中高亮 |
| `breadcrumb.rs` | 从根到当前路径的可点击分段，每段触发 NavigateTo |
| `context_menu.rs` | 右键菜单项：打开/下载/重命名/删除/详情 |
| `dialogs.rs` | 模态弹窗，EditorView 文本输入，Enter 确认 / Escape 取消 |
| `transfer_panel.rs` | 传输方向图标 + 文件名 + 进度百分比 + 进度条 + 状态标签 |

### 键盘快捷键

| 按键 | 动作 |
|------|------|
| Backspace | 返回上级目录 |
| Delete | 删除选中项 |
| Ctrl+Shift+N | 新建文件夹 |
| Escape | 取消搜索 / 关闭对话框 |

## 集成与接入点

### 与 SSH 管理器集成

SFTP 浏览器通过 `warp_ssh_manager` 获取连接信息。

接入点：

- `app/src/ssh_manager/panel.rs`：在服务器右键菜单添加"SFTP 浏览"选项
- `app/src/ssh_manager/server_view.rs`：在服务器详情操作栏添加"SFTP 浏览"按钮

连接流程：

1. 用户在 SSH 主机列表右键某服务器 → 菜单项"SFTP 浏览"
2. 获取 SshServerInfo（host, port, username, auth_type, key_path）
3. 通过 KeychainSecretStore 获取密码/密钥短语
4. 构建 AuthMethod
5. SftpOps::connect_from_server() 建立连接
6. 打开 SftpBrowserView Pane 并显示根目录

### Pane 系统集成

- `app/src/pane_group/pane/sftp_pane.rs`（新增）：`SftpPane` 包装 `SftpBrowserView` 为 `PaneContent`
  - 实现 PaneContent trait
  - 快照序列化为 `LeafContents::Sftp { node_id }`
  - 恢复时根据 node_id 自动重连

注册修改：

- `app/src/pane_group/pane/mod.rs`：声明 sftp_pane 模块
- `app/src/lib.rs`：注册 SftpPane 到 View 系统

### Feature Flag

不使用 Feature Flag，全局始终可用。

## 数据流与错误处理

### 操作数据流

```
用户操作（点击/右键/快捷键）
  → SftpBrowserView 接收 Action
  → dispatch_typed_action() 匹配 Action 类型
  → 异步任务通过 ctx.spawn() 提交：
      ├── 获取 SftpOps / Sftp 实例
      ├── 执行 SFTP 操作（在 smol 线程池运行）
      └── 返回结果到主线程
  → 更新 SftpBrowserView 状态
  → 触发重新渲染
```

### 连接生命周期

```
打开 Pane → Connect(node_id)
  → Connecting 状态（显示加载动画）
  → 成功 → Connected（加载根目录）
  → 失败 → Failed（显示错误信息 + 重试按钮）

关闭 Pane → Drop
  → SftpSession 自动断开（Drop impl）
```

### 错误处理策略

| 场景 | 处理方式 |
|------|----------|
| 连接失败（网络/认证） | 显示错误信息，提供重试按钮，不弹窗 |
| 目录加载失败 | 文件列表区域显示错误提示 + 刷新按钮 |
| 文件操作失败（删除/重命名） | 内联错误提示（Toast 样式），不阻塞 UI |
| 传输失败 | 传输面板标记为 Failed 状态，显示错误原因 |
| 连接中断 | 自动切换到 Disconnected 状态，提示重连 |

所有错误通过 `SftpError` 统一映射为用户可读的中文提示。
