use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use instant::Instant;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::color::internal_colors;
use warpui::assets::asset_cache::AssetSource;
use warpui::fonts::FamilyId;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
    Flex, Hoverable, Image, MainAxisAlignment, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Point, Radius, Shrinkable, Stack, Text,
};
use warpui::event::DispatchedEvent;
use warpui::image_cache::CacheOption;
use warpui::platform::Cursor;
use warpui::r#async::Timer;
use warpui::text_layout::ClipConfig;
use warpui::{
    AfterLayoutContext, AppContext, Entity, EntityId, EventContext, LayoutContext, PaintContext,
    SingletonEntity, SizeConstraint, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::terminal::cli_agent_sessions::listener::agent_supports_rich_status;
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionContext, CLIAgentSessionStatus, CLIAgentSessionsModel,
    CLIAgentSessionsModelEvent,
};
use crate::terminal::CLIAgent;
use crate::ui_components::icon_with_status::{
    render_icon_with_status, IconWithStatusSizing, IconWithStatusVariant,
};
use crate::workspace::WorkspaceAction;

const PANEL_PADDING: f32 = 10.;

/// State of a sub-agent character in the scene.
#[derive(Clone, Copy, PartialEq)]
enum SubAgentState {
    /// Walking from entrance to assigned desk.
    WalkingIn(usize),
    /// Sitting at desk, working.
    Working,
    /// Walking from desk toward boss desk after completion.
    WalkingOut(usize),
    /// Standing at boss area, reporting. Timer counts up to REPORT_FRAMES then auto-removed.
    Reporting(usize),
}

/// Frames to walk from entrance (bottom) to desk.
const WALK_IN_FRAMES: usize = 12;
/// Frames to walk from desk to boss area.
const WALK_OUT_FRAMES: usize = 10;
/// Frames to linger at boss desk before removal.
const REPORT_FRAMES: usize = 3;

/// Entrance position (bottom center of scene).
const ENTRANCE_X: f32 = 100.;
const ENTRANCE_Y: f32 = SCENE_H - CHAR_FRAME_H;

/// Aisle X positions — corridors that avoid all desk/chair furniture.
const LEFT_AISLE_X: f32 = 62.;
const RIGHT_AISLE_X: f32 = 148.;

/// Direction a character is facing (determines walking sprite).
#[derive(Clone, Copy, PartialEq)]
enum FacingDirection {
    Left,
    Right,
    /// Walking away from camera (y decreasing = moving up in scene).
    Back,
}

/// Tracks a sub-agent's visual representation in the pixel art scene.
#[derive(Clone)]
struct SubAgentCharacter {
    /// Which character sprite to use (0-5).
    char_index: usize,
    /// Current animation state.
    state: SubAgentState,
    /// Which desk row (0-3) the agent was assigned to.
    desk_row: usize,
    /// Which side of the desk (true = left).
    desk_side: bool,
    /// Display name for the label.
    name: String,
    /// Current interpolated position in scene space.
    current_x: f32,
    current_y: f32,
    /// Which direction the character is facing (for walking sprite selection).
    facing: FacingDirection,
    /// Task label to show in bubble (tool_name or summary).
    task_label: Option<String>,
}

impl SubAgentCharacter {
    /// Target desk position based on desk_row and desk_side.
    fn desk_position(&self) -> (f32, f32) {
        let desk_x = 90.;
        let desk_y = 78.;
        let row_gap = 46.;
        let x = if self.desk_side {
            desk_x - 13.
        } else {
            desk_x + 29.
        };
        let y = desk_y + self.desk_row as f32 * row_gap + 6.;
        (x, y)
    }

    /// Report position near the boss, side-dependent to avoid crossing furniture.
    fn report_position(&self) -> (f32, f32) {
        if self.desk_side {
            (78., 64.) // Left of boss desk
        } else {
            (140., 64.) // Right of boss desk
        }
    }

    /// Waypoints for walking from entrance to desk (avoids furniture).
    fn walk_in_waypoints(&self) -> Vec<(f32, f32)> {
        let (dx, dy) = self.desk_position();
        let aisle = if self.desk_side { LEFT_AISLE_X } else { RIGHT_AISLE_X };
        vec![
            (ENTRANCE_X, ENTRANCE_Y),
            (aisle, ENTRANCE_Y),
            (aisle, dy),
            (dx, dy),
        ]
    }

    /// Waypoints for walking from desk to boss report position (avoids furniture).
    fn walk_out_waypoints(&self) -> Vec<(f32, f32)> {
        let (dx, dy) = self.desk_position();
        let aisle = if self.desk_side { LEFT_AISLE_X } else { RIGHT_AISLE_X };
        let (rx, ry) = self.report_position();
        vec![
            (dx, dy),
            (aisle, dy),
            (aisle, ry),
            (rx, ry),
        ]
    }

    /// Interpolate position along a multi-waypoint path.
    /// Distributes progress proportionally to segment length for constant speed.
    fn lerp_along_path(waypoints: &[(f32, f32)], frame: usize, total_frames: usize) -> (f32, f32) {
        if waypoints.is_empty() {
            return (0., 0.);
        }
        if frame >= total_frames || waypoints.len() == 1 {
            return *waypoints.last().unwrap();
        }

        // Calculate total path length and per-segment lengths.
        let mut seg_lengths = Vec::with_capacity(waypoints.len() - 1);
        let mut total_len = 0.0f32;
        for i in 0..waypoints.len() - 1 {
            let dx = waypoints[i + 1].0 - waypoints[i].0;
            let dy = waypoints[i + 1].1 - waypoints[i].1;
            let len = (dx * dx + dy * dy).sqrt();
            seg_lengths.push(len);
            total_len += len;
        }
        if total_len == 0. {
            return *waypoints.last().unwrap();
        }

        let t = frame as f32 / total_frames as f32;
        let target_dist = t * total_len;

        let mut accumulated = 0.0f32;
        for (i, &seg_len) in seg_lengths.iter().enumerate() {
            if seg_len > 0. && accumulated + seg_len >= target_dist {
                let seg_t = (target_dist - accumulated) / seg_len;
                let (x0, y0) = waypoints[i];
                let (x1, y1) = waypoints[i + 1];
                return (x0 + (x1 - x0) * seg_t, y0 + (y1 - y0) * seg_t);
            }
            accumulated += seg_len;
        }

        *waypoints.last().unwrap()
    }
}
const AGENT_ROW_PADDING: f32 = 10.;
const AGENT_ROW_SPACING: f32 = 10.;
const AGENT_ICON_SIZE: f32 = 20.;
const AGENT_ICON_PADDING: f32 = 6.;
const AGENT_ROW_RADIUS: f32 = 8.;
const TITLE_FONT_SIZE: f32 = 13.;
const DETAIL_FONT_SIZE: f32 = 12.;
const EMPTY_FONT_SIZE: f32 = 13.;

#[derive(Clone)]
struct PixelAgentSnapshot {
    terminal_view_id: EntityId,
    agent: CLIAgent,
    status: CLIAgentSessionStatus,
    session_context: CLIAgentSessionContext,
}

/// Incrementally tracks the active subagent count from a Claude transcript JSONL file.
#[allow(dead_code)]
struct TranscriptCache {
    path: String,
    offset: u64,
    active_tool_ids: HashSet<String>,
    active_agent_ids: HashSet<String>,
    /// Peak number of concurrent subagents seen so far.
    peak_active: usize,
    last_poll: Instant,
    /// Whether the session is idle (Claude Code is at its prompt, not processing).
    /// Detected by the presence of a "last-prompt" or "turn_duration" line in the
    /// transcript, or by inactivity timeout when no active tools remain.
    session_idle: bool,
    /// Timestamp of last JSONL data received. Used for inactivity-based idle detection.
    last_data_at: Instant,
}

const TRANSCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// If no new transcript data arrives for this duration and there are no active
/// tool_use IDs, consider the session idle. Safety net for when neither
/// `last-prompt` nor `turn_duration` records appear (text-only turns, older
/// Claude Code versions, crashes).
const INACTIVITY_IDLE_THRESHOLD: Duration = Duration::from_secs(5);

impl TranscriptCache {
    fn new(path: String) -> Self {
        let mut cache = Self {
            path,
            offset: 0,
            active_tool_ids: HashSet::new(),
            active_agent_ids: HashSet::new(),
            peak_active: 0,
            last_poll: Instant::now() - TRANSCRIPT_POLL_INTERVAL,
            session_idle: false,
            last_data_at: Instant::now(),
        };
        // Initial full read
        cache.read_incremental();
        cache
    }

    fn active_count(&self) -> usize {
        self.active_tool_ids.len() + self.active_agent_ids.len()
    }

