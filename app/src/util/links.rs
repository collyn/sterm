use crate::channel::ChannelState;

// 上游 Warp 的文档站/Slack/隐私政策对 Zap fork 不再适用,
// 这些常量保留为占位空串,等 Zap 自有渠道落地后再填。
// `ctx.open_url("")` 在 UI 调用方是无害 no-op。
pub const USER_DOCS_URL: &str = "";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GITHUB_ISSUES_URL: &str = "https://github.com/collyn/sterm/issues";
pub const SLACK_URL: &str = "";
pub const PRIVACY_POLICY_URL: &str = "";

pub fn feedback_form_url() -> String {
    let mut url = url::Url::parse("https://github.com/collyn/sterm/issues/new/choose")
        .expect("Should not fail to parse");
    if let Some(version) = ChannelState::app_version() {
        url.query_pairs_mut().append_pair("sterm-version", version);
    }
    url.query_pairs_mut()
        .append_pair("os-version", &os_info::get().version().to_string());
    url.to_string()
}
