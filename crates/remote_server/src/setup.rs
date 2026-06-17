mod glibc;

pub use glibc::{GlibcVersion, RemoteLibc};

use std::time::Duration;

use anyhow::{anyhow, Result};
use warp_core::channel::{Channel, ChannelState};

/// State machine for the remote server install → launch → initialize flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteServerSetupState {
    /// Checking if the binary exists on remote.
    Checking,
    /// Downloading and installing the binary for the first time on this host.
    Installing { progress_percent: Option<u8> },
    /// Replacing an existing install with a differently-versioned binary.
    /// Rendered as "Updating..." in the UI so the user understands this
    /// isn't a fresh install.
    Updating,
    /// Binary is launched, waiting for InitializeResponse.
    Initializing,
    /// Handshake complete. Ready.
    Ready,
    /// Something failed. Fall back to ControlMaster.
    Failed { error: String },
    /// Preinstall check classified the host as incompatible with the
    /// prebuilt remote-server binary. The controller treats this as a
    /// clean fall-back to the legacy ControlMaster-backed SSH flow,
    /// distinct from `Failed` (which is rendered as a real error).
    Unsupported { reason: UnsupportedReason },
}

impl RemoteServerSetupState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }

    pub fn is_terminal(&self) -> bool {
        self.is_ready() || self.is_failed() || self.is_unsupported()
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }
}

/// Outcome of [`crate::transport::RemoteTransport::run_preinstall_check`].
///
/// The script runs over the existing SSH socket before any install UI
/// surfaces and reports whether the host can run the prebuilt
/// remote-server binary. The Rust side is intentionally a thin parser
/// over the script's structured stdout (see `preinstall_check.sh`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreinstallCheckResult {
    pub status: PreinstallStatus,
    pub libc: RemoteLibc,
    /// Verbatim, trimmed script stdout. Forwarded to telemetry for
    /// diagnosing `Unknown` outcomes on exotic distros.
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreinstallStatus {
    Supported,
    Unsupported {
        reason: UnsupportedReason,
    },
    /// Probe ran but couldn't classify the host. Treated as supported
    /// (fail open) by [`PreinstallCheckResult::is_supported`] so we keep
    /// today's install-and-try behavior on hosts where the probe is
    /// unreliable.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedReason {
    GlibcTooOld {
        detected: GlibcVersion,
        required: GlibcVersion,
    },
    NonGlibc {
        name: String,
    },
}

impl PreinstallCheckResult {
    /// Whether the host is supported. Both `Supported` and `Unknown`
    /// return true — only positive detection of an incompatible libc
    /// triggers the silent fall-back.
    pub fn is_supported(&self) -> bool {
        match self.status {
            PreinstallStatus::Supported | PreinstallStatus::Unknown => true,
            PreinstallStatus::Unsupported { .. } => false,
        }
    }

    /// Parses the structured `key=value` stdout emitted by
    /// `preinstall_check.sh`. Tolerates unknown keys and lines without
    /// `=` (forward-compatibility): future versions of the script can
    /// add new keys without coordinating a client release.
    pub fn parse(stdout: &str) -> Self {
        let mut status_str: Option<&str> = None;
        let mut reason_str: Option<&str> = None;
        let mut libc_family: Option<&str> = None;
        let mut libc_version: Option<&str> = None;
        let mut required_glibc: Option<&str> = None;

        for line in stdout.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "status" => status_str = Some(value.trim()),
                "reason" => reason_str = Some(value.trim()),
                "libc_family" => libc_family = Some(value.trim()),
                "libc_version" => libc_version = Some(value.trim()),
                "required_glibc" => required_glibc = Some(value.trim()),
                _ => {} // ignore unknown keys
            }
        }

        let libc = glibc::parse_libc(libc_family, libc_version);
        let status = parse_status(status_str, reason_str, &libc, required_glibc);

        Self {
            status,
            libc,
            raw: stdout.trim().to_string(),
        }
    }
}

fn parse_status(
    status: Option<&str>,
    reason: Option<&str>,
    _libc: &RemoteLibc,
    _required_glibc: Option<&str>,
) -> PreinstallStatus {
    // remote-server 现在是静态 musl 二进制(见 `preinstall_check.sh` 顶部
    // 注释),不链接宿主的动态 libc。因此 `glibc_too_old` / `non_glibc`
    // 已不再是「不支持」的理由 —— 任意 glibc 版本与 musl/uclibc 宿主都能
    // 运行该二进制。新版脚本不会再发出这两个 reason;但旧版 remote 端可能
    // 仍缓存着老脚本,所以这里把这些 libc 门禁理由一并当作 `Supported`,
    // 而不是 `Unsupported`,保持新旧脚本的判定一致。
    match status {
        Some("supported") => PreinstallStatus::Supported,
        Some("unsupported") => match reason {
            // 旧脚本残留的 libc 门禁理由:静态二进制下已失效,视为支持。
            Some("glibc_too_old") | Some("non_glibc") => PreinstallStatus::Supported,
            // 其他无法识别的 unsupported 理由:保守起见 fail open。
            _ => PreinstallStatus::Unknown,
        },
        // status=unknown, missing, or anything else → fail open.
        _ => PreinstallStatus::Unknown,
    }
}