    /// Reads new lines from the transcript file and updates active tool tracking.
    fn read_incremental(&mut self) {
        let Ok(mut file) = File::open(&self.path) else {
            return;
        };
        // Handle file truncation (log rotation)
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if file_len < self.offset {
            self.offset = 0;
            self.active_tool_ids.clear();
            self.active_agent_ids.clear();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let reader = BufReader::new(file);
        let mut new_offset = self.offset;
        for line in reader.lines().map_while(Result::ok) {
            new_offset += line.len() as u64 + 1; // +1 for newline
            self.last_data_at = Instant::now();
            process_jsonl_line(&line, &mut self.active_tool_ids, &mut self.active_agent_ids);
            // Track peak concurrent subagents
            let current = self.active_tool_ids.len() + self.active_agent_ids.len();
            if current > self.peak_active {
                self.peak_active = current;
            }
            // Detect session idle/active state from transcript record type.
            // Use JSON parsing instead of string matching so format variations
            // (whitespace, field order) don't cause missed detections.
            classify_transcript_line(&line, &mut self.session_idle, &mut self.active_tool_ids, &mut self.active_agent_ids);
        }
        self.offset = new_offset;
    }

    fn poll(&mut self) {
        if self.last_poll.elapsed() >= TRANSCRIPT_POLL_INTERVAL {
            self.read_incremental();
            self.last_poll = Instant::now();
        }
    }
}

/// Checks if a task output file (.output) represents an active sub-agent.
/// Parses the last line to see if the assistant has already finished (stop_reason is end_turn/stop_sequence).
fn is_output_file_active(path: &std::path::Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file_len == 0 {
        return true; // Newly created, active
    }
    // Seek to near end to read the last non-empty line
    let seek_pos = file_len.saturating_sub(2048);
    if file.seek(SeekFrom::Start(seek_pos)).is_err() {
        return true;
    }
    let reader = BufReader::new(file);
    let mut last_line = String::new();
    for line in reader.lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            last_line = line;
        }
    }
    if last_line.is_empty() {
        return true;
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&last_line) {
        let is_assistant = val.get("type").and_then(|t| t.as_str()) == Some("assistant");
        let stop_reason = val.get("stop_reason").and_then(|s| s.as_str())
            .or_else(|| val.get("message").and_then(|m| m.get("stop_reason")).and_then(|s| s.as_str()));
        let has_ended = matches!(stop_reason, Some("end_turn") | Some("stop_sequence"));
        if is_assistant && has_ended {
            return false; // Ended successfully
        }
    }
    true
}

/// Tracks sub-agent lifecycle from Claude Code transcript files.
/// Uses session_id from the session context to find the exact transcript,
/// falling back to scanning task output files when session_id is unavailable.
struct GlobalTranscriptScanner {
    last_scan: Instant,
    caches: HashMap<String, TranscriptCache>,
    /// Paths fed this cycle (for stale cache cleanup).
    fed_paths: HashSet<String>,
    cached_active: usize,
    cached_peak: usize,
    /// Whether any tracked session is idle (Claude Code at prompt).
    any_session_idle: bool,
}

const GLOBAL_SCAN_INTERVAL: Duration = Duration::from_millis(500);
const TASK_FALLBACK_WINDOW: Duration = Duration::from_secs(120);

impl GlobalTranscriptScanner {
    fn new() -> Self {
        Self {
            last_scan: Instant::now() - GLOBAL_SCAN_INTERVAL,
            caches: HashMap::new(),
            fed_paths: HashSet::new(),
            cached_active: 0,
            cached_peak: 0,
            any_session_idle: false,
        }
    }

    fn counts(&mut self) -> (usize, usize) {
        if self.last_scan.elapsed() >= GLOBAL_SCAN_INTERVAL {
            self.scan();
            self.last_scan = Instant::now();
        }
        (self.cached_peak.min(6), self.cached_active.min(6))
    }

    /// Register a session for transcript matching. Call before `counts()`.
    fn feed_session(&mut self, ctx: &CLIAgentSessionContext) {
        if let Some(path) = self.find_transcript(ctx) {
            self.fed_paths.insert(path.clone());
            let cache = self.caches
                .entry(path.clone())
                .or_insert_with(|| TranscriptCache::new(path.clone()));
            if cache.path != path {
                *cache = TranscriptCache::new(path.clone());
            }
            cache.poll();
        }
    }

    fn find_transcript(&self, session: &CLIAgentSessionContext) -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let projects_dir = std::path::Path::new(&home)
            .join(".claude").join("projects");

        // Exact session_id + cwd match
        if let Some(sid) = session.session_id.as_deref() {
            if let Some(cwd) = session.cwd.as_deref() {
                let cwd = cwd.trim_start_matches('/');
                if !cwd.is_empty() {
                    let project_dir = format!("-{}", cwd.replace('/', "-"));
                    let exact = projects_dir.join(&project_dir)
                        .join(sid).with_extension("jsonl");
                    if exact.exists() {
                        return Some(exact.to_string_lossy().into_owned());
                    }
                }
            }
            // Global search for session_id match
            if let Ok(project_dirs) = std::fs::read_dir(&projects_dir) {
                for pe in project_dirs.filter_map(|e| e.ok()) {
                    let exact = pe.path().join(sid).with_extension("jsonl");
                    if exact.exists() {
                        return Some(exact.to_string_lossy().into_owned());
                    }
                }
            }
        }

