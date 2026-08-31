//! Deterministic procedural graphics, animation, and the sprite review harness.
//!
//! Purpose: own the reusable artifact-generation engine that turns semantic
//! `CharacterSpec` inputs into indexed sprites and a self-contained review
//! page. The art layer is a pure boundary: it reads specifications and
//! produces images, owning no campaign state and encoding no domain rules.
//! Everything is integer arithmetic, so a given spec renders bit-identically
//! on every platform and build profile.
//! Owns: `Palette`/`Ramp`/`Material`, `Skeleton`/`Pose`, `Surface`/
//! `Canvas`, `SpriteSheet`, `ArtReview`/`ArtReviewReport`, and the
//! `render_*` / `encode_*` pipelines.
//! Reads: `CharacterSpec` and review config only.
//! Mutates: nothing persistent (canvases and sheets are produced, not
//! retained in `AppState`).
//! Does not own: campaign state, simulation rules, or HTML campaign
//! projection.
//! Canonical operations: `CharacterSpec` → `Canvas` (indexed) → PNG bytes →
//! data URI → `ArtReview` → `ArtReviewReport` → HTML; `build_art_review`
//! is the staged publication entry point.
//! Relevant invariants: one-way `Spec → PNG` (no round-trip claim);
//! palette length bounded; generated HTML/PNG are derived artifacts, not
//! sources; review finds drift without mutating specs.
//! The pipeline is layered:
//! ```text
//! specification -> palette and materials -> skeleton and pose -> shaded surface
//!   -> indexed canvas -> sheet -> automated review -> harness page
//! ```
//! Drawing never writes color directly. Primitives write material, light, and
//! depth into a [`surface::Surface`]; resolution maps light onto
//! hue-shifted ramps with ordered dithering and selective outlines. Form and
//! color therefore stay independent, which is what makes relighting,
//! recoloring, and restyling cheap.
//! ```
//! use civic_dynasty::art::{ArtReviewConfig, build_art_review, render_art_review_html};
//!
//! let review = build_art_review(ArtReviewConfig {
//!     seeds: 1,
//!     ..ArtReviewConfig::default()
//! })
//! .expect("valid art review config");
//! let page = render_art_review_html(&review);
//!
//! assert!(page.starts_with("<!DOCTYPE html>"));
//! ```

pub mod anim;
pub mod canvas;
pub mod color;
pub mod harness;
pub mod lint;
pub mod math;
pub mod png;
pub mod rig;
pub mod shape;
pub mod sprite;
pub mod surface;

pub use anim::{Clip, HumanClip, Keyframe, humanoid_clip, humanoid_clip_library};
pub use canvas::{Canvas, Rect};
pub use color::{Hsl, Palette, Ramp, RampHandle, Rgb8, ShadeProfile};
pub use harness::{
    ART_REVIEW_SCHEMA_VERSION, ArtReview, ArtReviewConfig, ArtReviewError, ArtReviewReport,
    ArtSubject, MAX_REVIEW_SCALE, MIN_REVIEW_SCALE, build_art_review, build_art_review_report,
    render_art_review_html,
};
pub use lint::{ArtCheck, ArtFinding, ArtSeverity, review_sheet};
pub use math::Angle;
pub use png::{encode_indexed_png, encode_png_data_uri};
pub use rig::{BodyProportions, HumanJoint, Pose, Skeleton, humanoid_skeleton};
pub use shape::{Form, LightDirection, Shading};
pub use sprite::{
    CharacterSpec, MAX_SPRITE_HEIGHT, MIN_SPRITE_HEIGHT, RenderedSprite, SpriteRole, SpriteSheet,
    render_character_clips, render_character_frame, render_character_sheet,
};
pub use surface::{Brush, DitherMode, Material, MaterialTable, OutlineMode, Surface};