/// The bundled preinstall check script. Loaded as a string so the SSH
/// transport can pipe it through the existing ControlMaster socket via
/// [`crate::ssh::run_ssh_script`].
///
/// The script is intentionally self-contained — the supported-glibc
/// floor is hardcoded inside the script (see `preinstall_check.sh`)
/// rather than templated from Rust.
pub const PREINSTALL_CHECK_SCRIPT: &str = include_str!("preinstall_check.sh");

/// Detected remote platform from `uname -sm` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePlatform {
    pub os: RemoteOs,
    pub arch: RemoteArch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOs {
    Linux,
    MacOs,
}

impl RemoteOs {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteArch {
    X86_64,
    Aarch64,
}

impl RemoteArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

/// Parse `uname -sm` output into a `RemotePlatform`.
///
/// The expected format is `<os> <arch>`, e.g. `Linux x86_64` or `Darwin arm64`.
/// Takes the last line to skip any shell initialization output.
pub fn parse_uname_output(output: &str) -> Result<RemotePlatform> {
    let line = output
        .lines()
        .last()
        .ok_or_else(|| anyhow!("empty uname output"))?
        .trim();

    let mut parts = line.split_whitespace();
    let os_str = parts
        .next()
        .ok_or_else(|| anyhow!("missing OS in uname output: {line}"))?;
    let arch_str = parts
        .next()
        .ok_or_else(|| anyhow!("missing arch in uname output: {line}"))?;

    let os = match os_str {
        "Linux" => RemoteOs::Linux,
        "Darwin" => RemoteOs::MacOs,
        other => return Err(anyhow!("unsupported OS: {other}")),
    };

    let arch = match arch_str {
        "x86_64" => RemoteArch::X86_64,
        "aarch64" | "arm64" | "armv8l" => RemoteArch::Aarch64,
        other => return Err(anyhow!("unsupported arch: {other}")),
    };

    Ok(RemotePlatform { os, arch })
}

/// 返回远端二进制安装目录,按 channel 隔离。
///
/// - stable:      `~/.warp/remote-server`
/// - preview:     `~/.warp-preview/remote-server`
/// - dev:         `~/.warp-dev/remote-server`
/// - local:       `~/.warp-local/remote-server`
/// - integration: `~/.warp-dev/remote-server`
/// - warp-oss:    `~/.zap/remote-server`
pub fn remote_server_dir() -> String {
    let warp_dir = match ChannelState::channel() {
        Channel::Stable => ".warp",
        Channel::Preview => ".warp-preview",
        Channel::Dev | Channel::Integration => ".warp-dev",
        Channel::Local => ".warp-local",
        Channel::Oss => ".zap",
    };
    format!("~/{warp_dir}/remote-server")
}

/// 返回可安全放入路径的 remote-server identity key 目录名。
///
/// identity key 不是密钥,但可能包含路径中不安全或有歧义的字节。
/// 保留 ASCII 字母数字以及 `-` / `_`,其他 UTF-8 字节做百分号编码。
pub fn remote_server_identity_dir_name(identity_key: &str) -> String {
    if identity_key.is_empty() {
        return "empty".to_string();
    }

    let mut encoded = String::with_capacity(identity_key.len());
    for byte in identity_key.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// 返回按 identity 隔离的远端目录,用于 daemon socket 和 PID 文件。
pub fn remote_server_daemon_dir(identity_key: &str) -> String {
    format!(
        "{}/{}",
        remote_server_dir(),
        remote_server_identity_dir_name(identity_key)
    )
}

/// 返回远端 remote-server 二进制文件名。
pub fn binary_name() -> &'static str {
    ChannelState::channel().cli_command_name()
}

/// 返回当前 channel 和客户端版本对应的远端二进制完整路径。
///
/// Local 构建保留无版本后缀路径,以便 `script/deploy_remote_server`
/// 覆盖同一个开发 slot。Zap release 构建带 `GIT_RELEASE_TAG`
/// 时使用版本后缀,这样新版本会自然触发重新安装;源码本地构建没有
/// release tag,仍使用无后缀路径。
pub fn remote_server_binary() -> String {
    let dir = remote_server_dir();
    let name = binary_name();
    match ChannelState::channel() {
        Channel::Local => format!("{dir}/{name}"),
        Channel::Oss if ChannelState::app_version().is_none() => format!("{dir}/{name}"),
        Channel::Oss => format!("{dir}/{name}-{}", pinned_version()),
        Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("{dir}/{name}-{}", pinned_version())
        }
    }
}

/// 返回检查远端 remote-server 二进制存在且可执行的 shell 命令。
///
/// 与上游一致,这里实际运行 `--version`,而不只是 `test -x`;
/// 这样可以把损坏或无法解析参数的二进制提前识别出来。
pub fn binary_check_command() -> String {
    format!("{} --version", remote_server_binary())
}