        // Fallback: most recent jsonl in project dir
        if let Some(cwd) = session.cwd.as_deref() {
            let cwd = cwd.trim_start_matches('/');
            if !cwd.is_empty() {
                let project_dir = projects_dir.join(
                    format!("-{}", cwd.replace('/', "-"))
                );
                if let Ok(entries) = std::fs::read_dir(&project_dir) {
                    let mut best: Option<(std::path::PathBuf, std::time::SystemTime)> = None;
                    for entry in entries.filter_map(|e| e.ok()) {
                        let p = entry.path();
                        if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                            continue;
                        }
                        if let Ok(mtime) = p.metadata().and_then(|m| m.modified()) {
                            if best.as_ref().is_none_or(|(_, t)| mtime > *t) {
                                  best = Some((p, mtime));
                            }
                        }
                    }
                    return best.map(|(p, _)| p.to_string_lossy().into_owned());
                }
            }
        }

        None
    }

    fn scan(&mut self) {
        let mut transcript_active = 0usize;
        let mut transcript_peak = 0usize;
        let mut task_fallback = 0usize;
        let mut any_idle = false;
        let now = SystemTime::now();

        // Primary: scan transcripts for accurate lifecycle
        for cache in self.caches.values_mut() {
            cache.poll();

            // Check for session idle from transcript events (last-prompt, turn_duration).
            if cache.session_idle {
                any_idle = true;
            }

            // Inactivity fallback: if no new JSONL data arrives for
            // INACTIVITY_IDLE_THRESHOLD, treat as idle regardless of active tool
            // count. Clear tracking BEFORE counting so the active count drops
            // to zero.  The JSONL file is written to continuously while Claude
            // is working — silence means the agent is done. Stale
            // active_tool_ids (from missing tool_result records) must not block
            // this fallback.
            if !cache.session_idle
                && cache.last_data_at.elapsed() >= INACTIVITY_IDLE_THRESHOLD
            {
                log::info!(
                    "[pixel_agents] inactivity timeout: clearing {} tool_ids + {} agent_ids, \
                     peak was {}",
                    cache.active_tool_ids.len(),
                    cache.active_agent_ids.len(),
                    cache.peak_active,
                );
                cache.session_idle = true;
                cache.active_tool_ids.clear();
                cache.active_agent_ids.clear();
                cache.peak_active = 0;
                any_idle = true;
            }

            // Count AFTER potential clearing above.
            transcript_active += cache.active_count();
            transcript_peak += cache.peak_active;
        }

        // Fallback: scan task output files if no transcripts are being tracked
        if self.caches.is_empty() {
            if let Ok(home) = std::env::var("HOME") {
                let tmp_dir = std::path::Path::new(&home).join(".claude").join("tmp");
                if let Ok(claude_dirs) = std::fs::read_dir(&tmp_dir) {
                    for claude_entry in claude_dirs.filter_map(|e| e.ok()) {
                        if let Ok(project_dirs) = std::fs::read_dir(claude_entry.path()) {
                            for project_entry in project_dirs.filter_map(|e| e.ok()) {
                                if let Ok(session_dirs) = std::fs::read_dir(project_entry.path()) {
                                    for session_entry in session_dirs.filter_map(|e| e.ok()) {
                                        let tasks_dir = session_entry.path().join("tasks");
                                        if let Ok(task_files) = std::fs::read_dir(&tasks_dir) {
                                            for task_entry in task_files.filter_map(|e| e.ok()) {
                                                if task_entry.path().extension()
                                                    .and_then(|e| e.to_str()) != Some("output")
                                                {
                                                    continue;
                                                }
                                                if task_entry.path().metadata().ok()
                                                    .and_then(|m| m.modified().ok())
                                                    .and_then(|t| now.duration_since(t).ok())
                                                    .is_some_and(|a| a < TASK_FALLBACK_WINDOW)
                                                    && is_output_file_active(&task_entry.path())
                                                {
                                                    task_fallback += 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let active = if transcript_active > 0 { transcript_active } else { task_fallback };
        // Peak is the higher of transcript-derived and task-fallback counts.
        // Do NOT carry over a stale cached_peak — it is reset when sessions go
        // idle (peak_active is zeroed on inactivity timeout).
        let peak = transcript_peak.max(task_fallback);

        if active != self.cached_active || peak != self.cached_peak || any_idle != self.any_session_idle {
            log::info!(
                "[pixel_agents] scan: active={active}, peak={peak}, idle={any_idle}, \
                 transcript=(a={transcript_active},p={transcript_peak}), \
                 fallback={task_fallback}, caches={}",
                self.caches.len(),
            );
        }

        // Remove caches for sessions that no longer exist
        self.caches.retain(|k, _| self.fed_paths.contains(k));
        self.fed_paths.clear();

        self.cached_active = active;
        self.cached_peak = peak;
        self.any_session_idle = any_idle;
    }
}

/// Classifies a JSONL transcript line to detect session idle/active state.
///
/// Uses proper JSON parsing rather than substring matching so format
/// variations (whitespace, field order) don't cause missed detections.
fn classify_transcript_line(
    line: &str,
    session_idle: &mut bool,
    active_tool_ids: &mut HashSet<String>,
    active_agent_ids: &mut HashSet<String>,
) {
    let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let record_type = record.get("type").and_then(|v| v.as_str());
    let subtype = record.get("subtype").and_then(|v| v.as_str());

    match (record_type, subtype) {
        // "last-prompt" — Claude Code finished all work and is back at its
        // prompt. Clear ALL tracked tools and background agents.
        (Some("last-prompt"), _) => {
            *session_idle = true;
            active_tool_ids.clear();
            active_agent_ids.clear();
        }
        // "turn_duration" — a single turn ended. Clear foreground tool IDs
        // (they are stale if tool_result was never written, e.g. sub-agent
        // crash). Background agent IDs are intentionally preserved — they can
        // span multiple turns.
        (_, Some("turn_duration")) => {
            *session_idle = true;
            active_tool_ids.clear();
        }
        // "assistant" — Claude started responding → session is active again.
        (Some("assistant"), _) => {
            *session_idle = false;
        }
        _ => {}
    }
}

/// Processes a single JSONL line, updating the set of active tool_use IDs and active background agent IDs.
fn process_jsonl_line(
    line: &str,
    active_tool_ids: &mut HashSet<String>,
    active_agent_ids: &mut HashSet<String>,
) {
    let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let Some(record_type) = record.get("type").and_then(|v| v.as_str()) else {
        return;
    };

    // 1. Check for background agent completion from task-notification attachments
    if record_type == "attachment" {
        let attachment = record.get("attachment");
        let is_task_notif = attachment
            .and_then(|a| a.get("type").and_then(|t| t.as_str()))
            == Some("task-notification");
        if is_task_notif {
            if let Some(agent_id) = attachment
                .and_then(|a| a.get("agentId"))
                .and_then(|a| a.as_str())
            {
                active_agent_ids.remove(agent_id);
            }
        }
        return;
    }

    // 2. Check for background agent launch from toolUseResult in user prompt submit response
    if record_type == "user" {
        if let Some(agent_id) = record.get("toolUseResult")
            .and_then(|t| t.get("agentId"))
            .and_then(|a| a.as_str())
        {
            active_agent_ids.insert(agent_id.to_owned());
        }
    }

    let content = record
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| record.get("content"));
    let Some(blocks) = content.and_then(|c| c.as_array()) else {
        return;
    };

    match record_type {
        "assistant" => {
            for block in blocks {
                let block_type = block.get("type").and_then(|t| t.as_str());
                let is_tool_use = block_type == Some("tool_use");
                if is_tool_use {
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    // Match Claude Code's sub-agent spawn tools:
                    // - Agent: primary sub-agent launcher
                    // - Task: legacy sub-agent launcher
                    // - TaskCreate: fleet task dispatch
                    // - Workflow: multi-agent workflow orchestration
                    let is_subagent = matches!(
                        name,
                        "Task" | "Agent" | "TaskCreate" | "Workflow"
                    );
                    if is_subagent {
                        if let Some(id) = block.get("id").and_then(|id| id.as_str()) {
                            active_tool_ids.insert(id.to_owned());
                        }
                    }
                }
            }
        }
        "user" => {
            for block in blocks {
                let is_tool_result =
                    block.get("type").and_then(|t| t.as_str()) == Some("tool_result");
                if is_tool_result {
                    if let Some(id) = block.get("tool_use_id").and_then(|id| id.as_str()) {
                        active_tool_ids.remove(id);
                    }
                }
            }
        }
        _ => {}
    }
}

pub(crate) struct PixelAgentsPanelView {
    sessions: HashMap<EntityId, PixelAgentSnapshot>,
    visible_terminal_ids: HashSet<EntityId>,
    /// Global scanner that aggregates sub-agent counts across all active transcripts.
    transcript_scanner: RefCell<GlobalTranscriptScanner>,
    /// Whether an animation timer is currently running.
    animation_timer_running: bool,
    /// Sub-agent characters in the scene (excludes the boss/main agent).
    sub_agents: Vec<SubAgentCharacter>,
    /// Cached sync state for change-detection logging (avoid spam).
    last_logged_active: usize,
    last_logged_idle: bool,
    last_logged_has_session: bool,
    last_logged_working: usize,
    last_logged_total: usize,
}

impl PixelAgentsPanelView {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            |view, _, event, ctx| {
                view.handle_session_event(event, ctx);
            },
        );

        Self {
            sessions: HashMap::new(),
            visible_terminal_ids: HashSet::new(),
            transcript_scanner: RefCell::new(GlobalTranscriptScanner::new()),
            animation_timer_running: false,
            sub_agents: Vec::new(),
            last_logged_active: 0,
            last_logged_idle: false,
            last_logged_has_session: false,
            last_logged_working: 0,
            last_logged_total: 0,
        }
    }

    /// Returns true if the animation loop should keep running.
    /// Timer runs whenever the panel has visible terminals or any tracked state.
    fn needs_animation_timer(&self) -> bool {
        !self.visible_terminal_ids.is_empty()
            || !self.sessions.is_empty()
            || !self.sub_agents.is_empty()
    }

    /// Spawns a periodic timer that re-renders the view every 240ms while agents are active.
    fn ensure_animation_timer(&mut self, ctx: &mut ViewContext<Self>) {
        if self.animation_timer_running {
            return;
        }
        if !self.needs_animation_timer() {
            return;
        }
        self.schedule_tick(ctx);
    }

    /// Spawns a one-shot 240ms timer. The callback re-spawns the timer if needed.
    fn schedule_tick(&mut self, ctx: &mut ViewContext<Self>) {
        self.animation_timer_running = true;
        ctx.spawn(
            async { Timer::after(Duration::from_millis(240)).await },
            |me, _, ctx| {
                me.animation_timer_running = false;
                // Keep sub-agents in sync with sessions (safety net for missed events).
                me.sync_sub_agents();
                // Advance all character animations
                for sa in &mut me.sub_agents {
                    match sa.state {
                        SubAgentState::WalkingIn(frame) => {
                            let next = frame + 1;
                            let waypoints = sa.walk_in_waypoints();
                            let (x, y) = SubAgentCharacter::lerp_along_path(
                                &waypoints,
                                next,
                                WALK_IN_FRAMES,
                            );
                            // Update facing based on dominant movement axis.
                            let dx = x - sa.current_x;
                            let dy = y - sa.current_y;
                            if dx.abs() > 0.1 || dy.abs() > 0.1 {
                                if dx.abs() > dy.abs() {
                                    // Horizontal movement dominates.
                                    sa.facing = if dx > 0. { FacingDirection::Right } else { FacingDirection::Left };
                                } else if dy < -0.1 {
                                    // Moving up → show back.
                                    sa.facing = FacingDirection::Back;
                                }
                                // Moving down: keep current facing (side looks natural).
                            }
                            sa.current_x = x;
                            sa.current_y = y;
                            sa.state = if next >= WALK_IN_FRAMES {
                                SubAgentState::Working
                            } else {
                                SubAgentState::WalkingIn(next)
                            };
                        }
                        SubAgentState::WalkingOut(frame) => {
                            let next = frame + 1;
                            let waypoints = sa.walk_out_waypoints();
                            let (x, y) = SubAgentCharacter::lerp_along_path(
                                &waypoints,
                                next,
                                WALK_OUT_FRAMES,
                            );
                            let dx = x - sa.current_x;
                            let dy = y - sa.current_y;
                            if dx.abs() > 0.1 || dy.abs() > 0.1 {
                                if dx.abs() > dy.abs() {
                                    sa.facing = if dx > 0. { FacingDirection::Right } else { FacingDirection::Left };
                                } else if dy < -0.1 {
                                    sa.facing = FacingDirection::Back;
                                }
                            }
                            sa.current_x = x;
                            sa.current_y = y;
                            sa.state = if next >= WALK_OUT_FRAMES {
                                // Snap to exact report position.
                                let (rx, ry) = sa.report_position();
                                sa.current_x = rx;
                                sa.current_y = ry;
                                SubAgentState::Reporting(0)
                            } else {
                                SubAgentState::WalkingOut(next)
                            };
                        }
                        SubAgentState::Reporting(timer) => {
                            sa.state = SubAgentState::Reporting(timer + 1);
                        }
                        SubAgentState::Working => {
                            let (x, y) = sa.desk_position();
                            sa.current_x = x;
                            sa.current_y = y;
                        }
                    }
                }
                // Auto-remove characters that finished reporting
                me.sub_agents.retain(|sa| {
                    !matches!(sa.state, SubAgentState::Reporting(t) if t >= REPORT_FRAMES)
                });
                let has_animating = me.sub_agents.iter().any(|sa| {
                    !matches!(sa.state, SubAgentState::Working)
                });
                if me.needs_animation_timer() || has_animating {
                    me.schedule_tick(ctx);
                }
                ctx.notify();
            },
        );
    }


    pub(crate) fn set_visible_terminal_ids(
        &mut self,
        terminal_ids: Vec<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let visible_terminal_ids = terminal_ids.into_iter().collect();
        if self.visible_terminal_ids != visible_terminal_ids {
            self.visible_terminal_ids = visible_terminal_ids;
            // Populate sessions from model for terminals already running.
            // This covers the case where the panel is shown after agents started;
            // Started events were missed and SessionUpdated only fires on new activity.
            let model = CLIAgentSessionsModel::handle(ctx);
            let model = model.as_ref(ctx);
            for &id in &self.visible_terminal_ids {
                if let std::collections::hash_map::Entry::Vacant(e) = self.sessions.entry(id) {
                    if let Some(session) = model.session(id) {
                        e.insert(PixelAgentSnapshot {
                            terminal_view_id: id,
                            agent: session.agent,
                            status: session.status.clone(),
                            session_context: session.session_context.clone(),
                        });
                    }
                }
            }
            self.sync_sub_agents();
            if self.needs_animation_timer() {
                self.ensure_animation_timer(ctx);
            }
            ctx.notify();
        }
    }

    fn sync_sub_agents(&mut self) {
        // Feed sessions before scanning
        {
            let mut scanner = self.transcript_scanner.borrow_mut();
            for s in self.sessions.values() {
                scanner.feed_session(&s.session_context);
            }
        }
        let (_peak, active) = self.transcript_scanner.borrow_mut().counts();
        let session_idle = self.transcript_scanner.borrow().any_session_idle;

        // If the transcript shows the session is idle (Claude Code at prompt)
        // but the local snapshot still shows InProgress, update to Success.
        // This handles the case where the CLI agent plugin doesn't send
        // stop/idle_prompt OSC events when Claude Code finishes processing.
        if session_idle {
            for s in self.sessions.values_mut() {
                if matches!(s.status, CLIAgentSessionStatus::InProgress) {
                    log::info!(
                        "[pixel_agents] transcript idle detected, transitioning session {:?} to Success",
                        s.terminal_view_id,
                    );
                    s.status = CLIAgentSessionStatus::Success;
                }
            }
        } else if active > 0 {
            // Reverse: if the session was previously marked idle/Success but
            // the transcript now shows active sub-agents again (user submitted
            // a new prompt), transition back to InProgress.
            for s in self.sessions.values_mut() {
                if matches!(s.status, CLIAgentSessionStatus::Success) {
                    log::info!(
                        "[pixel_agents] transcript active again, transitioning session {:?} back to InProgress",
                        s.terminal_view_id,
                    );
                    s.status = CLIAgentSessionStatus::InProgress;
                }
            }
        }

        // Only create characters when there are actual in-progress sessions.
        // Don't use in_progress as a numeric cap — one session can have many sub-agents.
        let has_active_session = self
            .sessions
            .values()
            .any(|s| matches!(s.status, CLIAgentSessionStatus::InProgress));
        let target = if has_active_session { active.min(8) } else { 0 };
        let working = self
            .sub_agents
            .iter()
            .filter(|sa| sa.state == SubAgentState::Working)
            .count();
        let total = self.sub_agents.len();
        // Only log when sync state actually changes — avoid log spam every 240ms.
        if active != self.last_logged_active
            || session_idle != self.last_logged_idle
            || has_active_session != self.last_logged_has_session
            || working != self.last_logged_working
            || total != self.last_logged_total
        {
            log::info!(
                "[pixel_agents] sync_sub_agents: active={active}, idle={session_idle}, \
                 has_session={has_active_session}, sub_agents={total}, working={working}",
            );
            self.last_logged_active = active;
            self.last_logged_idle = session_idle;
            self.last_logged_has_session = has_active_session;
            self.last_logged_working = working;
            self.last_logged_total = total;
        }

        // If we have more Working characters than the target count,
        // transition the excess to WalkingOut (they've completed).
        if working > target {
            let mut to_complete = working.saturating_sub(target);
            for sa in &mut self.sub_agents {
                if to_complete == 0 {
                    break;
                }
                if sa.state == SubAgentState::Working {
                    sa.state = SubAgentState::WalkingOut(0);
                    to_complete -= 1;
                }
            }
        }

        // Add new WalkingIn characters until we reach the peak target
        while self
            .sub_agents
            .iter()
            .filter(|sa| {
                matches!(
                    sa.state,
                    SubAgentState::Working
                        | SubAgentState::WalkingIn(_)
                        | SubAgentState::WalkingOut(_)
                        | SubAgentState::Reporting(_)
                )
            })
            .count()
            < target
        {
            let idx = self.sub_agents.len();
            if idx / 2 >= 4 {
                break; // max 4 rows (8 characters)
            }
            let char_index = idx % 6;
            let desk_row = idx / 2;
            let desk_side = idx.is_multiple_of(2);
            self.sub_agents.push(SubAgentCharacter {
                char_index,
                state: SubAgentState::WalkingIn(0),
                desk_row,
                desk_side,
                name: String::new(),
                current_x: ENTRANCE_X,
                current_y: ENTRANCE_Y,
                facing: if desk_side { FacingDirection::Left } else { FacingDirection::Right },
                task_label: None,
            });
        }
    }

    fn handle_session_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id,
                agent,
            } => {
                log::info!(
                    "[pixel_agents] Started: terminal={terminal_view_id:?}, agent={agent:?}"
                );
                self.sessions.insert(
                    *terminal_view_id,
                    PixelAgentSnapshot {
                        terminal_view_id: *terminal_view_id,
                        agent: *agent,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                    },
                );
                // Don't add a character here — the Started event fires for
                // the main session (e.g. Claude Code opening), not for sub-agents.
                // Characters are created by sync_sub_agents when the transcript
                // scanner detects actual sub-agent tool calls (active > 0).
                ctx.notify();
                self.ensure_animation_timer(ctx);
            }
            CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status,
                session_context,
            } => {
                // If transitioning to Success, start walk-out animation for ALL sub-agents.
                // The main session ending means all sub-agent work is done.
                if matches!(status, CLIAgentSessionStatus::Success) {
                    let mut any_transitioned = false;
                    for sa in &mut self.sub_agents {
                        if sa.state == SubAgentState::Working {
                            sa.state = SubAgentState::WalkingOut(0);
                            any_transitioned = true;
                        }
                    }
                    if any_transitioned {
                        self.ensure_animation_timer(ctx);
                    }
                }
                self.sessions
                    .entry(*terminal_view_id)
                    .and_modify(|snapshot| {
                        snapshot.agent = *agent;
                        snapshot.status = status.clone();
                        snapshot.session_context = *session_context.clone();
                    })
                    .or_insert_with(|| PixelAgentSnapshot {
                        terminal_view_id: *terminal_view_id,
                        agent: *agent,
                        status: status.clone(),
                        session_context: *session_context.clone(),
                    });
                ctx.notify();
                if matches!(status, CLIAgentSessionStatus::InProgress) {
                    self.ensure_animation_timer(ctx);
                }
            }
            CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: _,
            } => {
                // Transition ALL remaining Working characters to WalkingOut.
                // The main session has ended, so all sub-agent work is complete.
                let mut any_transitioned = false;
                for sa in &mut self.sub_agents {
                    if sa.state == SubAgentState::Working {
                        sa.state = SubAgentState::WalkingOut(0);
                        any_transitioned = true;
                    }
                }
                if any_transitioned {
                    self.ensure_animation_timer(ctx);
                }
                self.sessions.remove(terminal_view_id);
                ctx.notify();
            }
            CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id,
                agent: _,
            } => {
                if let Some(session) = CLIAgentSessionsModel::handle(ctx)
                    .as_ref(ctx)
                    .session(*terminal_view_id)
                {
                    log::info!(
                        "[pixel_agents] SessionUpdated: cwd={:?}, session_id={:?}",
                        session.session_context.cwd,
                        session.session_context.session_id,
                    );
                    // Only notify if visible-relevant data actually changed
                    let changed = match self.sessions.get(terminal_view_id) {
                        Some(existing) => {
                            existing.agent != session.agent
                                || existing.status != session.status
                                || existing.session_context.tool_name
                                    != session.session_context.tool_name
                                || existing.session_context.project
                                    != session.session_context.project
                                || existing.session_context.summary
                                    != session.session_context.summary
                                || existing.session_context.query != session.session_context.query
                                || existing.session_context.transcript_path
                                    != session.session_context.transcript_path
                        }
                        None => true,
                    };
                    // Update task_label on working sub-agent characters
                    let new_label = session
                        .session_context
                        .tool_name
                        .clone()
                        .or_else(|| session.session_context.summary.clone());
                    if let Some(sa) = self
                        .sub_agents
                        .iter_mut()
                        .find(|sa| sa.state == SubAgentState::Working)
                    {
                        sa.task_label = new_label;
                    }
                    self.sessions.insert(
                        *terminal_view_id,
                        PixelAgentSnapshot {
                            terminal_view_id: *terminal_view_id,
                            agent: session.agent,
                            status: session.status.clone(),
                            session_context: session.session_context.clone(),
                        },
                    );
                    // Ensure sub-agent characters stay in sync after session updates.
                    self.sync_sub_agents();
                    if changed {
                        ctx.notify();
                    }
                }
            }
            CLIAgentSessionsModelEvent::InputSessionChanged { .. } => {}
        }
    }

    fn visible_snapshots(&self) -> Vec<&PixelAgentSnapshot> {
        let mut snapshots: Vec<&PixelAgentSnapshot> = self
            .sessions
            .values()
            .filter(|snapshot| {
                self.visible_terminal_ids.is_empty()
                    || self
                        .visible_terminal_ids
                        .contains(&snapshot.terminal_view_id)
            })
            .collect();
        snapshots.sort_by(|a, b| {
            a.agent
                .display_name()
                .cmp(b.agent.display_name())
                .then_with(|| agent_title(a).cmp(&agent_title(b)))
        });
        snapshots
    }


    fn render_snapshot(
        snapshot: &PixelAgentSnapshot,
        sub_agents: &[SubAgentCharacter],
        app: &AppContext,
        animation_frame: usize,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let title_color = theme.main_text_color(theme.background()).into_solid();
        let detail_color = theme.sub_text_color(theme.background()).into_solid();
        let terminal_view_id = snapshot.terminal_view_id;
        let status = agent_supports_rich_status(&snapshot.agent)
            .then(|| snapshot.status.to_conversation_status());
        let icon = render_icon_with_status(
            IconWithStatusVariant::CLIAgent {
                agent: snapshot.agent,
                status,
            },
            &IconWithStatusSizing {
                icon_size: AGENT_ICON_SIZE,
                padding: AGENT_ICON_PADDING,
                badge_icon_size: 10.,
                badge_padding: 2.,
                overall_size_override: Some(AGENT_ICON_SIZE + AGENT_ICON_PADDING * 2.),
                badge_offset: (1., 1.),
            },
            theme,
            theme.background(),
        );
        let scene = render_pixel_agents_scene(snapshot, sub_agents, animation_frame, appearance.ui_font_family());

        let title = Text::new(
            agent_title(snapshot),
            appearance.ui_font_family(),
            TITLE_FONT_SIZE,
        )
        .with_color(title_color)
        .with_clip(ClipConfig::ellipsis())
        .finish();

        let detail = Text::new(
            agent_detail(snapshot),
            appearance.ui_font_family(),
            DETAIL_FONT_SIZE,
        )
        .with_color(detail_color)
        .with_clip(ClipConfig::ellipsis())
        .finish();

        let header = Flex::row()
            .with_spacing(AGENT_ROW_SPACING)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(
                Shrinkable::new(
                    1.,
                    Flex::column()
                        .with_spacing(3.)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(title)
                        .with_child(detail)
                        .finish(),
                )
                .finish(),
            )
            .finish();

        let card = Flex::column()
            .with_spacing(8.)
            .with_child(scene)
            .with_child(header)
            .finish();

        Hoverable::new(MouseStateHandle::default(), move |_| {
            Container::new(card)
                .with_uniform_padding(AGENT_ROW_PADDING)
                .with_background(internal_colors::fg_overlay_1(theme))
                .with_border(Border::all(1.).with_border_fill(internal_colors::fg_overlay_2(theme)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(AGENT_ROW_RADIUS)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::FocusTerminalViewInWorkspace {
                terminal_view_id,
            });
        })
        .finish()
    }

    fn render_empty(app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Container::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        "No CLI agents are running in this workspace.",
                        appearance.ui_font_family(),
                        EMPTY_FONT_SIZE,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into_solid())
                    .finish(),
                )
                .finish(),
        )
        .with_uniform_padding(16.)
        .finish()
    }
}

