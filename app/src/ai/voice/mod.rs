//! This module contains all code relevant to Voice within Zap.
//!
//! Voice is used for voice input within Zap.

// Zap Wave 6-1:`pub(crate) mod transcribe` 随 `ServerApi::transcribe` 一同物理删。
// 原子模块 `transcribe/api/{request,response}` 仅为已删除的云端 `/ai/transcribe` 端点
// 的 wire 类型。本地语音走 `voice/transcriber.rs::Transcriber` trait + `TranscribeError`。
