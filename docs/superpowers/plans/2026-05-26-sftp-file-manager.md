# SFTP 文件管理器实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Zap 终端添加原生 SFTP 文件管理器，使用 ssh2 crate 实现协议层，WarpUI 实现浏览器 Pane 面板。

**Architecture:** 新增 `warp_sftp` crate 封装 ssh2 协议操作（连接、文件读写、目录管理），在 `app/src/sftp_manager/` 中实现 WarpUI 浏览器视图，通过 `SftpPane` 集成到 Pane 系统。复用现有 `warp_ssh_manager` 获取主机信息和凭据。

**Tech Stack:** Rust, ssh2 crate (libssh2), smol, thiserror, WarpUI

---

## 文件清单

### 新增文件

| 文件 | 职责 |
|------|------|
| `crates/warp_sftp/Cargo.toml` | crate 依赖声明 |
| `crates/warp_sftp/build.rs` | Windows 链接 advapi32 |
| `crates/warp_sftp/src/lib.rs` | 模块根，导出公开 API |
| `crates/warp_sftp/src/error.rs` | SftpError / SftpChannelError |
| `crates/warp_sftp/src/types.rs` | FileType / Metadata / DirEntry / OpenOptions 等 |
| `crates/warp_sftp/src/session.rs` | SftpSession（SSH 连接管理、认证） |
| `crates/warp_sftp/src/sftp.rs` | Sftp（SFTP 通道，文件/目录操作） |
| `crates/warp_sftp/src/dir.rs` | Dir（目录读取与排序） |
| `crates/warp_sftp/src/file.rs` | File（文件读写） |
| `app/src/sftp_manager/mod.rs` | UI 模块根 |
| `app/src/sftp_manager/types.rs` | UI 类型 |
| `app/src/sftp_manager/sftp_ops.rs` | 高层操作桥接 |
| `app/src/sftp_manager/browser.rs` | SftpBrowserView 主视图 |
| `app/src/sftp_manager/file_list.rs` | 文件列表渲染 |
| `app/src/sftp_manager/breadcrumb.rs` | 面包屑导航 |
| `app/src/sftp_manager/context_menu.rs` | 右键菜单 |
| `app/src/sftp_manager/dialogs.rs` | 对话框 |
| `app/src/sftp_manager/transfer_panel.rs` | 传输进度面板 |
| `app/src/pane_group/pane/sftp_pane.rs` | SftpPane（PaneContent 实现） |

### 修改文件