const TILE: f32 = 16.;
const SCENE_W: f32 = 224.;
const SCENE_H: f32 = 336.;
const MIN_SCENE_SCALE: f32 = 0.65;
const MAX_SCENE_SCALE: f32 = 1.65;

const CHAR_FRAME_W: f32 = 16.;
const CHAR_FRAME_H: f32 = 32.;

fn render_pixel_agents_scene(
    snapshot: &PixelAgentSnapshot,
    sub_agents: &[SubAgentCharacter],
    animation_frame: usize,
    font_family: FamilyId,
) -> Box<dyn Element> {
    PixelAgentsSceneElement::new(snapshot.clone(), sub_agents.to_vec(), animation_frame, font_family).finish()
}

fn render_pixel_agents_scene_at_scale(
    sub_agents: &[SubAgentCharacter],
    boss_name: &str,
    animation_frame: usize,
    scale: f32,
    font_family: FamilyId,
) -> Box<dyn Element> {
    let mut s = Stack::new();

    render_office_tiles(&mut s, scale);
    render_wall_furniture(&mut s, scale);
    render_central_workstations(&mut s, sub_agents, animation_frame, scale, font_family);
    render_boss_workstation(&mut s, boss_name, animation_frame, scale);
    render_lounge_area(&mut s, scale);
    render_meeting_area(&mut s, scale);
    render_bottom_area(&mut s, scale);

    ConstrainedBox::new(s.finish())
        .with_width(scaled(SCENE_W, scale))
        .with_height(scaled(SCENE_H, scale))
        .finish()
}

