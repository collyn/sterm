//! 通知中心(mailbox + toast)。
//!
//! 由 002ce467 cloud-removal 误删后重建,只保留与云端无关的本地路径:
//! - 软件本体的 BYOP agent (Oz) 完成/出错通知
//! - 第三方 CLI agent (Claude Code / Codex / DeepSeek 等) 状态通知
//!
//! 模块布局:
//! - `item`         数据模型 (`NotificationItem` / `NotificationItems` 等)
//! - `item_rendering` 单条通知 UI (mailbox 和 toast 共用)
//! - `model`        单例 `NotificationsModel`(订阅 history / cli 会话 model,产出通知)
//! - `view`         `NotificationMailboxView`(信箱主面板)
//! - `toast_stack`  `AgentNotificationToastStack`(右下角 toast)
//! - `telemetry`    通知中心相关的 telemetry event(`NotificationsTelemetryEvent`)

pub(crate) mod item;
pub(crate) mod item_rendering;
pub mod model;
pub(crate) mod telemetry;
pub mod toast_stack;
pub mod view;

pub(crate) use item::{
    NotificationCategory, NotificationFilter, NotificationId, NotificationItem, NotificationItems,
    NotificationSourceAgent,
};
pub use toast_stack::AgentNotificationToastStack;
pub use view::{NotificationMailboxView, NotificationMailboxViewEvent};

pub fn init(app: &mut warpui::AppContext) {
    NotificationMailboxView::init(app);
}
