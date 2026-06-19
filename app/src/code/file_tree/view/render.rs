use warpui::elements::{DraggableState, MouseStateHandle};
use warpui::{AppContext, SingletonEntity};

use super::FileTreeItem;
use crate::code::icon_from_file_path;
use crate::settings::CodeSettings;
use crate::ui_components::item_highlight::ImageOrIcon;
use crate::{appearance::Appearance, ui_components::icons::Icon};

fn folder_color_from_name(name: &str) -> warp_core::ui::theme::Fill {
    let name_lower = name.to_lowercase();
    let color = match name_lower.as_str() {
        ".github" | ".git" | ".gitnexus" | ".idea" | ".lbdb" | ".npm" | "playwright-report"
        | "astroinfo" => warpui::color::ColorU::new(140, 148, 156, 255), // Gray
        ".vscode" | "vscode" | "src" | "lib" | "public" | "dist" => {
            warpui::color::ColorU::new(38, 139, 210, 255)
        } // Blue/Cyan
        "node_modules" | ".venv" | "venv" | "env" | "vendor" => {
            warpui::color::ColorU::new(133, 153, 0, 255)
        } // Green
        ".gemini" | ".agent" | ".agents" | ".claude" | "plans" | "specs" => {
            warpui::color::ColorU::new(220, 50, 47, 255)
        } // Red
        "functions" | "memory" | "data" | "android" | "firebase" | ".astro" | ".cloudflare" => {
            warpui::color::ColorU::new(181, 137, 0, 255)
        } // Gold/Orange
        _ => {
            // Hash-based color selection so any folder has a unique color
            let mut hash = 5381u32;
            for c in name.chars() {
                hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
            }
            let colors = [
                warpui::color::ColorU::new(220, 50, 47, 255),   // Red
                warpui::color::ColorU::new(203, 75, 22, 255),   // Orange
                warpui::color::ColorU::new(181, 137, 0, 255),   // Yellow
                warpui::color::ColorU::new(133, 153, 0, 255),   // Green
                warpui::color::ColorU::new(42, 161, 152, 255),  // Cyan
                warpui::color::ColorU::new(38, 139, 210, 255),  // Blue
                warpui::color::ColorU::new(108, 113, 196, 255), // Violet
                warpui::color::ColorU::new(211, 54, 130, 255),  // Magenta
            ];
            colors[(hash % colors.len() as u32) as usize]
        }
    };
    warp_core::ui::theme::Fill::Solid(color)
}

impl FileTreeItem {
    pub(super) fn to_render_state(
        &self,
        is_expanded: Option<bool>,
        app: &AppContext,
    ) -> RenderState {
        let appearance = Appearance::as_ref(app);
        match self {
            FileTreeItem::File {
                metadata,
                mouse_state_handle,
                depth,
                draggable_state,
            } => {
                let display_name = metadata
                    .path
                    .file_name()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| String::from("File"));

                let icon_from_file_path =
                    icon_from_file_path(metadata.path.as_str(), appearance).map(ImageOrIcon::Image);

                RenderState {
                    display_name,
                    icon: icon_from_file_path.unwrap_or(ImageOrIcon::Icon(Icon::File)),
                    is_expanded,
                    depth: *depth,
                    mouse_state: mouse_state_handle.clone(),
                    draggable_state: draggable_state.clone(),
                    is_ignored: metadata.ignored,
                    icon_color_override: None,
                }
            }
            FileTreeItem::DirectoryHeader {
                directory,
                mouse_state_handle,
                depth,
                draggable_state,
            } => {
                let display_name = directory
                    .path
                    .file_name()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| String::from("Folder"));

                let enable_folder_colors = *CodeSettings::as_ref(app).enable_folder_colors;
                let icon_color_override = if enable_folder_colors {
                    Some(folder_color_from_name(&display_name))
                } else {
                    None
                };

                RenderState {
                    display_name,
                    icon: ImageOrIcon::Icon(Icon::Folder),
                    is_expanded,
                    depth: *depth,
                    mouse_state: mouse_state_handle.clone(),
                    draggable_state: draggable_state.clone(),
                    is_ignored: directory.ignored,
                    icon_color_override,
                }
            }
        }
    }
}

pub(super) struct RenderState {
    pub display_name: String,
    pub icon: ImageOrIcon,
    pub is_expanded: Option<bool>,
    pub depth: usize,
    pub mouse_state: MouseStateHandle,
    pub draggable_state: DraggableState,
    pub is_ignored: bool,
    pub icon_color_override: Option<warp_core::ui::theme::Fill>,
}
