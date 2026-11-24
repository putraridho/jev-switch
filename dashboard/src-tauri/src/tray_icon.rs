//! The tray icon's pure rasteriser (DESIGN 9.11). T15 writes the body.

use jev_usage_core::assess::Severity;

/// What the icon shows: a ring gauge in 5 % steps and the level's shape.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct IconSpec {
    /// The usage in 5 % steps, 0..=20.
    pub used_step: u8,
    /// The level, drawn as a shape and not only a colour.
    pub level: Severity,
}

/// The icon as RGBA bytes, `size * size * 4` long; integer arithmetic only.
pub fn render(spec: IconSpec, size: u32) -> Vec<u8> {
    let _ = (spec, size);
    Vec::new()
}