fn render_office_tiles(stack: &mut Stack, scale: f32) {
    add(
        stack,
        "bundled/pixel-agents/assets/office_background.png",
        0.,
        0.,
        SCENE_W,
        SCENE_H,
        scale,
    );
}

fn render_wall_furniture(stack: &mut Stack, scale: f32) {
    // Left wall: single bookshelf
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DOUBLE_BOOKSHELF/DOUBLE_BOOKSHELF.png",
        20.,
        10.,
        32.,
        32.,
        scale,
    );
    // Right wall: single bookshelf
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DOUBLE_BOOKSHELF/DOUBLE_BOOKSHELF.png",
        172.,
        10.,
        32.,
        32.,
        scale,
    );
    // Whiteboard on left wall, above large plant
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/WHITEBOARD/WHITEBOARD.png",
        8.,
        18.,
        32.,
        32.,
        scale,
    );
    // Left edge: large plant
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LARGE_PLANT/LARGE_PLANT.png",
        4.,
        44.,
        32.,
        48.,
        scale,
    );
    // Right edge: large plant
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LARGE_PLANT/LARGE_PLANT.png",
        188.,
        42.,
        32.,
        48.,
        scale,
    );
}

fn render_central_workstations(
    stack: &mut Stack,
    sub_agents: &[SubAgentCharacter],
    animation_frame: usize,
    scale: f32,
    font_family: FamilyId,
) {
    let desk_x = 90.;
    let desk_y = 78.;
    let row_gap = 46.;

    // Always render 4 rows of office furniture (permanent background).
    for row in 0..4 {
        let y = desk_y + row as f32 * row_gap;
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/SMALL_TABLE/SMALL_TABLE_SIDE.png",
            desk_x,
            y,
            16.,
            48.,
            scale,
        );
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/SMALL_TABLE/SMALL_TABLE_SIDE.png",
            desk_x + 16.,
            y,
            16.,
            48.,
            scale,
        );
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/LAPTOP/LAPTOP_SIDE_RIGHT.png",
            desk_x + 3.,
            y + 15.,
            16.,
            16.,
            scale,
        );
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/LAPTOP/LAPTOP_SIDE_LEFT.png",
            desk_x + 14.,
            y + 15.,
            16.,
            16.,
            scale,
        );
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/COFFEE/COFFEE.png",
            desk_x + 2.,
            y + 31.,
            16.,
            16.,
            scale,
        );
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/WOODEN_CHAIR/WOODEN_CHAIR_SIDE.png",
            desk_x - 13.,
            y + 8.,
            16.,
            32.,
            scale,
        );
        add(
            stack,
            "bundled/pixel-agents/assets/furniture/WOODEN_CHAIR/WOODEN_CHAIR_SIDE_LEFT.png",
            desk_x + 29.,
            y + 8.,
            16.,
            32.,
            scale,
        );
    }
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/WOODEN_BENCH/WOODEN_BENCH.png",
        desk_x,
        desk_y + 184.,
        48.,
        16.,
        scale,
    );

    // Render characters at their current positions based on state.
    for sa in sub_agents {
        match sa.state {
            SubAgentState::Working => {
                add_char_frame(
                    stack,
                    sa.char_index,
                    sa.desk_side,
                    animation_frame,
                    sa.current_x,
                    sa.current_y,
                    scale,
                );
            }
            SubAgentState::WalkingIn(_) | SubAgentState::WalkingOut(_) => {
                match sa.facing {
                    FacingDirection::Back => {
                        add_back_walk_frame(
                            stack,
                            sa.char_index,
                            animation_frame,
                            sa.current_x,
                            sa.current_y,
                            scale,
                        );
                    }
                    _ => {
                        add_walk_frame(
                            stack,
                            sa.char_index,
                            sa.facing == FacingDirection::Right,
                            animation_frame,
                            sa.current_x,
                            sa.current_y,
                            scale,
                        );
                    }
                }
            }
            SubAgentState::Reporting(_) => {
                add_front_idle_frame(
                    stack,
                    sa.char_index,
                    sa.current_x,
                    sa.current_y,
                    scale,
                );
            }
        }
    }

    // Render task bubbles on top of all character sprites.
    render_task_bubbles(stack, sub_agents, scale, font_family);
}

