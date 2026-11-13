//! Colours and sizes. The graph palette is PatchMatrix's with the owner's high-contrast colours
//! (their fork, commit a417c10: "more contrast too"); the rest follows nuklear's dark defaults
//! that PatchMatrix's widgets had.

use crate::gfx::{Color, rgb};
use pbx_jack::PortKind;

// The graph (PatchMatrix, owner's colours).
pub const GRID_BG: Color = rgb(0, 0, 0);
pub const GRID_LINE: Color = rgb(40, 40, 40);
pub const HILIGHT: Color = rgb(255, 165, 48);
pub const BORDER: Color = rgb(0, 162, 255);
// PatchMatrix's wire_color (191,71,200) drew its mixer dials only; wires use HANDLE.
pub const HANDLE: Color = rgb(191, 71, 255);
pub const TOGGLE: Color = rgb(151, 255, 71);

// nuklear's dark style, as PatchMatrix's nodes and buttons used it.
pub const NODE_FILL: Color = rgb(40, 40, 40);
pub const MATRIX_FILL: Color = rgb(50, 50, 50);
pub const TEXT: Color = rgb(175, 175, 175);
pub const TEXT_BRIGHT: Color = rgb(225, 225, 225);
pub const TEXT_DIM: Color = rgb(110, 110, 110);

// Window chrome.
pub const BAR_BG: Color = rgb(24, 24, 26);
pub const PANEL_BG: Color = rgb(30, 30, 32);
pub const PANEL_LINE: Color = rgb(52, 52, 56);
pub const BUTTON: Color = rgb(45, 45, 45);
pub const BUTTON_HOVER: Color = rgb(58, 58, 60);
pub const MENU_BG: Color = rgb(36, 36, 38);
pub const TOOLTIP_BG: Color = rgb(20, 20, 22);
pub const OK: Color = rgb(151, 255, 71);
pub const WARN: Color = rgb(255, 165, 48);
pub const BAD: Color = rgb(255, 80, 80);

/// Port-type accents (tabs, sidebar dots).
pub fn kind_color(k: PortKind) -> Color {
    match k {
        PortKind::Audio => rgb(0, 162, 255),
        PortKind::Midi => rgb(255, 165, 48),
        PortKind::Cv => rgb(151, 255, 71),
        PortKind::Osc => rgb(191, 71, 255),
        PortKind::Other => rgb(150, 150, 150),
    }
}

// Sizes, in points (× the interface scale for pixels; × zoom too on the canvas).
pub const FONT: f32 = 13.0;
pub const FONT_SMALL: f32 = 11.0;
pub const TOPBAR_H: f32 = 34.0;
pub const SIDEBAR_W: f32 = 250.0;
pub const NODE_W: f32 = 200.0;
pub const NODE_H: f32 = 24.0;
pub const NODE_RADIUS: f32 = 4.0;
pub const CELL: f32 = 16.0;
/// A port row in an expanded node, and the padding around the rows.
pub const PORT_ROW: f32 = 16.0;
pub const BODY_PAD: f32 = 4.0;
pub const BODY_FILL: crate::gfx::Color = rgb(28, 28, 30);
pub const HANDLE_R: f32 = 4.0;
pub const GRID: f32 = 28.0;
pub const WIRE_BEND: f32 = 50.0;