/// 返回用于版本化安装路径的版本号。优先使用编译时注入的
/// `GIT_RELEASE_TAG`;没有 release tag 时回退到 `CARGO_PKG_VERSION`,
/// 让需要版本化路径的 channel 保持确定性,并在缺少对应 release 资产时
/// 清晰失败,而不是误用无版本路径。
fn pinned_version() -> &'static str {
    ChannelState::app_version().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// 安装脚本模板独立放在 `.sh` 文件里方便维护。
/// `{download_base_url}` 等占位符由 [`install_script`] 替换。
const INSTALL_SCRIPT_TEMPLATE: &str = include_str!("install_remote_server.sh");

/// 返回安装脚本。`staging_tarball_path` 非空时,脚本跳过远端下载,
/// 改为解压客户端通过 SCP 预上传的 tarball。
pub fn install_script(staging_tarball_path: Option<&str>) -> String {
    let version_suffix = version_suffix();
    INSTALL_SCRIPT_TEMPLATE
        .replace("{download_base_url}", &download_url())
        .replace("{install_dir}", &remote_server_dir())
        .replace("{binary_name}", binary_name())
        .replace("{version_suffix}", &version_suffix)
        .replace("{staging_tarball_path}", staging_tarball_path.unwrap_or(""))
}

/// 构造 Zap CLI release 资产下载基址。
fn download_url() -> String {
    let release_path = match ChannelState::app_version() {
        Some(tag) => format!("download/{tag}"),
        None => "latest/download".to_string(),
    };
    format!("https://github.com/zerx-lab/warp/releases/{release_path}")
}

fn version_suffix() -> String {
    match ChannelState::channel() {
        Channel::Local => String::new(),
        Channel::Oss if ChannelState::app_version().is_none() => String::new(),
        Channel::Oss | Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("-{}", pinned_version())
        }
    }
}

/// 返回指定远端平台对应的 Zap CLI tarball URL。
pub fn download_tarball_url(platform: &RemotePlatform) -> String {
    format!(
        "{}/zap-{}-{}.tar.gz",
        download_url(),
        platform.os.as_str(),
        platform.arch.as_str(),
    )
}

/// Zap fork:开发模式(DEBUG 源码构建,无 release tag)下,
/// SSH transport 不再从 GitHub 下载陈旧的发行版,而是本地交叉编译
/// 当前 `warp` 二进制并上传。下面这些常量集中描述该交叉编译产物,
/// 与 `script/deploy_remote_server` 保持一致(同 profile / 同 features /
/// 同 target),避免两处分叉。
///
/// 交叉编译目标三元组。
pub const DEV_MUSL_TARGET: &str = "x86_64-unknown-linux-musl";

/// 交叉编译使用的 cargo profile。对应 `Cargo.toml` 的 `[profile.dev-remote]`,
/// 它继承 `dev` 并 strip 符号以减小体积、加快上传。
pub const DEV_REMOTE_PROFILE: &str = "dev-remote";

/// 交叉编译启用的 features,与 `script/deploy_remote_server` 一致。
pub const DEV_REMOTE_FEATURES: &str = "release_bundle,crash_reporting,standalone,agent_mode_debug";

/// 判断当前是否处于「开发模式 remote-server 安装」路径。
///
/// 默认条件:DEBUG 构建(`debug_assertions`)且没有注入 `GIT_RELEASE_TAG`
/// (`app_version().is_none()`,即源码本地构建,非发行版)。这与
/// `remote_server_binary()` / `download_url()` 中对「无 release tag」的
/// 判定保持同一标准。release 构建恒为 `false`,行为完全不变。
///
/// 显式覆盖:设置 `WARP_REMOTE_SERVER_FROM_LOCAL=1` 强制走本地交叉编译路径
/// (`0`/未设视为关闭)。用于 release 构建里临时联调本地 remote-server。
pub fn is_dev_source_build() -> bool {
    if let Some(raw) = std::env::var_os("WARP_REMOTE_SERVER_FROM_LOCAL") {
        let lossy = raw.to_string_lossy();
        let trimmed = lossy.trim();
        let disabled =
            trimmed.is_empty() || trimmed == "0" || trimmed.eq_ignore_ascii_case("false");
        if !disabled {
            return true;
        }
    }
    cfg!(debug_assertions) && ChannelState::app_version().is_none()
}

/// 检查二进制是否存在的超时。
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// 常规远端安装脚本超时。
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

/// SCP fallback 包含本地下载、上传和远端解压,给它更宽松的超时。
pub const SCP_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// 开发模式交叉编译可能要从头编译整个 crate 图,给它一个很宽松的超时。
pub const DEV_CROSS_COMPILE_TIMEOUT: Duration = Duration::from_secs(900);

/// 开发模式上传本地交叉编译产物的超时。dev 二进制(未优化 + 调试信息)有
/// 数百 MB,即便 scp 开了 `-C` 压缩,跨公网上传也可能要数分钟,因此给一个
/// 远超 `SCP_INSTALL_TIMEOUT` 的宽松上限。
pub const DEV_UPLOAD_TIMEOUT: Duration = Duration::from_secs(1800);

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