fn render_boss_workstation(
    stack: &mut Stack,
    _boss_name: &str,
    _animation_frame: usize,
    scale: f32,
) {
    let boss_x = 88.;
    let boss_y = 48.;

    // Boss chair (front-facing, rendered first so it's behind character)
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/WOODEN_CHAIR/WOODEN_CHAIR_FRONT.png",
        boss_x + 16.,
        boss_y - 16.,
        16.,
        32.,
        scale,
    );
    // Boss character (char_0 front-facing idle, positioned so desk hides lower body)
    add(
        stack,
        "bundled/pixel-agents/assets/characters/char_0_single_idle.png",
        boss_x + 16.,
        boss_y - 16., // bottom of character extends below desk top → desk covers lower body
        CHAR_FRAME_W,
        CHAR_FRAME_H,
        scale,
    );
    // Boss desk (front-facing) - rendered after character so desk appears in front
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DESK/DESK_FRONT.png",
        boss_x,
        boss_y,
        48.,
        32.,
        scale,
    );
    // Coffee on desk
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/COFFEE/COFFEE.png",
        boss_x + 36.,
        boss_y + 8.,
        16.,
        16.,
        scale,
    );
}

fn render_lounge_area(stack: &mut Stack, scale: f32) {
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LARGE_PLANT/LARGE_PLANT.png",
        10.,
        210.,
        32.,
        48.,
        scale,
    );
}

fn render_meeting_area(stack: &mut Stack, scale: f32) {
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LARGE_PLANT/LARGE_PLANT.png",
        190.,
        186.,
        32.,
        48.,
        scale,
    );
}

fn render_bottom_area(stack: &mut Stack, scale: f32) {
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/BOOKSHELF/BOOKSHELF.png",
        150.,
        286.,
        32.,
        16.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/HANGING_PLANT/HANGING_PLANT.png",
        184.,
        260.,
        16.,
        32.,
        scale,
    );
}

fn scaled(v: f32, s: f32) -> f32 {
    v * s
}

fn add(stack: &mut Stack, path: &'static str, x: f32, y: f32, w: f32, h: f32, scale: f32) {
    stack.add_positioned_child(
        ConstrainedBox::new(
            Image::new(AssetSource::Bundled { path }, CacheOption::Original)
                .contain()
                .finish(),
        )
        .with_width(scaled(w, scale))
        .with_height(scaled(h, scale))
        .finish(),
        OffsetPositioning::offset_from_parent(
            vec2f(scaled(x, scale), scaled(y, scale)),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::TopLeft,
            ChildAnchor::TopLeft,
        ),
    );
}

fn add_char_frame(
    stack: &mut Stack,
    char_index: usize,
    facing_right: bool,
    animation_frame: usize,
    x: f32,
    y: f32,
    scale: f32,
) {
    let i = char_index % 6;
    let frame = animation_frame % 2;
    let path = match (i, facing_right, frame) {
        (0, true, 0) => "bundled/pixel-agents/assets/characters/char_0_side_sit_right_0.png",
        (0, true, 1) => "bundled/pixel-agents/assets/characters/char_0_side_sit_right_1.png",
        (1, true, 0) => "bundled/pixel-agents/assets/characters/char_1_side_sit_right_0.png",
        (1, true, 1) => "bundled/pixel-agents/assets/characters/char_1_side_sit_right_1.png",
        (2, true, 0) => "bundled/pixel-agents/assets/characters/char_2_side_sit_right_0.png",
        (2, true, 1) => "bundled/pixel-agents/assets/characters/char_2_side_sit_right_1.png",
        (3, true, 0) => "bundled/pixel-agents/assets/characters/char_3_side_sit_right_0.png",
        (3, true, 1) => "bundled/pixel-agents/assets/characters/char_3_side_sit_right_1.png",
        (4, true, 0) => "bundled/pixel-agents/assets/characters/char_4_side_sit_right_0.png",
        (4, true, 1) => "bundled/pixel-agents/assets/characters/char_4_side_sit_right_1.png",
        (5, true, 0) => "bundled/pixel-agents/assets/characters/char_5_side_sit_right_0.png",
        (5, true, 1) => "bundled/pixel-agents/assets/characters/char_5_side_sit_right_1.png",
        (0, false, 0) => "bundled/pixel-agents/assets/characters/char_0_side_sit_left_0.png",
        (0, false, 1) => "bundled/pixel-agents/assets/characters/char_0_side_sit_left_1.png",
        (1, false, 0) => "bundled/pixel-agents/assets/characters/char_1_side_sit_left_0.png",
        (1, false, 1) => "bundled/pixel-agents/assets/characters/char_1_side_sit_left_1.png",
        (2, false, 0) => "bundled/pixel-agents/assets/characters/char_2_side_sit_left_0.png",
        (2, false, 1) => "bundled/pixel-agents/assets/characters/char_2_side_sit_left_1.png",
        (3, false, 0) => "bundled/pixel-agents/assets/characters/char_3_side_sit_left_0.png",
        (3, false, 1) => "bundled/pixel-agents/assets/characters/char_3_side_sit_left_1.png",
        (4, false, 0) => "bundled/pixel-agents/assets/characters/char_4_side_sit_left_0.png",
        (4, false, 1) => "bundled/pixel-agents/assets/characters/char_4_side_sit_left_1.png",
        (5, false, 0) => "bundled/pixel-agents/assets/characters/char_5_side_sit_left_0.png",
        (5, false, 1) => "bundled/pixel-agents/assets/characters/char_5_side_sit_left_1.png",
        _ => "bundled/pixel-agents/assets/characters/char_0_side_sit_right_0.png",
    };
    add(stack, path, x, y, CHAR_FRAME_W, CHAR_FRAME_H, scale);
}

/// Renders a character in walking pose using side-facing sprites + vertical bob.
fn add_walk_frame(
    stack: &mut Stack,
    char_index: usize,
    walking_right: bool,
    animation_frame: usize,
    x: f32,
    y: f32,
    scale: f32,
) {
    let i = char_index % 6;
    let path: &'static str = match (i, walking_right) {
        (0, true) => "bundled/pixel-agents/assets/characters/char_0_side_right.png",
        (0, false) => "bundled/pixel-agents/assets/characters/char_0_side_left.png",
        (1, true) => "bundled/pixel-agents/assets/characters/char_1_side_right.png",
        (1, false) => "bundled/pixel-agents/assets/characters/char_1_side_left.png",
        (2, true) => "bundled/pixel-agents/assets/characters/char_2_side_right.png",
        (2, false) => "bundled/pixel-agents/assets/characters/char_2_side_left.png",
        (3, true) => "bundled/pixel-agents/assets/characters/char_3_side_right.png",
        (3, false) => "bundled/pixel-agents/assets/characters/char_3_side_left.png",
        (4, true) => "bundled/pixel-agents/assets/characters/char_4_side_right.png",
        (4, false) => "bundled/pixel-agents/assets/characters/char_4_side_left.png",
        (5, true) => "bundled/pixel-agents/assets/characters/char_5_side_right.png",
        (5, false) => "bundled/pixel-agents/assets/characters/char_5_side_left.png",
        _ => "bundled/pixel-agents/assets/characters/char_0_side_right.png",
    };
    // Vertical bob: ±1px per frame to simulate walking
    let bob = if animation_frame.is_multiple_of(2) { -1. } else { 0. };
    add(stack, path, x, y + bob, CHAR_FRAME_W, CHAR_FRAME_H, scale);
}

