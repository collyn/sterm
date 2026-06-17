//! This module contains all code relevant to Agent Predict within Zap.
//!
//! Agent Predict attempts to predict the next action the user will take in Zap.

pub(crate) mod generate_ai_input_suggestions;
pub(crate) mod generate_am_query_suggestions;
pub mod next_command_model;
// Zap(Wave 3-2):`predict_am_queries` API 模块已物理删 — 原 `ServerApi::predict_am_queries`
// 0 外部消费已同步删除；FeatureFlag::PredictAMQueries / terminal/input.rs 中
// `predict_am_queries_future_handle` 仅作为控制开关/句柄代号保留，不再需要该模块。
pub mod prompt_suggestions;