| 文件 | 修改内容 |
|------|----------|
| `Cargo.toml` | workspace members 自动包含（crates/*），无需修改 |
| `app/Cargo.toml` | 添加 warp_sftp 依赖 |
| `app/src/lib.rs` | 声明 sftp_manager 模块 |
| `app/src/app_state.rs` | 添加 LeafContents::Sftp 变体 |
| `app/src/pane_group/pane/mod.rs` | 添加 IPaneType::Sftp + Display + PaneId + render + 模块声明 |
| `app/src/pane_group/mod.rs` | restore_leaf_from_snapshot 添加 Sftp 分支 |
| `app/src/ssh_manager/panel.rs` | 添加右键菜单"SFTP 浏览"选项 |
| `app/src/workspace/view.rs` | 添加 open_sftp_pane 方法 |

---

### Task 1: 创建分支并初始化 warp_sftp crate

**Files:**
- Create: `crates/warp_sftp/Cargo.toml`
- Create: `crates/warp_sftp/build.rs`
- Create: `crates/warp_sftp/src/lib.rs`
- Create: `crates/warp_sftp/src/error.rs`
- Create: `crates/warp_sftp/src/types.rs`
- Create: `crates/warp_sftp/src/session.rs`
- Create: `crates/warp_sftp/src/sftp.rs`
- Create: `crates/warp_sftp/src/dir.rs`
- Create: `crates/warp_sftp/src/file.rs`

- [ ] **Step 1: 创建 feature 分支**

```bash
git checkout -b feature/sftp-manager
```

- [ ] **Step 2: 创建 crate 目录结构**

```bash
mkdir -p crates/warp_sftp/src
```

- [ ] **Step 3: 创建 Cargo.toml**

```toml
[package]
name = "warp_sftp"
version = "0.1.0"
edition = "2021"

[dependencies]
ssh2 = { version = "0.9", features = ["openssl-on-win32"] }
openssl-sys = { version = "*", features = ["vendored"] }
smol = "2"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: 创建 build.rs**

```rust
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
```

- [ ] **Step 5: 创建 error.rs**

```rust
use thiserror::Error;

/// SFTP 协议级错误
#[derive(Debug, Error)]
pub enum SftpError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("SSH2 错误: {0}")]
    Ssh2(#[from] ssh2::Error),

    #[error("连接失败: {0}")]
    ConnectionFailed(String),

    #[error("认证失败: {0}")]
    AuthFailed(String),

    #[error("操作超时")]
    Timeout,

    #[error("文件未找到: {0}")]
    NoSuchFile(String),

    #[error("权限不足: {0}")]
    PermissionDenied(String),

    #[error("操作失败: {0}")]
    General(String),
}

/// SFTP 通道错误
#[derive(Debug, Error)]
pub enum SftpChannelError {
    #[error("SFTP 错误: {0}")]
    Sftp(#[from] SftpError),

    #[error("发送请求失败: {0}")]
    SendFailed(String),

    #[error("接收响应失败: {0}")]
    RecvFailed(String),
}

impl From<ssh2::Error> for SftpChannelError {
    fn from(e: ssh2::Error) -> Self {
        SftpChannelError::Sftp(SftpError::Ssh2(e))
    }
}
```

- [ ] **Step 6: 创建 types.rs**

```rust
use std::path::PathBuf;

/// 文件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Dir,
    File,
    Symlink,
    Other,
}

impl FileType {
    /// 从 unix 权限 mode 位解析文件类型
    pub fn from_mode(mode: u32) -> Self {
        match mode & 0o170000 {
            0o040000 => FileType::Dir,
            0o100000 => FileType::File,
            0o120000 => FileType::Symlink,
            _ => FileType::Other,
        }
    }
}

/// 文件权限（Unix 风格）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePermissions {
    pub owner_read: bool,
    pub owner_write: bool,
    pub owner_exec: bool,
    pub group_read: bool,
    pub group_write: bool,
    pub group_exec: bool,
    pub other_read: bool,
    pub other_write: bool,
    pub other_exec: bool,
}

impl FilePermissions {
    /// 从 unix mode 位解析权限
    pub fn from_mode(mode: u32) -> Self {
        Self {
            owner_read: mode & 0o400 != 0,
            owner_write: mode & 0o200 != 0,
            owner_exec: mode & 0o100 != 0,
            group_read: mode & 0o040 != 0,
            group_write: mode & 0o020 != 0,
            group_exec: mode & 0o010 != 0,
            other_read: mode & 0o004 != 0,
            other_write: mode & 0o002 != 0,
            other_exec: mode & 0o001 != 0,
        }
    }
}

/// 文件元数据
#[derive(Debug, Clone)]
pub struct Metadata {
    pub file_type: FileType,
    pub permissions: FilePermissions,
    pub size: u64,
    pub uid: u32,
    pub gid: u32,
    pub accessed: Option<std::time::SystemTime>,
    pub modified: Option<std::time::SystemTime>,
}

impl Metadata {
    /// 从 ssh2::FileStat 创建
    pub fn from_ssh2(m: ssh2::FileStat) -> Self {
        let file_type = if m.is_dir() {
            FileType::Dir
        } else if m.is_file() {
            FileType::File
        } else {
            FileType::Other
        };
        Self {
            file_type,
            permissions: FilePermissions::from_mode(m.perm.unwrap_or(0)),
            size: m.size.unwrap_or(0),
            uid: m.uid.unwrap_or(0),
            gid: m.gid.unwrap_or(0),
            accessed: m.atime.map(|t| {
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(t)
            }),
            modified: m.mtime.map(|t| {
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(t)
            }),
        }
    }
}

/// 写入模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Write,
    Append,
}

/// 打开文件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFileType {
    File,
    Dir,
}

/// 文件打开选项
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub read: bool,
    pub write: Option<WriteMode>,
    pub create: bool,
    pub truncate: bool,
    pub mode: Option<u32>,
    pub file_type: OpenFileType,
}

impl OpenOptions {
    pub fn read() -> Self {
        Self {
            read: true,
            write: None,
            create: false,
            truncate: false,
            mode: None,
            file_type: OpenFileType::File,
        }
    }

    pub fn write() -> Self {
        Self {
            read: false,
            write: Some(WriteMode::Write),
            create: true,
            truncate: true,
            mode: Some(0o644),
            file_type: OpenFileType::File,
        }
    }

    pub fn append() -> Self {
        Self {
            read: false,
            write: Some(WriteMode::Append),
            create: true,
            truncate: false,
            mode: Some(0o644),
            file_type: OpenFileType::File,
        }
    }

    pub fn create_new() -> Self {
        Self {
            read: false,
            write: Some(WriteMode::Write),
            create: true,
            truncate: false,
            mode: Some(0o644),
            file_type: OpenFileType::File,
        }
    }
}

/// 重命名选项
#[derive(Debug, Clone, Default)]
pub struct RenameOptions {
    pub overwrite: bool,
    pub atomic: bool,
    pub native: bool,
}

/// 目录条目
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub metadata: Metadata,
}
```

- [ ] **Step 7: 创建 session.rs**

```rust
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::SftpError;
use crate::sftp::Sftp;

/// 认证方式
#[derive(Debug, Clone)]
pub enum AuthMethod {
    Password { password: String },
    PublicKey { key_path: PathBuf, passphrase: Option<String> },
}

/// SFTP 会话，封装 ssh2 连接
pub struct SftpSession {
    session: Arc<ssh2::Session>,
    _tcp: TcpStream,
}

impl SftpSession {
    /// 通过指定参数建立 SSH 连接
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
    ) -> Result<Self, SftpError> {
        let addr = format!("{host}:{port}");
        let tcp = TcpStream::connect(&addr)
            .map_err(|e| SftpError::ConnectionFailed(format!("连接 {addr} 失败: {e}")))?;

        let mut session = ssh2::Session::new()
            .map_err(|e| SftpError::ConnectionFailed(format!("创建 SSH 会话失败: {e}")))?;

        let tcp_for_session = tcp.try_clone()
            .map_err(|e| SftpError::ConnectionFailed(format!("克隆 TCP 流失败: {e}")))?;
        session.set_tcp_stream(tcp_for_session);
        session.handshake()
            .map_err(|e| SftpError::ConnectionFailed(format!("SSH 握手失败: {e}")))?;

        match &auth {
            AuthMethod::Password { password } => {
                session.userauth_password(username, password)
                    .map_err(|e| SftpError::AuthFailed(format!("密码认证失败: {e}")))?;
            }
            AuthMethod::PublicKey { key_path, passphrase } => {
                let pass = passphrase.as_deref();
                session.userauth_pubkey_file(username, None, key_path, pass)
                    .map_err(|e| SftpError::AuthFailed(format!("密钥认证失败: {e}")))?;
            }
        }

        if !session.authenticated() {
            return Err(SftpError::AuthFailed("认证未通过".into()));
        }

        Ok(Self {
            session: Arc::new(session),
            _tcp: tcp,
        })
    }

    /// 获取 SFTP 通道
    pub fn sftp(&self) -> Result<Sftp, SftpError> {
        let sftp = self.session.sftp()?;
        Ok(Sftp::new(sftp))
    }

    /// 断开连接
    pub fn disconnect(&self) -> Result<(), SftpError> {
        self.session.disconnect(None, "bye", None)?;
        Ok(())
    }

    /// 检查连接是否存活
    pub fn is_authenticated(&self) -> bool {
        self.session.authenticated()
    }
}

impl Drop for SftpSession {
    fn drop(&mut self) {
        let _ = self.session.disconnect(None, "bye", None);
    }
}
```

- [ ] **Step 8: 创建 sftp.rs**

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use crate::dir::Dir;
use crate::error::SftpError;
use crate::file::File;
use crate::types::{DirEntry, Metadata, OpenOptions, RenameOptions};

/// SFTP 通道，所有远程文件系统操作的入口
#[derive(Clone)]
pub struct Sftp {
    inner: Arc<Mutex<ssh2::Sftp>>,
}

impl Sftp {
    /// 从 ssh2::Sftp 创建 Sftp 实例
    pub(crate) fn new(sftp: ssh2::Sftp) -> Self {
        Self {
            inner: Arc::new(Mutex::new(sftp)),
        }
    }

    /// 打开远程文件
    pub fn open(&self, path: &Path, options: OpenOptions) -> Result<File, SftpError> {
        let sftp = self.inner.lock().unwrap();
        File::open(&sftp, path, &options)
    }

    /// 创建目录
    pub fn create_dir(&self, path: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.mkdir(path, 0o755)?;
        Ok(())
    }

    /// 删除目录（必须为空）
    pub fn remove_dir(&self, path: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.rmdir(path)?;
        Ok(())
    }

    /// 删除文件
    pub fn remove_file(&self, path: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.unlink(path)?;
        Ok(())
    }

    /// 重命名/移动
    pub fn rename(&self, src: &Path, dst: &Path, opts: RenameOptions) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        let mut flags = ssh2::RenameFlags::empty();
        if opts.overwrite {
            flags |= ssh2::RenameFlags::OVERWRITE;
        }
        if opts.atomic {
            flags |= ssh2::RenameFlags::ATOMIC;
        }
        if opts.native {
            flags |= ssh2::RenameFlags::NATIVE;
        }
        sftp.rename(src, dst, Some(flags))?;
        Ok(())
    }

    /// 获取文件元数据（跟随符号链接）
    pub fn stat(&self, path: &Path) -> Result<Metadata, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let stat = sftp.stat(path)?;
        Ok(Metadata::from_ssh2(stat))
    }

    /// 获取文件元数据（不跟随符号链接）
    pub fn lstat(&self, path: &Path) -> Result<Metadata, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let stat = sftp.lstat(path)?;
        Ok(Metadata::from_ssh2(stat))
    }

    /// 读取目录内容
    pub fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>, SftpError> {
        let sftp = self.inner.lock().unwrap();
        Dir::read_dir(&sftp, path)
    }

    /// 创建符号链接
    pub fn symlink(&self, src: &Path, dst: &Path) -> Result<(), SftpError> {
        let sftp = self.inner.lock().unwrap();
        sftp.symlink(src, dst)?;
        Ok(())
    }

    /// 读取符号链接目标
    pub fn readlink(&self, path: &Path) -> Result<PathBuf, SftpError> {
        let sftp = self.inner.lock().unwrap();
        let target = sftp.readlink(path)?;
        Ok(target)
    }
}
```

- [ ] **Step 9: 创建 dir.rs**

```rust
use std::path::Path;

use crate::error::SftpError;
use crate::types::{DirEntry, FileType, Metadata};

/// SFTP 远程目录操作
pub struct Dir;

impl Dir {
    /// 读取远程目录内容
    pub(crate) fn read_dir(sftp: &ssh2::Sftp, path: &Path) -> Result<Vec<DirEntry>, SftpError> {
        let mut entries = Vec::new();
        for entry in sftp.readdir(path)? {
            let name = entry
                .0
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name == "." || name == ".." {
                continue;
            }
            let metadata = Metadata::from_ssh2(entry.1);
            entries.push(DirEntry {
                name,
                path: entry.0,
                metadata,
            });
        }
        entries.sort_by(|a, b| {
            let a_is_dir = a.metadata.file_type == FileType::Dir;
            let b_is_dir = b.metadata.file_type == FileType::Dir;
            b_is_dir
                .cmp(&a_is_dir)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }
}
```

- [ ] **Step 10: 创建 file.rs**

```rust
use std::io::{Read, Write};

use crate::error::SftpError;
use crate::types::OpenOptions;

/// SFTP 远程文件句柄
pub struct File {
    handle: ssh2::File,
}

impl File {
    /// 打开远程文件
    pub(crate) fn open(
        sftp: &ssh2::Sftp,
        path: &std::path::Path,
        options: &OpenOptions,
    ) -> Result<Self, SftpError> {
        let mut flags = ssh2::OpenFlags::empty();
        if options.read {
            flags |= ssh2::OpenFlags::READ;
        }
        if options.write.is_some() {
            flags |= ssh2::OpenFlags::WRITE;
        }
        if options.create && options.truncate {
            flags |= ssh2::OpenFlags::CREATE;
            flags |= ssh2::OpenFlags::TRUNCATE;
        } else if options.create {
            flags |= ssh2::OpenFlags::CREATE;
        }
        if matches!(options.write, Some(crate::types::WriteMode::Append)) {
            flags |= ssh2::OpenFlags::APPEND;
        }

        let handle = sftp.open_mode(
            path,
            flags,
            options.mode.unwrap_or(0o644) as i32,
            ssh2::OpenType::File,
        )?;
        Ok(File { handle })
    }

    /// 读取文件全部内容
    pub fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<u64, SftpError> {
        let n = self.handle.read_to_end(buf)?;
        Ok(n as u64)
    }

    /// 写入全部内容
    pub fn write_all(&mut self, buf: &[u8]) -> Result<(), SftpError> {
        self.handle.write_all(buf)?;
        Ok(())
    }

    /// 刷新写入缓冲
    pub fn flush(&mut self) -> Result<(), SftpError> {
        self.handle.flush()?;
        Ok(())
    }

    /// 获取文件元数据
    pub fn stat(&mut self) -> Result<crate::types::Metadata, SftpError> {
        let stat = self.handle.stat()?;
        Ok(crate::types::Metadata::from_ssh2(stat))
    }

    /// 读取一块数据到缓冲区
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, SftpError> {
        use std::io::Read;
        let n = self.handle.read(buf)?;
        Ok(n)
    }
}
```

- [ ] **Step 11: 创建 lib.rs**

```rust
pub mod dir;
pub mod error;
pub mod file;
pub mod session;
pub mod sftp;
pub mod types;

pub use dir::Dir;
pub use error::{SftpChannelError, SftpError};
pub use file::File;
pub use session::{AuthMethod, SftpSession};
pub use sftp::Sftp;
pub use types::*;
```

- [ ] **Step 12: 验证 crate 编译**

Run: `cargo check -p warp_sftp`
Expected: 编译成功（可能需要下载 openssl-sys vendored 依赖）

- [ ] **Step 13: 提交**

```bash
git add crates/warp_sftp/
git commit -m "feat: 添加 warp_sftp crate，实现 SFTP 协议层"
```

---

### Task 2: 添加 warp_sftp 依赖到 app 并创建 UI 模块骨架

**Files:**
- Modify: `app/Cargo.toml`
- Modify: `app/src/lib.rs`
- Create: `app/src/sftp_manager/mod.rs`
- Create: `app/src/sftp_manager/types.rs`
- Create: `app/src/sftp_manager/sftp_ops.rs`

- [ ] **Step 1: 在 app/Cargo.toml 添加依赖**

在 `[dependencies]` 部分找到 `warp_ssh_manager` 行附近，添加：

```toml
warp_sftp = { path = "crates/warp_sftp" }
```

- [ ] **Step 2: 在 app/src/lib.rs 声明模块**

找到其他模块声明（如 `pub mod ssh_manager;`），在附近添加：

```rust
pub mod sftp_manager;
```

- [ ] **Step 3: 创建 sftp_manager/mod.rs**

```rust
pub mod breadcrumb;
pub mod browser;
pub mod context_menu;
pub mod dialogs;
pub mod file_list;
pub mod sftp_ops;
pub mod transfer_panel;
pub mod types;

#[allow(unused_imports)]
pub use browser::{SftpBrowserAction, SftpBrowserView};
#[allow(unused_imports)]
pub use types::*;
```

- [ ] **Step 4: 创建 sftp_manager/types.rs**

```rust
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 文件条目类型（UI 层）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEntryType {
    File,
    Directory,
    Symlink,
    Other,
}

/// 文件条目（UI 展示用）
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub file_type: FileEntryType,
    pub size: u64,
    pub modified: Option<String>,
    pub permissions: Option<String>,
}

/// 传输方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

/// 传输状态
#[derive(Debug, Clone)]
pub enum TransferState {
    Pending,
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}

/// 传输任务
#[derive(Debug, Clone)]
pub struct TransferTask {
    pub id: usize,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub direction: TransferDirection,
    pub total_size: u64,
    pub transferred: u64,
    pub state: TransferState,
    pub cancel_flag: Arc<AtomicBool>,
}

impl TransferTask {
    /// 创建新的传输任务
    pub fn new(
        id: usize,
        source_path: PathBuf,
        target_path: PathBuf,
        direction: TransferDirection,
        total_size: u64,
    ) -> Self {
        Self {
            id,
            source_path,
            target_path,
            direction,
            total_size,
            transferred: 0,
            state: TransferState::Pending,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 计算进度百分比 (0-100)
    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 0;
        }
        ((self.transferred as f64 / self.total_size as f64) * 100.0) as u8
    }

    /// 取消传输
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    /// 检查是否已取消
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }
}

/// 对话框类型
#[derive(Debug, Clone)]
pub enum Dialog {
    DeleteConfirm { paths: Vec<PathBuf> },
    Rename {
        path: PathBuf,
        original_name: String,
    },
    CreateFolder {
        parent_path: PathBuf,
    },
    Move {
        source: PathBuf,
        target_dir: PathBuf,
    },
    OverwriteConfirm {
        source: PathBuf,
        target: PathBuf,
    },
    FileDetails { entry: FileEntry },
}

/// 连接状态
#[derive(Debug)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Failed(String),
}

/// 格式化文件大小
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{size} B")
    }
}
```

- [ ] **Step 5: 创建 sftp_manager/sftp_ops.rs**

```rust
//! SFTP 操作封装层

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use warp_sftp::session::{AuthMethod, SftpSession};
use warp_sftp::types::OpenOptions;
use warp_sftp::Sftp;
use warp_ssh_manager::secrets::{SecretKind, SshSecretStore};
use warp_ssh_manager::types::{AuthType, SshServerInfo};

use super::types::{FileEntry, FileEntryType};

/// 最大并行传输数
const MAX_PARALLEL_TRANSFERS: usize = 2;

/// SFTP 操作错误
#[derive(Debug)]
pub enum SftpOpsError {
    Connection(String),
    Operation(String),
    LocalIo(String),
    NoCredentials(String),
    Cancelled,
}

impl std::fmt::Display for SftpOpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SftpOpsError::Connection(msg) => write!(f, "连接错误: {msg}"),
            SftpOpsError::Operation(msg) => write!(f, "操作错误: {msg}"),
            SftpOpsError::LocalIo(msg) => write!(f, "本地 IO 错误: {msg}"),
            SftpOpsError::NoCredentials(msg) => write!(f, "未找到凭据: {msg}"),
            SftpOpsError::Cancelled => write!(f, "传输已取消"),
        }
    }
}

impl From<warp_sftp::SftpError> for SftpOpsError {
    fn from(e: warp_sftp::SftpError) -> Self {
        SftpOpsError::Operation(e.to_string())
    }
}

impl From<std::io::Error> for SftpOpsError {
    fn from(e: std::io::Error) -> Self {
        SftpOpsError::LocalIo(e.to_string())
    }
}

/// 进度回调类型
pub type ProgressCallback = Box<dyn Fn(u64, u64) + Send>;

/// 使用服务器配置建立 SFTP 连接
pub fn connect_from_server(
    server: &SshServerInfo,
    secret_store: &dyn SshSecretStore,
) -> Result<SftpSession, SftpOpsError> {
    let auth = build_auth_method(server, secret_store)?;
    SftpSession::connect(&server.host, server.port, &server.username, auth)
        .map_err(|e| SftpOpsError::Connection(e.to_string()))
}

/// 列出远程目录内容
pub fn list_dir(sftp: &Sftp, path: &Path) -> Result<Vec<FileEntry>, SftpOpsError> {
    let entries = sftp.read_dir(path)?;
    let result = entries
        .into_iter()
        .map(|entry| {
            let file_type = match entry.metadata.file_type {
                warp_sftp::types::FileType::Dir => FileEntryType::Directory,
                warp_sftp::types::FileType::File => FileEntryType::File,
                warp_sftp::types::FileType::Symlink => FileEntryType::Symlink,
                warp_sftp::types::FileType::Other => FileEntryType::Other,
            };
            let modified = entry.metadata.modified.map(|t| {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M").to_string()
            });
            let perms = &entry.metadata.permissions;
            let permissions = Some(format!(
                "{}{}{}{}{}{}{}{}{}",
                if perms.owner_read { 'r' } else { '-' },
                if perms.owner_write { 'w' } else { '-' },
                if perms.owner_exec { 'x' } else { '-' },
                if perms.group_read { 'r' } else { '-' },
                if perms.group_write { 'w' } else { '-' },
                if perms.group_exec { 'x' } else { '-' },
                if perms.other_read { 'r' } else { '-' },
                if perms.other_write { 'w' } else { '-' },
                if perms.other_exec { 'x' } else { '-' },
            ));
            FileEntry {
                name: entry.name,
                path: entry.path,
                file_type,
                size: entry.metadata.size,
                modified,
                permissions,
            }
        })
        .collect();
    Ok(result)
}

/// 删除远程文件
pub fn delete_file(sftp: &Sftp, path: &Path) -> Result<(), SftpOpsError> {
    sftp.remove_file(path)?;
    Ok(())
}

/// 递归删除远程目录
pub fn delete_dir_recursive(sftp: &Sftp, path: &Path) -> Result<(), SftpOpsError> {
    let entries = sftp.read_dir(path)?;
    for entry in entries {
        match entry.metadata.file_type {
            warp_sftp::types::FileType::Dir => {
                delete_dir_recursive(sftp, &entry.path)?;
            }
            _ => {
                sftp.remove_file(&entry.path)?;
            }
        }
    }
    sftp.remove_dir(path)?;
    Ok(())
}

/// 创建远程目录
pub fn create_dir(sftp: &Sftp, path: &Path) -> Result<(), SftpOpsError> {
    sftp.create_dir(path)?;
    Ok(())
}

/// 重命名远程文件或目录
pub fn rename(sftp: &Sftp, old_path: &Path, new_path: &Path) -> Result<(), SftpOpsError> {
    let opts = warp_sftp::types::RenameOptions {
        overwrite: false,
        atomic: false,
        native: false,
    };
    sftp.rename(old_path, new_path, opts)?;
    Ok(())
}

/// 流式上传本地文件到远程
pub fn upload_file_streaming(
    sftp: &Sftp,
    local_path: &Path,
    remote_path: &Path,
    progress_cb: Option<&ProgressCallback>,
) -> Result<(), SftpOpsError> {
    let mut local_file =
        fs::File::open(local_path).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
    let total_size = local_file.metadata().map(|m| m.len()).unwrap_or(0);

    let mut remote_file = sftp.open(remote_path, OpenOptions::write())?;

    const CHUNK_SIZE: usize = 32 * 1024;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = 0;

    loop {
        let n = std::io::Read::read(&mut local_file, &mut buf)
            .map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
        if n == 0 {
            break;
        }
        remote_file.write_all(&buf[..n])?;
        transferred += n as u64;
        if let Some(cb) = progress_cb {
            cb(transferred, total_size);
        }
    }

    remote_file.flush()?;
    Ok(())
}

/// 流式下载远程文件到本地
pub fn download_file_streaming(
    sftp: &Sftp,
    remote_path: &Path,
    local_path: &Path,
    progress_cb: Option<&ProgressCallback>,
) -> Result<(), SftpOpsError> {
    let mut remote_file = sftp.open(remote_path, OpenOptions::read())?;
    let metadata = remote_file.stat()?;
    let total_size = metadata.size;

    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
    }

    let mut local_file =
        fs::File::create(local_path).map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;

    const CHUNK_SIZE: usize = 32 * 1024;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = 0;

    loop {
        let n = remote_file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        local_file
            .write_all(&buf[..n])
            .map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
        transferred += n as u64;
        if let Some(cb) = progress_cb {
            cb(transferred, total_size);
        }
    }

    local_file.flush().map_err(|e| SftpOpsError::LocalIo(e.to_string()))?;
    Ok(())
}

/// 根据服务器配置构建认证方式
fn build_auth_method(
    server: &SshServerInfo,
    secret_store: &dyn SshSecretStore,
) -> Result<AuthMethod, SftpOpsError> {
    match server.auth_type {
        AuthType::Password => {
            let password = secret_store
                .get(&server.node_id, SecretKind::Password)
                .map_err(|e| SftpOpsError::NoCredentials(format!("读取密码失败: {e}")))?
                .ok_or_else(|| {
                    SftpOpsError::NoCredentials(format!(
                        "服务器 {} 未存储密码",
                        server.host
                    ))
                })?;
            Ok(AuthMethod::Password {
                password: password.to_string(),
            })
        }
        AuthType::Key => {
            let key_path = server
                .key_path
                .as_ref()
                .ok_or_else(|| {
                    SftpOpsError::NoCredentials("密钥认证但未指定密钥路径".to_string())
                })?;
            let expanded = shellexpand_path(key_path);
            let passphrase = secret_store
                .get(&server.node_id, SecretKind::Passphrase)
                .ok()
                .flatten()
                .map(|p| p.to_string());
            Ok(AuthMethod::PublicKey {
                key_path: PathBuf::from(expanded),
                passphrase,
            })
        }
    }
}

/// 展开路径中的 ~ 为用户主目录
fn shellexpand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}/{}", home.display(), &path[2..]);
        }
    }
    path.to_string()
}
```

- [ ] **Step 6: 创建其余 UI 文件的占位模块（确保编译通过）**

创建 `browser.rs`：

```rust
//! SFTP 浏览器视图（占位，Task 3 完善）

use std::path::PathBuf;

/// SFTP 浏览器 Action
#[derive(Debug, Clone)]
pub enum SftpBrowserAction {
    NavigateTo(PathBuf),
    GoUp,
}

/// SFTP 浏览器视图（占位）
pub struct SftpBrowserView;

impl SftpBrowserView {
    pub fn new(_node_id: String, _ctx: &mut warpui::ViewContext<Self>) -> Self {
        Self
    }
    pub fn pane_configuration(&self) -> warpui::ModelHandle<crate::pane_group::PaneConfiguration> {
        unimplemented!("Task 3 中实现")
    }
}

impl warpui::Entity for SftpBrowserView {
    type Event = crate::pane_group::PaneEvent;
}

impl warpui::TypedActionView for SftpBrowserView {
    type Action = SftpBrowserAction;
    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut warpui::ViewContext<Self>) {}
}

impl warpui::View for SftpBrowserView {
    fn ui_name() -> &'static str { "SftpBrowserView" }
    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        use warpui::elements::Flex;
        Flex::column().finish()
    }
}

impl crate::pane_group::BackingView for SftpBrowserView {
    type PaneHeaderOverflowMenuAction = SftpBrowserAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut warpui::ViewContext<Self>,
    ) {}
    fn close(&mut self, _ctx: &mut warpui::ViewContext<Self>) {
        _ctx.emit(crate::pane_group::PaneEvent::Close);
    }
    fn focus_contents(&mut self, _ctx: &mut warpui::ViewContext<Self>) {}
    fn render_header_content(
        &self,
        _ctx: &crate::pane_group::pane::view::HeaderRenderContext<'_>,
        _app: &warpui::AppContext,
    ) -> crate::pane_group::pane::view::HeaderContent {
        crate::pane_group::pane::view::HeaderContent::simple("SFTP Browser".to_string())
    }
    fn set_focus_handle(
        &mut self,
        _focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        _ctx: &mut warpui::ViewContext<Self>,
    ) {}
}
```

创建 `file_list.rs`：

```rust
//! SFTP 文件列表渲染（占位）
use warpui::Element;
use warp_core::ui::appearance::Appearance;
use std::collections::HashSet;
use warpui::elements::MouseStateHandle;
use super::types::FileEntry;

pub fn render_header(_appearance: &Appearance) -> Box<dyn Element> {
    warpui::elements::Flex::column().finish()
}

pub fn render_file_rows(
    _entries: &[FileEntry],
    _selected: &HashSet<usize>,
    _mouse_handles: &[MouseStateHandle],
    _appearance: &Appearance,
) -> Box<dyn Element> {
    warpui::elements::Flex::column().finish()
}
```

创建 `breadcrumb.rs`：

```rust
//! SFTP 面包屑导航（占位）
use std::path::PathBuf;
use warpui::Element;
use warp_core::ui::appearance::Appearance;

pub fn render_breadcrumb(_current_path: &PathBuf, _appearance: &Appearance) -> Vec<Box<dyn Element>> {
    Vec::new()
}
```

创建 `context_menu.rs`：

```rust
//! SFTP 右键菜单（占位）
use warpui::Element;
use warp_core::ui::appearance::Appearance;

#[derive(Debug)]
pub struct ContextMenuState {
    pub entry_index: usize,
    pub position: (f32, f32),
}

impl ContextMenuState {
    pub fn new(entry_index: usize, position: (f32, f32)) -> Self {
        Self { entry_index, position }
    }
}

pub fn render_context_menu(_state: &ContextMenuState, _appearance: &Appearance) -> Box<dyn Element> {
    warpui::elements::Flex::column().finish()
}
```

创建 `dialogs.rs`：

```rust
//! SFTP 对话框渲染（占位）
use warpui::Element;
use warp_core::ui::appearance::Appearance;
use crate::editor::EditorView;
use super::types::Dialog;

pub fn render_dialog(
    _dialog: &Dialog,
    _rename_editor: &warpui::ViewHandle<EditorView>,
    _new_folder_editor: &warpui::ViewHandle<EditorView>,
    _appearance: &Appearance,
) -> Box<dyn Element> {
    warpui::elements::Flex::column().finish()
}
```

创建 `transfer_panel.rs`：

```rust
//! SFTP 传输面板（占位）
use warpui::Element;
use warp_core::ui::appearance::Appearance;
use super::types::TransferTask;

pub fn render_transfer_panel(
    _transfers: &[TransferTask],
    _is_expanded: bool,
    _appearance: &Appearance,
) -> Box<dyn Element> {
    warpui::elements::Flex::column().finish()
}
```

- [ ] **Step 7: 验证编译**

Run: `cargo check -p warp`
Expected: 编译成功（可能有 unused warnings，正常）

- [ ] **Step 8: 提交**

```bash
git add app/Cargo.toml app/src/lib.rs app/src/sftp_manager/
git commit -m "feat: 添加 sftp_manager UI 模块骨架和操作封装层"
```

---

### Task 3: 实现 SftpPane 和 Pane 系统集成

**Files:**
- Create: `app/src/pane_group/pane/sftp_pane.rs`
- Modify: `app/src/app_state.rs` — 添加 LeafContents::Sftp
- Modify: `app/src/pane_group/pane/mod.rs` — 添加 IPaneType::Sftp 及相关注册
- Modify: `app/src/pane_group/mod.rs` — restore_leaf_from_snapshot 添加 Sftp 分支

- [ ] **Step 1: 在 app_state.rs 的 LeafContents 枚举中添加 Sftp 变体**

在 `LeafContents::SshServer { node_id: String }` 后面添加：

```rust
Sftp { node_id: String },
```

在 `is_persisted()` 方法中，`LeafContents::SshServer { .. } => false,` 后面添加：

```rust
LeafContents::Sftp { .. } => false,
```

- [ ] **Step 2: 在 pane/mod.rs 中添加 IPaneType::Sftp**

在 `IPaneType` 枚举的 `SshServer` 后面添加 `Sftp` 变体。

在 `Display` impl 中添加：

```rust
IPaneType::Sftp => write!(f, "SFTP"),
```

在 `PaneId` 中添加工厂方法：

```rust
pub fn from_sftp_pane_ctx(ctx: &ViewContext<PaneView<SftpBrowserView>>) -> Self {
    Self::new_from_ctx(IPaneType::Sftp, ctx)
}

pub fn from_sftp_pane_view(
    sftp_pane_view: &ViewHandle<PaneView<SftpBrowserView>>,
) -> Self {
    Self::new(IPaneType::Sftp, sftp_pane_view)
}
```

在 `PaneId::render` 中添加：

```rust
IPaneType::Sftp => {
    ChildView::<PaneView<SftpBrowserView>>::with_id(self.0.pane_view_id).finish()
}
```

在模块声明中添加：

```rust
pub(crate) mod sftp_pane;
```

在文件顶部的 use 语句中，找到 SshServerView 的导入，在附近添加：

```rust
use crate::sftp_manager::browser::SftpBrowserView;
```

- [ ] **Step 3: 创建 sftp_pane.rs**

```rust
use warpui::{AppContext, ModelHandle, ViewContext, ViewHandle};

use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{
    BackingView, DetachType, LeafContents, PaneConfiguration, PaneContent, PaneEvent,
    PaneGroup, PaneId,
};
use crate::sftp_manager::browser::SftpBrowserView;

use super::view::{ChildView, PaneView};

pub struct SftpPane {
    view: ViewHandle<PaneView<SftpBrowserView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    node_id: String,
}

impl SftpPane {
    pub fn new(node_id: String, ctx: &mut ViewContext<impl warpui::View>) -> Self {
        let id_for_view = node_id.clone();
        let server_view =
            ctx.add_typed_action_view(move |ctx| SftpBrowserView::new(id_for_view, ctx));
        let pane_configuration = server_view.as_ref(ctx).pane_configuration();
        let pane_view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_sftp_pane_ctx(ctx);
            PaneView::new(pane_id, server_view, (), pane_configuration.clone(), ctx)
        });
        Self { view: pane_view, pane_configuration, node_id }
    }
}

impl PaneContent for SftpPane {
    fn id(&self) -> PaneId { PaneId::from_sftp_pane_view(&self.view) }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view.update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));
        let child = self.view.as_ref(ctx).child(ctx);
        let pane_id = self.id();
        ctx.subscribe_to_view(&child, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let child = self.view.as_ref(ctx).child(ctx);
        ctx.unsubscribe_to_view(&child);
    }

    fn snapshot(&self, _ctx: &AppContext) -> LeafContents {
        LeafContents::Sftp { node_id: self.node_id.clone() }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.view.as_ref(ctx).child(ctx).update(ctx, BackingView::focus_contents)
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<crate::pane_group::ShareableLink, crate::pane_group::ShareableLinkError> {
        Ok(crate::pane_group::ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
```

- [ ] **Step 4: 在 pane_group/mod.rs 的 restore_leaf_from_snapshot 中添加 Sftp 分支**

找到 `LeafContents::SshServer { .. }` 的 match arm，在其后添加：

```rust
LeafContents::Sftp { .. } => {
    Err(anyhow::anyhow!(
        "SFTP pane should not have been persisted, as it cannot be restored"
    ))
}
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p warp`
Expected: 编译成功

- [ ] **Step 6: 提交**

```bash
git add app/src/app_state.rs app/src/pane_group/
git commit -m "feat: 实现 SftpPane 和 Pane 系统集成"
```

---

### Task 4: 接入 SSH 管理器右键菜单和 Workspace

**Files:**
- Modify: `app/src/ssh_manager/panel.rs` — 添加"SFTP 浏览"菜单项
- Modify: `app/src/workspace/view.rs` — 添加 open_sftp_pane 方法

- [ ] **Step 1: 在 ssh_manager/panel.rs 添加事件和菜单项**

在 `SshManagerPanelEvent` 枚举中添加：

```rust
OpenSftpPane { node_id: String, server: SshServerInfo },
```

在服务器右键菜单项列表中，找到 `"Connect"` 菜单项，在其后面添加 "SFTP 浏览" 菜单项，对应 dispatch `SshManagerPanelEvent::OpenSftpPane { node_id, server }`。

具体位置：找到构建服务器右键菜单项的代码，在 "连接" 菜单项后添加一个新菜单项 "SFTP 浏览"。

- [ ] **Step 2: 在 workspace/view.rs 添加处理**

在 `LeftPanelEvent` 的处理 match 中，找到 `OpenSshTerminal` 的处理分支，在其附近添加：

```rust
LeftPanelEvent::OpenSftpPane { node_id, server: _ } => {
    self.open_sftp_pane(node_id.clone(), ctx);
}
```

添加 `open_sftp_pane` 方法：

```rust
pub fn open_sftp_pane(&mut self, node_id: String, ctx: &mut ViewContext<Self>) {
    use crate::pane_group::pane::sftp_pane::SftpPane;
    self.active_tab_pane_group().update(ctx, |pane_group, ctx| {
        let pane = SftpPane::new(node_id, ctx);
        let smart_split_direction = pane_group.smart_split_direction(ctx, WORKFLOW_AND_ENV_VAR_SPLIT_RATIO);
        pane_group.add_pane_with_direction(smart_split_direction, pane, true, ctx);
    });
}
```

注意：需要确认 `LeftPanelEvent` 中是否已有 `OpenSftpPane` 变体，如果没有，需要在定义处添加。

- [ ] **Step 3: 验证编译**

Run: `cargo check -p warp`
Expected: 编译成功

- [ ] **Step 4: 提交**

```bash
git add app/src/ssh_manager/panel.rs app/src/workspace/view.rs
git commit -m "feat: 接入 SSH 管理器右键菜单，支持打开 SFTP 浏览面板"
```

---

### Task 5: 完善 SftpBrowserView 主视图

**Files:**
- Modify: `app/src/sftp_manager/browser.rs` — 替换占位为完整实现

- [ ] **Step 1: 用完整实现替换 browser.rs**

将 Task 2 中创建的占位 `browser.rs` 替换为完整实现。完整代码参考 openwarp 的 `app/src/sftp_manager/browser.rs`（约 1097 行），关键修改点：

1. `use warp_sftp::session_bridge::` → `use warp_sftp::session::`
2. `use crate::sftp_manager::sftp_ops` 保持不变
3. 所有 `warp_core::ui::appearance::Appearance` 确认 zap-2 中使用相同的 import 路径
4. 确认 `Icon` 枚举中的变体名在 zap-2 的 `ui_components::icons::Icon` 中存在

完整实现包括：
- `SftpBrowserAction` 枚举（所有 Action）
- `SftpBrowserView` 结构体（连接状态、导航、传输、对话框等所有字段）
- `new()` 构造函数（初始化所有字段 + 订阅编辑器事件 + 自动连接）
- `connect_to_server()` 方法
- `refresh_dir()` / `navigate_to()` / `go_up()` / `go_back()` / `go_forward()` 方法
- `open_entry()` / `delete_selected()` / `confirm_delete()` / `download_entry()` / `show_details()` / `rename_entry()` 方法
- `render_toolbar_btn()` / `render_toolbar()` / `render_breadcrumb()` / `render_connection_state()` / `render_error()` 方法
- `TypedActionView` impl（handle_action，处理所有 Action）
- `View` impl（render，构建完整 UI 布局）
- `BackingView` impl（所有 trait 方法）
- 辅助函数 `build_rename_path` / `build_new_folder_path` / `build_upload_remote_path`

- [ ] **Step 2: 验证编译**

Run: `cargo check -p warp`
Expected: 编译成功

- [ ] **Step 3: 提交**

```bash
git add app/src/sftp_manager/browser.rs
git commit -m "feat: 实现 SftpBrowserView 完整浏览器视图"
```

---

### Task 6: 完善 UI 子模块

**Files:**
- Modify: `app/src/sftp_manager/file_list.rs` — 替换占位为完整实现
- Modify: `app/src/sftp_manager/breadcrumb.rs` — 替换占位为完整实现
- Modify: `app/src/sftp_manager/context_menu.rs` — 替换占位为完整实现
- Modify: `app/src/sftp_manager/dialogs.rs` — 替换占位为完整实现
- Modify: `app/src/sftp_manager/transfer_panel.rs` — 替换占位为完整实现

- [ ] **Step 1: 替换 file_list.rs 为完整实现**

替换为 openwarp 的完整 `file_list.rs` 代码（约 224 行）。包含：
- `file_icon()` 函数
- `render_file_row()` 函数（单行渲染，含图标/名称/大小/日期，点击/双击事件）
- `render_header()` 公有函数（表头：名称/大小/修改时间）
- `render_file_rows()` 公有函数（文件行列表）

- [ ] **Step 2: 替换 breadcrumb.rs 为完整实现**

替换为 openwarp 的完整 `breadcrumb.rs` 代码（约 110 行）。包含：
- `render_breadcrumb()` 公有函数（可点击路径分段 + 分隔符图标）

- [ ] **Step 3: 替换 context_menu.rs 为完整实现**

替换为 openwarp 的完整 `context_menu.rs` 代码（约 139 行）。包含：
- `ContextMenuState` 结构体（保持不变，已是完整版）
- `MenuItem` 结构体
- `build_file_menu_items()` 函数
- `render_menu_item()` 函数
- `render_context_menu()` 公有函数

- [ ] **Step 4: 替换 dialogs.rs 为完整实现**

替换为 openwarp 的完整 `dialogs.rs` 代码（约 382 行）。包含：
- `dialog_shell()` 函数
- `render_button()` 函数
- `render_delete_confirm()` / `render_rename()` / `render_create_folder()` / `render_file_details()` 函数
- `render_dialog()` 公有函数

- [ ] **Step 5: 替换 transfer_panel.rs 为完整实现**

替换为 openwarp 的完整 `transfer_panel.rs` 代码（约 200 行）。包含：
- `render_direction_icon()` / `render_state_label()` / `render_progress_bar()` 函数
- `render_transfer_row()` 函数
- `render_transfer_panel()` 公有函数

- [ ] **Step 6: 验证编译**

Run: `cargo check -p warp`
Expected: 编译成功

- [ ] **Step 7: 提交**

```bash
git add app/src/sftp_manager/
git commit -m "feat: 实现 SFTP 浏览器全部 UI 子模块"
```

---

### Task 7: 处理编译错误和遗漏的集成点

**Files:**
- Various（根据编译结果修改）

- [ ] **Step 1: 全量编译检查**

Run: `cargo check -p warp 2>&1 | head -100`
Expected: 可能有一些 import 路径差异或缺失的 match arm

- [ ] **Step 2: 逐个修复编译错误**

常见需要修复的问题：
1. `LeftPanelEvent` 枚举中缺少 `OpenSftpPane` 变体 → 添加
2. `persistence/sqlite.rs` 中 `LeafContents` 的 match 缺少 `Sftp` 分支 → 添加（不持久化）
3. `Icon` 枚举中某些变体名不同 → 替换为 zap-2 中对应的名称
4. import 路径差异（`warp_core::ui::` vs 其他路径）→ 修正

- [ ] **Step 3: 再次全量编译**

Run: `cargo check -p warp`
Expected: 编译成功，仅有 unused warnings

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -m "fix: 修复 SFTP 集成编译错误"
```

---

### Task 8: 验证完整构建

- [ ] **Step 1: 完整 release 构建**

Run: `cargo build -p warp --release`
Expected: 构建成功

- [ ] **Step 2: 最终提交**

```bash
git add -A
git commit -m "feat: SFTP 文件管理器功能完成"
```