/// Renders a character in back-facing walking pose (walking away from camera, y decreasing).
/// Alternates between back_left and back_right sprites + vertical bob.
fn add_back_walk_frame(
    stack: &mut Stack,
    char_index: usize,
    animation_frame: usize,
    x: f32,
    y: f32,
    scale: f32,
) {
    let i = char_index % 6;
    let use_left = animation_frame.is_multiple_of(2);
    let path: &'static str = match (i, use_left) {
        (0, true) => "bundled/pixel-agents/assets/characters/char_0_back_left.png",
        (0, false) => "bundled/pixel-agents/assets/characters/char_0_back_right.png",
        (1, true) => "bundled/pixel-agents/assets/characters/char_1_back_left.png",
        (1, false) => "bundled/pixel-agents/assets/characters/char_1_back_right.png",
        (2, true) => "bundled/pixel-agents/assets/characters/char_2_back_left.png",
        (2, false) => "bundled/pixel-agents/assets/characters/char_2_back_right.png",
        (3, true) => "bundled/pixel-agents/assets/characters/char_3_back_left.png",
        (3, false) => "bundled/pixel-agents/assets/characters/char_3_back_right.png",
        (4, true) => "bundled/pixel-agents/assets/characters/char_4_back_left.png",
        (4, false) => "bundled/pixel-agents/assets/characters/char_4_back_right.png",
        (5, true) => "bundled/pixel-agents/assets/characters/char_5_back_left.png",
        (5, false) => "bundled/pixel-agents/assets/characters/char_5_back_right.png",
        _ => "bundled/pixel-agents/assets/characters/char_0_back_left.png",
    };
    let bob = if animation_frame.is_multiple_of(2) { -1. } else { 0. };
    add(stack, path, x, y + bob, CHAR_FRAME_W, CHAR_FRAME_H, scale);
}

/// Renders a character in front-facing idle pose (for reporting at boss desk).
fn add_front_idle_frame(
    stack: &mut Stack,
    char_index: usize,
    x: f32,
    y: f32,
    scale: f32,
) {
    let path: &'static str = match char_index % 6 {
        0 => "bundled/pixel-agents/assets/characters/char_0_single_idle.png",
        1 => "bundled/pixel-agents/assets/characters/char_1_single_idle.png",
        2 => "bundled/pixel-agents/assets/characters/char_2_single_idle.png",
        3 => "bundled/pixel-agents/assets/characters/char_3_single_idle.png",
        4 => "bundled/pixel-agents/assets/characters/char_4_single_idle.png",
        5 => "bundled/pixel-agents/assets/characters/char_5_single_idle.png",
        _ => "bundled/pixel-agents/assets/characters/char_0_single_idle.png",
    };
    add(stack, path, x, y, CHAR_FRAME_W, CHAR_FRAME_H, scale);
}

const BUBBLE_FONT_SIZE: f32 = 5.;
const BUBBLE_PADDING: f32 = 2.;
const BUBBLE_MAX_CHARS: usize = 12;
const BUBBLE_OFFSET_Y: f32 = -10.;

/// Renders task label bubbles above working characters.
fn render_task_bubbles(
    stack: &mut Stack,
    sub_agents: &[SubAgentCharacter],
    scale: f32,
    font_family: FamilyId,
) {
    for sa in sub_agents {
        if !matches!(sa.state, SubAgentState::Working) {
            continue;
        }
        let Some(label) = sa.task_label.as_deref() else {
            continue;
        };
        if label.is_empty() {
            continue;
        }
        let display: String = if label.chars().count() > BUBBLE_MAX_CHARS {
            let truncated: String = label.chars().take(BUBBLE_MAX_CHARS - 1).collect();
            format!("{truncated}…")
        } else {
            label.to_owned()
        };

        let bubble = Container::new(
            Text::new(display, font_family, scaled(BUBBLE_FONT_SIZE, scale))
                .with_color(ColorU::new(255, 255, 255, 255))
                .with_clip(ClipConfig::ellipsis())
                .finish(),
        )
        .with_uniform_padding(scaled(BUBBLE_PADDING, scale))
        .with_background_color(ColorU::new(0, 0, 0, 180))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(scaled(3., scale))))
        .finish();

        let bx = sa.current_x + CHAR_FRAME_W / 2.;
        let by = sa.current_y + BUBBLE_OFFSET_Y;
        stack.add_positioned_child(
            bubble,
            OffsetPositioning::offset_from_parent(
                vec2f(scaled(bx, scale), scaled(by, scale)),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );
    }
}

struct PixelAgentsSceneElement {
    snapshot: PixelAgentSnapshot,
    sub_agents: Vec<SubAgentCharacter>,
    animation_frame: usize,
    font_family: FamilyId,
    child: Option<Box<dyn Element>>,
    size: Option<pathfinder_geometry::vector::Vector2F>,
    origin: Option<Point>,
    /// Cache key: (scale_bits, sub_agents_hash, is_working, animation_frame).
    cache_key: Option<(u32, u64, bool, usize)>,
}

impl PixelAgentsSceneElement {
    fn new(
        snapshot: PixelAgentSnapshot,
        sub_agents: Vec<SubAgentCharacter>,
        animation_frame: usize,
        font_family: FamilyId,
    ) -> Self {
        Self {
            snapshot,
            sub_agents,
            animation_frame,
            font_family,
            child: None,
            size: None,
            origin: None,
            cache_key: None,
        }
    }

    fn scale_for_constraint(constraint: SizeConstraint) -> f32 {
        if constraint.max.x().is_finite() {
            (constraint.max.x() / SCENE_W).clamp(MIN_SCENE_SCALE, MAX_SCENE_SCALE)
        } else {
            1.
        }
    }

    /// Compute a simple hash of sub_agents state for cache invalidation.
    fn sub_agents_hash(sub_agents: &[SubAgentCharacter]) -> u64 {
        let mut h: u64 = sub_agents.len() as u64;
        for sa in sub_agents {
            h = h.wrapping_mul(31).wrapping_add(sa.current_x.to_bits() as u64);
            h = h.wrapping_mul(31).wrapping_add(sa.current_y.to_bits() as u64);
            h = h.wrapping_mul(31).wrapping_add(match sa.state {
                SubAgentState::WalkingIn(f) => f as u64,
                SubAgentState::Working => 100,
                SubAgentState::WalkingOut(f) => 200 + f as u64,
                SubAgentState::Reporting(t) => 300 + t as u64,
            });
            if let Some(label) = &sa.task_label {
                for b in label.bytes() {
                    h = h.wrapping_mul(31).wrapping_add(b as u64);
                }
            }
        }
        h
    }
}

impl Element for PixelAgentsSceneElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> pathfinder_geometry::vector::Vector2F {
        let scale = Self::scale_for_constraint(constraint);
        let width = scaled(SCENE_W, scale);
        let height = scaled(SCENE_H, scale);
        let is_working = matches!(self.snapshot.status, CLIAgentSessionStatus::InProgress);
        let key = (
            scale.to_bits(),
            Self::sub_agents_hash(&self.sub_agents),
            is_working,
            self.animation_frame % 2,
        );

        // Only rebuild scene tree when key changes
        let needs_rebuild = self.cache_key != Some(key);

        if needs_rebuild {
            let boss_name = agent_title(&self.snapshot);
            let mut child = render_pixel_agents_scene_at_scale(
                &self.sub_agents,
                &boss_name,
                self.animation_frame,
                scale,
                self.font_family,
            );
            let size = child.layout(
                SizeConstraint {
                    min: vec2f(width, height),
                    max: vec2f(width, height),
                },
                ctx,
                app,
            );
            self.child = Some(child);
            self.size = Some(size);
            self.cache_key = Some(key);
        } else {
            // Re-layout cached child with same constraint (no tree rebuild)
            if let Some(child) = self.child.as_mut() {
                let size = child.layout(
                    SizeConstraint {
                        min: vec2f(width, height),
                        max: vec2f(width, height),
                    },
                    ctx,
                    app,
                );
                self.size = Some(size);
            }
        }
        self.size.unwrap_or(vec2f(width, height))
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        if let Some(child) = self.child.as_mut() {
            child.after_layout(ctx, app);
        }
    }

    fn paint(
        &mut self,
        origin: pathfinder_geometry::vector::Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        if let Some(child) = self.child.as_mut() {
            child.paint(origin, ctx, app);
        }
        // Only repaint when there are active agents or animating characters
        let has_activity = matches!(self.snapshot.status, CLIAgentSessionStatus::InProgress)
            || self.sub_agents.iter().any(|sa| !matches!(sa.state, SubAgentState::Working));
        if has_activity {
            ctx.repaint_after(Duration::from_millis(240));
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| child.dispatch_event(event, ctx, app))
    }

    fn size(&self) -> Option<pathfinder_geometry::vector::Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

impl Entity for PixelAgentsPanelView {
    type Event = ();
}

impl View for PixelAgentsPanelView {
    fn ui_name() -> &'static str {
        "PixelAgentsPanelView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let animation_frame = (millis / 240) as usize;

        let snapshots = self.visible_snapshots();
        let content = if snapshots.is_empty() {
            Self::render_empty(app)
        } else {
            Flex::column()
                .with_spacing(8.)
                .with_children(snapshots.into_iter().map(|snapshot| {
                    Self::render_snapshot(snapshot, &self.sub_agents, app, animation_frame)
                }))
                .finish()
        };

