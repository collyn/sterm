use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::color::internal_colors;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
    Flex, Hoverable, Image, MainAxisAlignment, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Point, Radius, Shrinkable, Stack, Text,
};
use warpui::r#async::Timer;
use warpui::event::DispatchedEvent;
use warpui::image_cache::CacheOption;
use warpui::platform::Cursor;
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
struct TranscriptCache {
    path: String,
    offset: u64,
    active_tool_ids: HashSet<String>,
    /// Peak number of concurrent subagents seen so far.
    peak_active: usize,
    last_poll: Instant,
}

const TRANSCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(500);

impl TranscriptCache {
    fn new(path: String) -> Self {
        let mut cache = Self {
            path,
            offset: 0,
            active_tool_ids: HashSet::new(),
            peak_active: 0,
            last_poll: Instant::now() - TRANSCRIPT_POLL_INTERVAL,
        };
        // Initial full read
        cache.read_incremental();
        cache
    }

    fn active_count(&self) -> usize {
        // Return peak count so characters don't disappear when subagents complete
        self.peak_active
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
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let reader = BufReader::new(file);
        let mut new_offset = self.offset;
        for line in reader.lines().map_while(Result::ok) {
            new_offset += line.len() as u64 + 1; // +1 for newline
            process_jsonl_line(&line, &mut self.active_tool_ids);
            // Track peak concurrent subagents
            let current = self.active_tool_ids.len();
            if current > self.peak_active {
                self.peak_active = current;
            }
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

/// Processes a single JSONL line, updating the set of active tool_use IDs.
fn process_jsonl_line(line: &str, active_tool_ids: &mut HashSet<String>) {
    let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let Some(record_type) = record.get("type").and_then(|v| v.as_str()) else {
        return;
    };
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
                let is_tool_use = block.get("type").and_then(|t| t.as_str()) == Some("tool_use");
                let is_subagent = block
                    .get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| matches!(n, "Task" | "Agent"));
                if is_tool_use && is_subagent {
                    if let Some(id) = block.get("id").and_then(|id| id.as_str()) {
                        active_tool_ids.insert(id.to_owned());
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
    transcript_caches: RefCell<HashMap<EntityId, TranscriptCache>>,
    /// Whether an animation timer is currently running.
    animation_timer_running: bool,
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
            transcript_caches: RefCell::new(HashMap::new()),
            animation_timer_running: false,
        }
    }

    /// Returns true if any tracked agent is currently in progress.
    fn has_active_agents(&self) -> bool {
        self.sessions
            .values()
            .any(|s| matches!(s.status, CLIAgentSessionStatus::InProgress))
    }

    /// Spawns a periodic timer that re-renders the view every 240ms while agents are active.
    fn ensure_animation_timer(&mut self, ctx: &mut ViewContext<Self>) {
        if self.animation_timer_running {
            return;
        }
        self.animation_timer_running = true;
        ctx.spawn(
            async {
                loop {
                    Timer::after(Duration::from_millis(240)).await;
                }
            },
            |me, _, ctx| {
                me.animation_timer_running = false;
                if me.has_active_agents() {
                    me.ensure_animation_timer(ctx);
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
            ctx.notify();
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
                self.sessions.insert(
                    *terminal_view_id,
                    PixelAgentSnapshot {
                        terminal_view_id: *terminal_view_id,
                        agent: *agent,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                    },
                );
                ctx.notify();
                self.ensure_animation_timer(ctx);
            }
            CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status,
                session_context,
            } => {
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
                self.sessions.remove(terminal_view_id);
                self.transcript_caches.borrow_mut().remove(terminal_view_id);
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
                    // Update transcript cache if transcript path is available
                    if let Some(path) = session.session_context.transcript_path.as_deref() {
                        let mut caches = self.transcript_caches.borrow_mut();
                        let needs_new = match caches.get(terminal_view_id) {
                            Some(c) => c.path != path,
                            None => true,
                        };
                        if needs_new {
                            caches.insert(*terminal_view_id, TranscriptCache::new(path.to_owned()));
                        }
                    }
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
                    self.sessions.insert(
                        *terminal_view_id,
                        PixelAgentSnapshot {
                            terminal_view_id: *terminal_view_id,
                            agent: session.agent,
                            status: session.status.clone(),
                            session_context: session.session_context.clone(),
                        },
                    );
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

    fn worker_count(&self, snapshot: &PixelAgentSnapshot) -> usize {
        // Try cached transcript data
        if let Some(path) = snapshot.session_context.transcript_path.as_deref() {
            let mut caches = self.transcript_caches.borrow_mut();
            let cache = caches
                .entry(snapshot.terminal_view_id)
                .or_insert_with(|| TranscriptCache::new(path.to_owned()));
            // Ensure path is up-to-date
            if cache.path != path {
                *cache = TranscriptCache::new(path.to_owned());
            }
            cache.poll();
            let count = cache.active_count();
            if count > 0 {
                return count.clamp(1, 6);
            }
        }
        // Heuristic fallback: parse query/title for agent count hints
        let haystack = format!(
            "{} {} {}",
            agent_title(snapshot),
            snapshot
                .session_context
                .summary
                .as_deref()
                .unwrap_or_default(),
            snapshot
                .session_context
                .query
                .as_deref()
                .unwrap_or_default()
        )
        .to_lowercase();
        for token in haystack.split(|c: char| !c.is_ascii_alphanumeric()) {
            if let Ok(count) = token.parse::<usize>() {
                if (2..=6).contains(&count)
                    && (haystack.contains("agent") || haystack.contains("task"))
                {
                    return count;
                }
            }
        }
        if haystack.contains("multi agent") || haystack.contains("multi-agent") {
            return 4;
        }
        1
    }

    fn render_snapshot(
        snapshot: &PixelAgentSnapshot,
        app: &AppContext,
        workers: usize,
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
        let scene = render_pixel_agents_scene(snapshot, workers, animation_frame);

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
    workers: usize,
    animation_frame: usize,
) -> Box<dyn Element> {
    PixelAgentsSceneElement::new(snapshot.clone(), workers, animation_frame).finish()
}

fn render_pixel_agents_scene_at_scale(
    workers: usize,
    animation_frame: usize,
    scale: f32,
) -> Box<dyn Element> {
    let mut s = Stack::new();

    render_office_tiles(&mut s, scale);
    render_wall_furniture(&mut s, scale);
    render_central_workstations(&mut s, workers, animation_frame, scale);
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
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DOUBLE_BOOKSHELF/DOUBLE_BOOKSHELF.png",
        20.,
        10.,
        32.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DOUBLE_BOOKSHELF/DOUBLE_BOOKSHELF.png",
        54.,
        10.,
        32.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DOUBLE_BOOKSHELF/DOUBLE_BOOKSHELF.png",
        142.,
        10.,
        32.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DOUBLE_BOOKSHELF/DOUBLE_BOOKSHELF.png",
        174.,
        10.,
        32.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/DESK/DESK_FRONT.png",
        42.,
        42.,
        48.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/PLANT/PLANT.png",
        48.,
        26.,
        16.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LAPTOP/LAPTOP_FRONT_OFF.png",
        68.,
        28.,
        16.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/WHITEBOARD/WHITEBOARD.png",
        150.,
        38.,
        32.,
        32.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LARGE_PLANT/LARGE_PLANT.png",
        8.,
        32.,
        32.,
        48.,
        scale,
    );
    add(
        stack,
        "bundled/pixel-agents/assets/furniture/LARGE_PLANT/LARGE_PLANT.png",
        188.,
        34.,
        32.,
        48.,
        scale,
    );
}

fn render_central_workstations(
    stack: &mut Stack,
    workers: usize,
    animation_frame: usize,
    scale: f32,
) {
    let desk_x = 90.;
    let desk_y = 78.;
    let row_gap = 46.;

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

    for index in 0..workers.min(8) {
        let row = index / 2;
        let left_side = index % 2 == 0;
        let x = if left_side {
            desk_x - 13.
        } else {
            desk_x + 29.
        };
        let y = desk_y + row as f32 * row_gap + 6.;
        add_char_frame(stack, index, left_side, animation_frame, x, y, scale);
    }
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

struct PixelAgentsSceneElement {
    snapshot: PixelAgentSnapshot,
    workers: usize,
    animation_frame: usize,
    child: Option<Box<dyn Element>>,
    size: Option<pathfinder_geometry::vector::Vector2F>,
    origin: Option<Point>,
    /// Cache key: (scale_bits, workers, is_working, animation_frame). Scene is rebuilt only when this changes.
    cache_key: Option<(u32, usize, bool, usize)>,
}

impl PixelAgentsSceneElement {
    fn new(snapshot: PixelAgentSnapshot, workers: usize, animation_frame: usize) -> Self {
        Self {
            snapshot,
            workers,
            animation_frame,
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
            self.workers,
            is_working,
            self.animation_frame % 2,
        );

        // Only rebuild scene tree when key changes
        let needs_rebuild = self.cache_key != Some(key);

        if needs_rebuild {
            let mut child =
                render_pixel_agents_scene_at_scale(self.workers, self.animation_frame, scale);
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
        if matches!(self.snapshot.status, CLIAgentSessionStatus::InProgress) {
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
        // NOTE: Transcript caches are polled lazily in worker_count() with a 500ms
        // throttle, so no explicit poll is needed here.

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
                    let workers = self.worker_count(snapshot);
                    Self::render_snapshot(snapshot, app, workers, animation_frame)
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
        // Two Task tool_use calls
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"task-1","name":"Task","input":{}},{"type":"tool_use","id":"task-2","name":"Task","input":{}}]}}"#,
            &mut active,
        );
        assert_eq!(active.len(), 2);
        assert!(active.contains("task-1"));
        assert!(active.contains("task-2"));
        // One tool_result completes task-1
        process_jsonl_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"task-1"}]}}"#,
            &mut active,
        );
        assert_eq!(active.len(), 1);
        assert!(!active.contains("task-1"));
        assert!(active.contains("task-2"));
    }

    #[test]
    fn process_jsonl_ignores_non_agent_tools() {
        let mut active = HashSet::new();
        process_jsonl_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"bash-1","name":"Bash","input":{}}]}}"#,
            &mut active,
        );
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn process_jsonl_ignores_malformed_lines() {
        let mut active = HashSet::new();
        process_jsonl_line("not json", &mut active);
        process_jsonl_line("{}", &mut active);
        process_jsonl_line(r#"{"type":"assistant"}"#, &mut active);
        assert_eq!(active.len(), 0);
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
}
