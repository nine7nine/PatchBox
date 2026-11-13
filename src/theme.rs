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
/// The window's top (title bar and toolbar, one dark grey), its control groups, the sidebar and
/// pages (the old toolbar grey), the LCD.
pub const BAR_BG: Color = rgb(19, 20, 22);
pub const TITLE_BG: Color = BAR_BG;
pub const GROUP_BG: Color = rgb(13, 14, 16);
pub const LCD_BG: Color = rgb(8, 9, 10);
pub const PANEL_BG: Color = rgb(24, 24, 26);
/// Luluda's buttons: a dark face, a border tinted by what the button does.
pub const BTN_FACE: Color = rgb(34, 36, 39);
pub const TINT_GREEN: Color = rgb(0x00, 0xe6, 0x76);
pub const TINT_AMBER: Color = rgb(0xff, 0xa7, 0x26);
pub const TINT_BLUE: Color = rgb(0x64, 0xb5, 0xf6);
pub const TINT_PURPLE: Color = rgb(0xba, 0x68, 0xc8);
pub const TINT_TEAL: Color = rgb(0x4d, 0xd0, 0xe1);
pub const TINT_RED: Color = rgb(0xef, 0x53, 0x50);
pub const TINT_GREY: Color = rgb(0x9e, 0x9e, 0xa4);
pub const PANEL_LINE: Color = rgb(52, 52, 56);
pub const BUTTON: Color = rgb(45, 45, 45);
pub const BUTTON_HOVER: Color = rgb(58, 58, 60);
pub const MENU_BG: Color = rgb(36, 36, 38);
pub const TOOLTIP_BG: Color = rgb(20, 20, 22);
pub const OK: Color = rgb(151, 255, 71);
pub const WARN: Color = rgb(255, 165, 48);
pub const BAD: Color = rgb(255, 80, 80);

/// Port-type colours: wires, port strips and dots, tabs. Luluda's palette (the owner's Element
/// fork: src/ui/grapheditorcomponent.cpp), so the graph reads by type at a glance.
pub fn kind_color(k: PortKind) -> Color {
    match k {
        PortKind::Audio => rgb(0x00, 0xe6, 0x76),
        PortKind::Midi => rgb(0xff, 0xa7, 0x26),
        PortKind::Cv => rgb(0xba, 0x68, 0xc8),
        PortKind::Osc => rgb(0x4d, 0xd0, 0xe1),
        PortKind::Video => rgb(0xff, 0x5a, 0x8c),
        PortKind::Other => rgb(0xbd, 0xbd, 0xbd),
    }
}

// Luluda's blocks: a light fill, black text, a border in the node type's colour with a dark
// inset line inside it (src/ui/block.cpp).
pub const BLOCK_FILL: Color = rgb(0xaa, 0xaa, 0xaa);
pub const BLOCK_FILL_HOT: Color = rgb(0xbe, 0xbe, 0xbe);
pub const BLOCK_BODY: Color = rgb(0x96, 0x96, 0x98);
pub const BLOCK_TEXT: Color = rgb(0x00, 0x00, 0x00);
pub const BLOCK_INSET: Color = [0.0, 0.0, 0.0, 0.6];
/// Node types: hardware audio, MIDI devices, the PipeWire bridge, applications.
pub const NODE_HW: Color = rgb(0x00, 0xe6, 0x76);
pub const NODE_MIDI: Color = rgb(0xff, 0xa7, 0x26);
pub const NODE_BRIDGE: Color = rgb(0x4d, 0xd0, 0xe1);
pub const NODE_APP: Color = rgb(0x95, 0x75, 0xcd);
/// Wires: alpha at rest; hovered and lit wires go brighter and opaque.
pub const WIRE_ALPHA: f32 = 0.78;

// Sizes, in points (× the interface scale for pixels; × zoom too on the canvas).
pub const FONT: f32 = 13.0;
pub const FONT_SMALL: f32 = 11.0;
pub const TOPBAR_H: f32 = 34.0;
/// The title bar (desktop scale) and the toolbar row under it (interface scale), points.
pub const TITLE_H: f32 = 30.0;
pub const TOOLBAR_H: f32 = 46.0;
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
