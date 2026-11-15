//! Icons of the interface: glyphs of the Lucide icon font (bundled,
//! `assets/lucide.ttf`, ISC licence), set inline in text so that they take
//! its colour and size. The font holds every Lucide icon: a new one is a
//! constant here, its code point from `font/codepoints.json` of the
//! `lucide-static` package (the same version, 1.57.0).

pub const SELECT: &str = "\u{e1c3}"; // mouse-pointer-2
pub const PIN: &str = "\u{e111}"; // map-pin
pub const UNDO: &str = "\u{e2a1}"; // undo-2
pub const REDO: &str = "\u{e2a0}"; // redo-2
pub const BOTH_VIEWS: &str = "\u{e098}"; // columns-2
pub const VIEW_3D: &str = "\u{e061}"; // box
pub const SLICE: &str = "\u{e258}"; // scan-line
pub const COLOUR: &str = "\u{e1dd}"; // palette
pub const HIDE: &str = "\u{e0bb}"; // eye-off
pub const CLIP: &str = "\u{e14e}"; // scissors
pub const EFFECTS: &str = "\u{e412}"; // sparkles
pub const CAMERA: &str = "\u{e064}"; // camera
pub const TRACING: &str = "\u{e0a9}"; // cpu
pub const SAVE: &str = "\u{e14d}"; // save
pub const PICKED: &str = "\u{e0ac}"; // crosshair
pub const GO_TO: &str = "\u{e1db}"; // locate-fixed
pub const REMOVE: &str = "\u{e1b2}"; // x
pub const DELETE: &str = "\u{e18e}"; // trash-2
pub const LIGHT: &str = "\u{e178}"; // sun
pub const DARK: &str = "\u{e11e}"; // moon
pub const SYSTEM: &str = "\u{e11d}"; // monitor
pub const SHOW: &str = "\u{e0ba}"; // eye
pub const ISOLATE: &str = "\u{e29e}"; // focus
pub const GHOST: &str = "\u{e20e}"; // ghost
pub const LEVELS: &str = "\u{e52a}"; // layers-2
pub const FRAME: &str = "\u{e257}"; // scan
pub const COPY: &str = "\u{e09e}"; // copy
pub const SEARCH: &str = "\u{e151}"; // search
pub const PARENT: &str = "\u{e04a}"; // arrow-up
pub const BACK: &str = "\u{e048}"; // arrow-left
pub const FORWARD: &str = "\u{e049}"; // arrow-right
pub const UNIVERSE: &str = "\u{e2d0}"; // boxes
pub const CELL: &str = "\u{e524}"; // cuboid
pub const MATERIAL: &str = "\u{e59c}"; // blend
pub const OPEN: &str = "\u{e537}"; // scan-search
pub const INVERT: &str = "\u{e402}"; // circle-slash
pub const INFO: &str = "\u{e0f9}"; // info
pub const HISTORY: &str = "\u{e1f5}"; // history
pub const KEYS: &str = "\u{e284}"; // keyboard
pub const TARGET: &str = "\u{e180}"; // target
pub const CUBOID_ALT: &str = "\u{e061}"; // box
pub const OPENED: &str = "\u{e06d}"; // chevron-down
pub const CLOSED: &str = "\u{e06f}"; // chevron-right
pub const ASCENDING: &str = "\u{e04c}"; // arrow-up-narrow-wide
pub const DESCENDING: &str = "\u{e047}"; // arrow-down-wide-narrow
pub const FIELD: &str = "\u{e186}"; // thermometer
pub const FOLDER_OPEN: &str = "\u{e247}"; // folder-open
pub const MESH: &str = "\u{e0e9}"; // grid-3x3
pub const MAXIMIZE: &str = "\u{e113}"; // maximize-2
pub const RESTORE: &str = "\u{e11b}"; // minimize-2
pub const FOLD_LEFT: &str = "\u{e21c}"; // panel-left-close
pub const UNFOLD_LEFT: &str = "\u{e21d}"; // panel-left-open
pub const FOLD_RIGHT: &str = "\u{e432}"; // panel-right-close
pub const UNFOLD_RIGHT: &str = "\u{e434}"; // panel-right-open
pub const EXPLORE: &str = "\u{e09b}"; // compass
pub const CHECK: &str = "\u{e4aa}"; // search-check
pub const RESULTS: &str = "\u{e2a5}"; // chart-line
pub const VOID: &str = "\u{e4b0}"; // circle-dashed
pub const AIR: &str = "\u{e1b0}"; // wind
pub const RULER: &str = "\u{e14b}"; // ruler

/// An icon followed by a label, as buttons and headers show them.
pub fn with(icon: &str, label: &str) -> String {
    format!("{icon}  {label}")
}