        Shrinkable::new(
            1.,
            ConstrainedBox::new(
                Container::new(content)
                    .with_uniform_padding(PANEL_PADDING)
                    .finish(),
            )
            .with_min_height(0.)
            .with_width(f32::INFINITY)
            .finish(),
        )
        .finish()
    }
}

fn agent_title(snapshot: &PixelAgentSnapshot) -> String {
    let title = snapshot
        .session_context
        .display_title()
        .unwrap_or_else(|| snapshot.agent.display_name().to_owned());
    // Filter out XML-like notification content that may leak into the query field
    if title.trim_start().starts_with('<') {
        return snapshot.agent.display_name().to_owned();
    }
    title
}

fn agent_detail(snapshot: &PixelAgentSnapshot) -> String {
    let status = status_label(&snapshot.status);
    if let Some(tool_name) = snapshot.session_context.tool_name.as_deref() {
        return format!("{status} · {tool_name}");
    }
    if let Some(project) = snapshot.session_context.project.as_deref() {
        return format!("{status} · {project}");
    }
    status.to_owned()
}

fn status_label(status: &CLIAgentSessionStatus) -> &'static str {
    match status {
        CLIAgentSessionStatus::InProgress => "Working",
        CLIAgentSessionStatus::Success => "Done",
        CLIAgentSessionStatus::Blocked { .. } => "Needs input",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_matches_session_status() {
        assert_eq!(status_label(&CLIAgentSessionStatus::InProgress), "Working");
        assert_eq!(status_label(&CLIAgentSessionStatus::Success), "Done");
        assert_eq!(
            status_label(&CLIAgentSessionStatus::Blocked { message: None }),
            "Needs input"
        );
    }

    #[test]
    fn process_jsonl_tracks_task_tool_lifecycle() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();
        // Two Task tool_use calls
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"task-1","name":"Task","input":{}},{"type":"tool_use","id":"task-2","name":"Task","input":{}}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 2);
        assert!(active.contains("task-1"));
        assert!(active.contains("task-2"));
        // One tool_result completes task-1
        process_jsonl_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"task-1"}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 1);
        assert!(!active.contains("task-1"));
        assert!(active.contains("task-2"));
    }

    #[test]
    fn process_jsonl_ignores_non_agent_tools() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"bash-1","name":"Bash","input":{}}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn process_jsonl_tracks_taskcreate_tool() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tc-1","name":"TaskCreate","input":{}}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 1);
        assert!(active.contains("tc-1"));
        // tool_result completes it
        process_jsonl_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tc-1"}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn process_jsonl_tracks_workflow_tool() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"wf-1","name":"Workflow","input":{}}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 1);
        assert!(active.contains("wf-1"));
        // tool_result completes it
        process_jsonl_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"wf-1"}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn process_jsonl_tracks_agent_tool() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"agent-1","name":"Agent","input":{}}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 1);
        assert!(active.contains("agent-1"));
    }

    #[test]
    fn process_jsonl_tracks_background_agent_lifecycle() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();

        // 1. Tool use triggers active_tool_ids insertion
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"call-1","name":"Agent","input":{}}]}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 1);
        assert_eq!(active_agents.len(), 0);

        // 2. tool_result containing toolUseResult.agentId removes tool ID and inserts agent ID
        process_jsonl_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"call-1"}]},"toolUseResult":{"isAsync":true,"agentId":"agent-id-123"}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 0);
        assert_eq!(active_agents.len(), 1);
        assert!(active_agents.contains("agent-id-123"));

        // 3. task-notification attachment removes agent ID
        process_jsonl_line(
            r#"{"type":"attachment","attachment":{"type":"task-notification","agentId":"agent-id-123","content":""}}"#,
            &mut active,
            &mut active_agents,
        );
        assert_eq!(active.len(), 0);
        assert_eq!(active_agents.len(), 0);
    }

    #[test]
    fn process_jsonl_ignores_malformed_lines() {
        let mut active = HashSet::new();
        let mut active_agents = HashSet::new();
        process_jsonl_line("not json", &mut active, &mut active_agents);
        process_jsonl_line("{}", &mut active, &mut active_agents);
        process_jsonl_line(r#"{"type":"assistant"}"#, &mut active, &mut active_agents);
        assert_eq!(active.len(), 0);
        assert_eq!(active_agents.len(), 0);
    }

    #[test]
    fn agent_detail_prefers_tool_then_project() {
        let mut snapshot = PixelAgentSnapshot {
            terminal_view_id: EntityId::new(),
            agent: CLIAgent::Codex,
            status: CLIAgentSessionStatus::InProgress,
            session_context: CLIAgentSessionContext::default(),
        };
        snapshot.session_context.project = Some("sterm".to_owned());
        assert_eq!(agent_detail(&snapshot), "Working · sterm");
        snapshot.session_context.tool_name = Some("Read".to_owned());
        assert_eq!(agent_detail(&snapshot), "Working · Read");
    }

    #[test]
    fn transcript_cache_tracks_session_idle() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");

        // Write assistant line → not idle
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, r#"{{"type":"assistant","message":{{"content":[]}}}}"#).unwrap();
        }
        let mut cache = TranscriptCache::new(path.to_string_lossy().into_owned());
        assert!(!cache.session_idle, "should not be idle after assistant line");

        // Append last-prompt → idle
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f, r#"{{"type":"last-prompt","leafUuid":"abc"}}"#).unwrap();
        }
        cache.read_incremental();
        assert!(cache.session_idle, "should be idle after last-prompt line");

        // Append new assistant line → not idle again
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f, r#"{{"type":"assistant","message":{{"content":[]}}}}"#).unwrap();
        }
        cache.read_incremental();
        assert!(!cache.session_idle, "should not be idle after new assistant line");
    }

    #[test]
    fn classify_transcript_line_detects_last_prompt() {
        let mut idle = false;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();
        tools.insert("stale-tool".to_owned());
        agents.insert("stale-agent".to_owned());

        classify_transcript_line(
            r#"{"type":"last-prompt","leafUuid":"abc"}"#,
            &mut idle,
            &mut tools,
            &mut agents,
        );
        assert!(idle, "should detect idle from last-prompt");
        assert!(tools.is_empty(), "should clear tool IDs");
        assert!(agents.is_empty(), "should clear agent IDs");
    }

    #[test]
    fn classify_transcript_line_detects_turn_duration_via_subtype() {
        let mut idle = false;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();
        tools.insert("stale-tool".to_owned());
        agents.insert("bg-agent".to_owned());

        classify_transcript_line(
            r#"{"type":"summary","subtype":"turn_duration","duration_ms":1500}"#,
            &mut idle,
            &mut tools,
            &mut agents,
        );
        assert!(idle, "should detect idle from turn_duration");
        assert!(tools.is_empty(), "should clear foreground tool IDs");
        assert!(!agents.is_empty(), "should preserve background agent IDs");
    }

    #[test]
    fn classify_transcript_line_detects_assistant() {
        let mut idle = true;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();

        classify_transcript_line(
            r#"{"type":"assistant","message":{"content":[]}}"#,
            &mut idle,
            &mut tools,
            &mut agents,
        );
        assert!(!idle, "should detect active from assistant");
    }

    #[test]
    fn classify_transcript_line_ignores_unknown_types() {
        let mut idle = false;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();
        tools.insert("keep-me".to_owned());

        classify_transcript_line(
            r#"{"type":"user","message":{"content":[]}}"#,
            &mut idle,
            &mut tools,
            &mut agents,
        );
        assert!(!idle, "should stay not-idle for user messages");
        assert_eq!(tools.len(), 1, "should not clear tools for user messages");
    }

    #[test]
    fn classify_transcript_line_handles_malformed_json() {
        let mut idle = false;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();

        classify_transcript_line("not json at all", &mut idle, &mut tools, &mut agents);
        assert!(!idle, "should not crash on malformed JSON");
    }

    #[test]
    fn classify_transcript_line_handles_json_with_spaces() {
        // JSON with spaces after colons — the old string matching would miss this
        let mut idle = false;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();
        tools.insert("t1".to_owned());

        classify_transcript_line(
            r#"{"type": "last-prompt", "leafUuid": "abc"}"#,
            &mut idle,
            &mut tools,
            &mut agents,
        );
        assert!(idle, "should detect last-prompt even with spaces in JSON");
        assert!(tools.is_empty(), "should clear tools");
    }

    #[test]
    fn classify_transcript_line_handles_turn_duration_with_spaces() {
        let mut idle = false;
        let mut tools = HashSet::new();
        let mut agents = HashSet::new();
        tools.insert("t1".to_owned());

        classify_transcript_line(
            r#"{"type": "summary", "subtype": "turn_duration", "duration_ms": 1500}"#,
            &mut idle,
            &mut tools,
            &mut agents,
        );
        assert!(idle);
        assert!(tools.is_empty());
    }
}
