//! Procedural character sprites: material assembly, posed drawing, and sheet composition.
//!
//! Purpose: assemble `CharacterSpec` → rig → `Surface` → `Canvas` →
//! `SpriteSheet` deterministically for every role/seed/frame.
//! Owns: role palettes, material tables, body/face/hair drawing, sheet
//! tiling, and height/scale validation (`MIN_SPRITE_HEIGHT`..`MAX_SPRITE_HEIGHT`).
//! Reads: `rig::Skeleton` / `anim::Clip`, `color::Ramp`.
//! Mutates: nothing persistent (returns owned `SpriteSheet`).
//! Does not own: animation clip authoring or harness orchestration.
//! Invariants: integer geometry/shading; every valid config emits frames
//! with stable ordering and palette indices; determinism across profiles.
//! Focused tests: `src/art/sprite.rs` composition and determinism.

use super::anim::Clip;
use super::canvas::Canvas;
use super::color::{Hsl, Palette, Ramp, Rgb8, ShadeProfile};
use super::math::{ONE, scale};
use super::rig::{BodyProportions, HumanJoint, Segment, Skeleton, humanoid_skeleton, to_subpixels};
use super::shape::{
    Form, Shading, add_contact_occlusion, add_depth_seam, add_edge_light, fill_capsule,
    fill_ellipsoid, fill_polygon,
};
use super::surface::{
    BACKGROUND_DEPTH, Brush, DitherMode, Material, MaterialId, MaterialTable, OutlineMode, Surface,
};
use crate::rng::DeterministicRng;
use serde::{Deserialize, Serialize};

/// Smallest supported humanoid sprite height in pixels.
pub const MIN_SPRITE_HEIGHT: i32 = 16;

/// Largest supported humanoid sprite height in pixels.
pub const MAX_SPRITE_HEIGHT: i32 = 256;

/// The occupational silhouette a character is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SpriteRole {
    Baker,
    Merchant,
    Laborer,
    Official,
}

impl SpriteRole {
    pub const ALL: [Self; 4] = [Self::Baker, Self::Merchant, Self::Laborer, Self::Official];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Baker => "baker",
            Self::Merchant => "merchant",
            Self::Laborer => "laborer",
            Self::Official => "official",
        }
    }

    /// Returns whether the role wears a full-length outer garment.
    #[must_use]
    pub const fn wears_long_garment(self) -> bool {
        matches!(self, Self::Merchant | Self::Official)
    }

    /// Returns whether the role wears a waist apron.
    #[must_use]
    pub const fn wears_apron(self) -> bool {
        matches!(self, Self::Baker | Self::Laborer)
    }
}

/// A complete procedural recipe for one character sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterSpec {
    pub role: SpriteRole,
    /// Standing height in pixels.
    pub height: i32,
    /// Per-mille build factor applied to limb and torso thickness.
    pub build: i32,
    pub skin: Rgb8,
    pub hair: Rgb8,
    pub garment: Rgb8,
    pub accent: Rgb8,
    pub leather: Rgb8,
}

impl CharacterSpec {
    /// Builds a deterministic character from a seed and role.
    ///
    /// # Panics
    ///
    /// Panics when `height` falls outside the supported sprite-height range.
    #[must_use]
    pub fn from_seed(seed: u64, role: SpriteRole, height: i32) -> Self {
        assert!(
            (MIN_SPRITE_HEIGHT..=MAX_SPRITE_HEIGHT).contains(&height),
            "sprite height must be in {MIN_SPRITE_HEIGHT}..={MAX_SPRITE_HEIGHT}"
        );
        let mut rng = DeterministicRng::seeded(seed);
        let skin = pick(
            &mut rng,
            &[
                Rgb8::new(206, 166, 136),
                Rgb8::new(182, 138, 106),
                Rgb8::new(150, 106, 76),
                Rgb8::new(110, 74, 52),
                Rgb8::new(78, 52, 38),
            ],
        );
        let hair = pick(
            &mut rng,
            &[
                Rgb8::new(44, 32, 26),
                Rgb8::new(92, 60, 36),
                Rgb8::new(146, 104, 56),
                Rgb8::new(178, 148, 96),
                Rgb8::new(122, 118, 112),
            ],
        );
        let garment = jitter(
            &mut rng,
            match role {
                SpriteRole::Baker => Rgb8::new(196, 178, 148),
                SpriteRole::Merchant => Rgb8::new(96, 62, 108),
                SpriteRole::Laborer => Rgb8::new(108, 96, 76),
                SpriteRole::Official => Rgb8::new(58, 72, 112),
            },
            240,
            120,
        );
        let accent = jitter(
            &mut rng,
            match role {
                SpriteRole::Baker => Rgb8::new(158, 74, 52),
                SpriteRole::Merchant => Rgb8::new(184, 148, 68),
                SpriteRole::Laborer => Rgb8::new(72, 92, 74),
                SpriteRole::Official => Rgb8::new(158, 44, 52),
            },
            300,
            80,
        );
        let build = 880 + i32::try_from(rng.range_u32(280)).expect("build variation must fit i32");
        Self {
            role,
            height,
            build,
            skin,
            hair,
            garment,
            accent,
            leather: jitter(&mut rng, Rgb8::new(94, 66, 44), 200, 100),
        }
    }

    #[must_use]
    pub fn proportions(self) -> BodyProportions {
        self.assert_valid();
        BodyProportions::adult(self.height).with_build(self.build)
    }

    #[must_use]
    pub fn skeleton(self) -> Skeleton {
        humanoid_skeleton(self.proportions())
    }

    /// Returns the pixel size of one animation frame for this character.
    ///
    /// # Panics
    ///
    /// Panics when this spec's public height field falls outside the supported sprite-height
    /// range.
    #[must_use]
    pub fn frame_size(self) -> (u32, u32) {
        self.assert_valid();
        let width = u32::try_from(scale(self.height, 15, 16).max(8))
            .expect("validated sprite width must fit u32");
        let height = u32::try_from(
            self.height
                .checked_add(12)
                .expect("validated sprite frame height must fit i32"),
        )
        .expect("validated sprite frame height must fit u32");
        (width | 1, height)
    }

    fn assert_valid(self) {
        assert!(
            (MIN_SPRITE_HEIGHT..=MAX_SPRITE_HEIGHT).contains(&self.height),
            "sprite height must be in {MIN_SPRITE_HEIGHT}..={MAX_SPRITE_HEIGHT}"
        );
    }
}

fn pick(rng: &mut DeterministicRng, options: &[Rgb8]) -> Rgb8 {
    assert!(!options.is_empty(), "color choices must not be empty");
    let count = u32::try_from(options.len()).expect("color choice count must fit u32");
    let index = usize::try_from(rng.range_u32(count)).expect("color choice index must fit usize");
    options[index]
}

fn jitter(rng: &mut DeterministicRng, base: Rgb8, hue_range: i32, lightness_range: i32) -> Rgb8 {
    assert!(hue_range >= 0, "hue jitter range must not be negative");
    assert!(
        lightness_range >= 0,
        "lightness jitter range must not be negative"
    );
    let span =
        |range: i32| u32::try_from(i64::from(range) * 2 + 1).expect("jitter span must fit u32");
    let hue_delta =
        i32::try_from(rng.range_u32(span(hue_range))).expect("hue jitter must fit i32") - hue_range;
    let lightness_delta = i32::try_from(rng.range_u32(span(lightness_range)))
        .expect("lightness jitter must fit i32")
        - lightness_range;
    Hsl::from_rgb(base)
        .shifted(hue_delta, 0, lightness_delta)
        .to_rgb()
}

/// Skin holds a narrower highlight range than cloth so that faces and hands do not blow out.
const SKIN_PROFILE: ShadeProfile = ShadeProfile {
    steps: 9,
    shadow_hue_shift: -140,
    highlight_hue_shift: 60,
    shadow_depth: 470,
    highlight_reach: 120,
    shadow_saturation_gain: 60,
    highlight_saturation_loss: 340,
};

/// The material slots every character sprite uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CharacterMaterials {
    skin: MaterialId,
    hair: MaterialId,
    garment: MaterialId,
    accent: MaterialId,
    leather: MaterialId,
    shadow: MaterialId,
    detail: MaterialId,
}

fn build_materials(spec: CharacterSpec) -> (Palette, MaterialTable, CharacterMaterials) {
    let mut palette = Palette::new();
    let mut materials = MaterialTable::new();

    let skin_ramp = palette.insert_ramp(&Ramp::build(spec.skin, SKIN_PROFILE));
    let hair_ramp = palette.insert_ramp(&Ramp::build(spec.hair, ShadeProfile::specular()));
    let garment_ramp = palette.insert_ramp(&Ramp::build(spec.garment, ShadeProfile::material()));
    let accent_ramp = palette.insert_ramp(&Ramp::build(spec.accent, ShadeProfile::material()));
    let leather_ramp = palette.insert_ramp(&Ramp::build(spec.leather, ShadeProfile::specular()));
    let shadow_ramp = palette.insert_ramp(&Ramp::from_colors(vec![
        Rgb8::new(28, 26, 34),
        Rgb8::new(44, 42, 52),
    ]));
    let detail_ramp = palette.insert_ramp(&Ramp::from_colors(vec![
        Hsl::from_rgb(spec.skin).shifted(-150, 60, -620).to_rgb(),
        Hsl::from_rgb(spec.skin).shifted(-150, 40, -520).to_rgb(),
    ]));

    let slots = CharacterMaterials {
        skin: materials.register(Material::new(skin_ramp).with_dither(DitherMode::ShadowOnly)),
        hair: materials.register(Material::new(hair_ramp).with_dither(DitherMode::ShadowOnly)),
        garment: materials.register(Material::new(garment_ramp).with_dither(DitherMode::None)),
        accent: materials.register(Material::new(accent_ramp).with_dither(DitherMode::None)),
        leather: materials
            .register(Material::new(leather_ramp).with_dither(DitherMode::ShadowOnly)),
        shadow: materials.register(
            Material::new(shadow_ramp)
                .with_dither(DitherMode::None)
                .with_outline(OutlineMode::None),
        ),
        detail: materials.register(
            Material::new(detail_ramp)
                .with_dither(DitherMode::None)
                .with_outline(OutlineMode::None),
        ),
    };
    (palette, materials, slots)
}

/// One rendered animation frame with the palette needed to display it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedSprite {
    pub canvas: Canvas,
    pub palette: Palette,
}

/// A horizontal strip of frames for one clip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteSheet {
    pub clip_name: String,
    pub looping: bool,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_count: u32,
    pub canvas: Canvas,
    pub palette: Palette,
}

/// Renders one posed frame of a character.
///
/// # Panics
///
/// Panics when `spec` violates the supported sprite-height range, `clip` does not match the
/// humanoid skeleton's joint count, or a clip root offset moves the rig outside the representable
/// sub-pixel coordinate range.
#[must_use]
pub fn render_character_frame(spec: CharacterSpec, clip: &Clip, frame: u32) -> RenderedSprite {
    let (palette, materials, slots) = build_materials(spec);
    let (width, height) = spec.frame_size();
    let mut surface = Surface::new(width, height);
    let skeleton = spec.skeleton();
    let sample = clip.sample(frame);
    let proportions = spec.proportions();

    let ground = i32::try_from(height).expect("validated frame height must fit i32") - 7;
    let root_x = to_subpixels(scale(
        i32::try_from(width).expect("validated frame width must fit i32"),
        45,
        100,
    ));
    let root_y = coordinate_add(
        to_subpixels(ground - scale(proportions.height, 470, 1_000)),
        sample.root_offset.1,
    );
    let segments = skeleton.resolve_segments(
        &sample.pose,
        (coordinate_add(root_x, sample.root_offset.0), root_y),
    );
    let shading = Shading::key();

    draw_ground_shadow(&mut surface, slots, &proportions, ground, width);
    draw_legs(&mut surface, shading, slots, spec, &segments);
    draw_torso(&mut surface, shading, slots, spec, &segments, &proportions);
    draw_arms(&mut surface, shading, slots, spec, &segments);
    draw_head(&mut surface, shading, slots, spec, &segments, &proportions);

    add_depth_seam(&mut surface, 190, 8);
    add_contact_occlusion(&mut surface, 2, 110, 8);
    add_edge_light(&mut surface, 0, -1, 70, 520);
    add_edge_light(&mut surface, -1, 0, 45, 520);

    RenderedSprite {
        canvas: surface.resolve(&materials),
        palette,
    }
}

/// Renders every frame of one clip into a horizontal sheet.
///
/// # Panics
///
/// Panics when the composed sheet width overflows `u32`.
#[must_use]
pub fn render_character_sheet(spec: CharacterSpec, clip: &Clip) -> SpriteSheet {
    let (frame_width, frame_height) = spec.frame_size();
    let frame_count = clip.frame_count();
    let sheet_width = frame_width
        .checked_mul(frame_count)
        .expect("sheet width must fit u32");
    let mut canvas = Canvas::new(sheet_width, frame_height);
    let mut palette = Palette::new();
    for frame in 0..frame_count {
        let rendered = render_character_frame(spec, clip, frame);
        let origin = frame
            .checked_mul(frame_width)
            .expect("frame origin must fit u32");
        canvas.blit(
            &rendered.canvas,
            i32::try_from(origin).expect("frame origin must fit i32 coordinates"),
            0,
        );
        palette = rendered.palette;
    }
    SpriteSheet {
        clip_name: clip.name().to_owned(),
        looping: clip.is_looping(),
        frame_width,
        frame_height,
        frame_count,
        canvas,
        palette,
    }
}

/// Renders every standard clip for a character.
#[must_use]
pub fn render_character_clips(spec: CharacterSpec) -> Vec<SpriteSheet> {
    let skeleton = spec.skeleton();
    crate::art::humanoid_clip_library(&skeleton)
        .iter()
        .map(|clip| render_character_sheet(spec, clip))
        .collect()
}

fn draw_ground_shadow(
    surface: &mut Surface,
    slots: CharacterMaterials,
    proportions: &BodyProportions,
    ground: i32,
    width: u32,
) {
    let width = i32::try_from(width).expect("frame width must fit i32");
    let radius_x = scale(proportions.hip_width, 3, 2).clamp(3, width / 2 - 2);
    let center_x = width / 2;
    fill_ellipsoid(
        surface,
        Shading::key(),
        Form::new(slots.shadow, BACKGROUND_DEPTH),
        center_x,
        ground + 1,
        radius_x,
        (radius_x / 3).clamp(1, 2),
    );
}

fn coordinate_add(left: i32, right: i32) -> i32 {
    i32::try_from(i64::from(left) + i64::from(right)).expect("sprite coordinate must fit i32")
}

fn pixel_segment(segments: &[Segment], joint: HumanJoint) -> ((i32, i32), (i32, i32), i32) {
    let segment = segments[joint.index()];
    let (start, end) = segment.to_pixel_endpoints();
    (start, end, segment.pixel_radius())
}

fn limb_depth(joint: HumanJoint, base: i16) -> i16 {
    if joint.is_near_side() {
        base + 20
    } else {
        base - 20
    }
}

fn draw_legs(
    surface: &mut Surface,
    shading: Shading,
    slots: CharacterMaterials,
    spec: CharacterSpec,
    segments: &[Segment],
) {
    let thigh_material = if spec.role.wears_long_garment() {
        slots.garment
    } else {
        slots.leather
    };
    for (joint, material) in [
        (HumanJoint::FarThigh, thigh_material),
        (HumanJoint::FarShin, slots.leather),
        (HumanJoint::FarFoot, slots.leather),
        (HumanJoint::NearThigh, thigh_material),
        (HumanJoint::NearShin, slots.leather),
        (HumanJoint::NearFoot, slots.leather),
    ] {
        let (start, end, radius) = pixel_segment(segments, joint);
        let depth = limb_depth(joint, -30);
        let bias = if joint.is_near_side() { 0 } else { -90 };
        fill_capsule(
            surface,
            shading,
            Form::new(material, depth).with_light_bias(bias),
            start,
            end,
            radius,
        );
    }
}

fn draw_torso(
    surface: &mut Surface,
    shading: Shading,
    slots: CharacterMaterials,
    spec: CharacterSpec,
    segments: &[Segment],
    proportions: &BodyProportions,
) {
    let pelvis = segments[HumanJoint::Pelvis.index()];
    let spine = segments[HumanJoint::Spine.index()];
    let chest = segments[HumanJoint::Chest.index()];
    let (pelvis_point, _) = pelvis.to_pixel_endpoints();
    let (_, waist_point) = spine.to_pixel_endpoints();
    let (_, chest_point) = chest.to_pixel_endpoints();

    fill_capsule(
        surface,
        shading,
        Form::new(slots.garment, 0),
        pelvis_point,
        waist_point,
        proportions.hip_width,
    );
    fill_capsule(
        surface,
        shading,
        Form::new(slots.garment, 1),
        waist_point,
        chest_point,
        (proportions.shoulder_width * 7 / 10).max(2),
    );
    fill_ellipsoid(
        surface,
        shading,
        Form::new(slots.garment, 2),
        chest_point.0,
        chest_point.1,
        proportions.shoulder_width,
        (proportions.shoulder_width * 2 / 3).max(1),
    );

    if spec.role.wears_apron() {
        let half = (proportions.hip_width * 11 / 10).max(1);
        let waist_y = pelvis_point.1 - (pelvis_point.1 - chest_point.1) / 5;
        fill_polygon(
            surface,
            Form::new(slots.accent, 6).flat(),
            &[
                (pelvis_point.0 - half, waist_y),
                (pelvis_point.0 + half, waist_y),
                (
                    pelvis_point.0 + half + 1,
                    pelvis_point.1 + proportions.hip_width * 3 / 2,
                ),
                (
                    pelvis_point.0 - half - 1,
                    pelvis_point.1 + proportions.hip_width * 3 / 2,
                ),
            ],
            470,
        );
    }
    if spec.role.wears_long_garment() {
        let half = (proportions.shoulder_width * 11 / 10).max(2);
        let hem = pelvis_point.1 + proportions.height * 220 / 1_000;
        fill_polygon(
            surface,
            Form::new(slots.accent, 4).with_light_bias(-40).flat(),
            &[
                (chest_point.0 - half + 1, chest_point.1),
                (chest_point.0 + half - 1, chest_point.1),
                (chest_point.0 + half, hem),
                (chest_point.0 - half, hem),
            ],
            520,
        );
        draw_folds(surface, slots, chest_point.0, chest_point.1, hem, half);
    }
}

/// Darkens vertical creases down a long garment.
///
/// A flat panel of one ramp step reads as cardboard at any size. Two creases placed off-center
/// give the cloth a direction to hang in without spending palette entries or breaking the
/// silhouette.
fn draw_folds(
    surface: &mut Surface,
    slots: CharacterMaterials,
    center_x: i32,
    top: i32,
    hem: i32,
    half: i32,
) {
    if half < 3 {
        return;
    }
    for (offset, depth) in [(-half / 2, 110), (half / 3, 70)] {
        for y in (top + 1)..hem {
            let x = center_x + offset;
            if surface.material_at(x, y) == slots.accent {
                surface.add_light(x, y, -depth);
            }
        }
    }
}

fn draw_arms(
    surface: &mut Surface,
    shading: Shading,
    slots: CharacterMaterials,
    spec: CharacterSpec,
    segments: &[Segment],
) {
    let sleeve = if spec.role.wears_long_garment() {
        slots.accent
    } else {
        slots.garment
    };
    for (joint, material) in [
        (HumanJoint::FarUpperArm, sleeve),
        (HumanJoint::FarForearm, sleeve),
        (HumanJoint::FarHand, slots.skin),
        (HumanJoint::NearUpperArm, sleeve),
        (HumanJoint::NearForearm, sleeve),
        (HumanJoint::NearHand, slots.skin),
    ] {
        let (start, end, radius) = pixel_segment(segments, joint);
        let depth = limb_depth(joint, 10);
        let mut bias = if joint.is_near_side() { 0 } else { -90 };
        if material == slots.skin {
            bias -= 60;
        }
        fill_capsule(
            surface,
            shading,
            Form::new(material, depth).with_light_bias(bias),
            start,
            end,
            radius,
        );
    }
}

fn draw_head(
    surface: &mut Surface,
    shading: Shading,
    slots: CharacterMaterials,
    spec: CharacterSpec,
    segments: &[Segment],
    proportions: &BodyProportions,
) {
    let neck = segments[HumanJoint::Neck.index()];
    let head = segments[HumanJoint::Head.index()];
    let (neck_start, neck_end) = neck.to_pixel_endpoints();
    let (_, head_end) = head.to_pixel_endpoints();
    let radius_y = (proportions.head_height / 2).max(2);
    let radius_x = radius_y.max(2);
    let center = (
        i32::midpoint(neck_end.0, head_end.0),
        i32::midpoint(neck_end.1, head_end.1),
    );

    fill_capsule(
        surface,
        shading,
        Form::new(slots.skin, 8).with_light_bias(-80),
        neck_start,
        neck_end,
        (radius_x * 4 / 10).max(1),
    );
    fill_ellipsoid(
        surface,
        shading,
        Form::new(slots.skin, 30).with_light_bias(-40),
        center.0,
        center.1,
        radius_x,
        radius_y,
    );
    draw_hair(surface, slots, center, radius_x, radius_y);
    if spec.role == SpriteRole::Official {
        fill_ellipsoid(
            surface,
            shading,
            Form::new(slots.accent, 34).flat(),
            center.0,
            center.1 - radius_y,
            radius_x + 1,
            (radius_y / 2).max(1),
        );
    }
    draw_face(surface, slots, center, radius_x, radius_y);
}

/// Recolors the head volume into hair where the cut covers it.
///
/// The hair is a mask over the already-shaded skull rather than a second ellipsoid. Reusing the
/// skull's light keeps the hair sitting on the head instead of floating above it as a cap, and it
/// lets the cut follow a hairline: over the crown, down the back, and stopping short of the brow.
fn draw_hair(
    surface: &mut Surface,
    slots: CharacterMaterials,
    center: (i32, i32),
    radius_x: i32,
    radius_y: i32,
) {
    let mut hairline = Vec::new();
    for y in (center.1 - radius_y)..=(center.1 + radius_y) {
        for x in (center.0 - radius_x)..=(center.0 + radius_x) {
            if surface.material_at(x, y) != slots.skin {
                continue;
            }
            let normal_x = scale(x - center.0, ONE, radius_x);
            let normal_y = scale(y - center.1, ONE, radius_y);
            let covers_crown = normal_y <= -ONE / 8;
            let covers_back = normal_x <= -ONE / 3 && normal_y <= ONE * 3 / 5;
            let bares_face = normal_x >= ONE / 5 && normal_y >= -ONE / 8;
            if (covers_crown || covers_back) && !bares_face {
                hairline.push((x, y, surface.light_at(x, y), surface.depth_at(x, y)));
            }
        }
    }
    for (x, y, light, depth) in hairline {
        surface.plot(x, y, Brush::new(slots.hair, light, depth + 1));
    }
}

/// Draws the brow, eyes, nose, and mouth that let a small head read as a face.
///
/// Detail is placed relative to the head radii, so it scales with the sprite instead of assuming
/// one size, and every mark is suppressed when the head is too small to carry it.
fn draw_face(
    surface: &mut Surface,
    slots: CharacterMaterials,
    center: (i32, i32),
    radius_x: i32,
    radius_y: i32,
) {
    if radius_x < 2 || radius_y < 2 {
        return;
    }
    let facing = (radius_x / 4).max(0);
    let eye_y = center.1 + (radius_y / 5).max(0);
    let spread = (radius_x * 45 / 100).max(1);

    for offset in [-spread, spread] {
        let eye_x = center.0 + facing + offset;
        if surface.material_at(eye_x, eye_y) == slots.skin {
            let depth = surface.depth_at(eye_x, eye_y);
            surface.plot(eye_x, eye_y, Brush::new(slots.detail, 600, depth + 1));
        }
        surface.add_light(center.0 + facing + offset, eye_y - 1, -170);
    }

    if radius_y >= 3 {
        surface.add_light(center.0 + facing + spread / 2, eye_y + 1, -130);
        let mouth_y = center.1 + (radius_y * 3 / 5).max(1);
        for offset in 0..=i32::from(radius_x >= 3) {
            surface.add_light(center.0 + facing - offset, mouth_y, -150);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::anim::{HumanClip, humanoid_clip};
    use super::super::color::TRANSPARENT_INDEX;
    use super::super::rig::humanoid_skeleton;
    use super::*;

    fn spec() -> CharacterSpec {
        CharacterSpec::from_seed(7, SpriteRole::Baker, 48)
    }

    fn idle_clip(spec: CharacterSpec) -> Clip {
        humanoid_clip(&humanoid_skeleton(spec.proportions()), HumanClip::Idle)
    }

    #[test]
    fn generated_characters_are_deterministic_for_a_seed() {
        let first = CharacterSpec::from_seed(42, SpriteRole::Merchant, 48);
        let second = CharacterSpec::from_seed(42, SpriteRole::Merchant, 48);

        assert_eq!(first, second);
    }

    #[test]
    #[should_panic(expected = "sprite height must be in")]
    fn fabricated_invalid_specs_are_rejected_at_the_render_boundary() {
        let mut invalid = spec();
        invalid.height = MIN_SPRITE_HEIGHT - 1;

        let _ = invalid.frame_size();
    }

    #[test]
    fn distinct_seeds_produce_distinct_characters() {
        let first = CharacterSpec::from_seed(1, SpriteRole::Merchant, 48);
        let second = CharacterSpec::from_seed(2, SpriteRole::Merchant, 48);

        assert_ne!(first, second);
    }

    #[test]
    fn rendered_frames_fill_a_plausible_share_of_the_frame() {
        let spec = spec();
        let clip = idle_clip(spec);
        let rendered = render_character_frame(spec, &clip, 0);
        let (width, height) = spec.frame_size();
        let area = usize::try_from(width * height).expect("frame area must fit usize");
        let opaque = rendered.canvas.opaque_count();

        assert!(
            opaque * 100 / area > 12 && opaque * 100 / area < 70,
            "silhouette covered {opaque} of {area} pixels"
        );
    }

    #[test]
    fn rendered_frames_stay_inside_the_frame_bounds() {
        let spec = spec();
        let clip = idle_clip(spec);
        let rendered = render_character_frame(spec, &clip, 0);
        let (width, height) = spec.frame_size();
        let bounds = rendered
            .canvas
            .opaque_bounds()
            .expect("a frame must draw pixels");

        assert!(bounds.x >= 0 && bounds.y >= 0);
        assert!(u32::try_from(bounds.x).unwrap_or(0) + bounds.width <= width);
        assert!(u32::try_from(bounds.y).unwrap_or(0) + bounds.height <= height);
    }

    #[test]
    fn rendering_the_same_frame_twice_is_identical() {
        let spec = spec();
        let clip = idle_clip(spec);

        assert_eq!(
            render_character_frame(spec, &clip, 3),
            render_character_frame(spec, &clip, 3)
        );
    }

    #[test]
    fn animation_frames_differ_from_one_another() {
        let spec = CharacterSpec::from_seed(3, SpriteRole::Laborer, 48);
        let skeleton = humanoid_skeleton(spec.proportions());
        let clip = humanoid_clip(&skeleton, HumanClip::Walk);

        let first = render_character_frame(spec, &clip, 0).canvas;
        let middle = render_character_frame(spec, &clip, 4).canvas;

        assert_ne!(first, middle);
    }

    #[test]
    fn sheets_lay_frames_out_horizontally() {
        let spec = spec();
        let skeleton = humanoid_skeleton(spec.proportions());
        let clip = humanoid_clip(&skeleton, HumanClip::Walk);

        let sheet = render_character_sheet(spec, &clip);

        assert_eq!(sheet.canvas.width(), sheet.frame_width * sheet.frame_count);
        assert_eq!(sheet.canvas.height(), sheet.frame_height);
    }

    #[test]
    fn every_role_renders_a_non_empty_silhouette() {
        for role in SpriteRole::ALL {
            let spec = CharacterSpec::from_seed(11, role, 48);
            let clip = idle_clip(spec);
            let rendered = render_character_frame(spec, &clip, 0);

            assert!(
                rendered.canvas.opaque_count() > 0,
                "{} produced no pixels",
                role.name()
            );
        }
    }

    #[test]
    fn palettes_stay_within_the_indexed_limit() {
        let spec = spec();
        let clip = idle_clip(spec);
        let rendered = render_character_frame(spec, &clip, 0);

        assert!(rendered.palette.len() <= 64);
        assert_eq!(rendered.canvas.get(0, 0), TRANSPARENT_INDEX);
    }

    #[test]
    fn every_standard_clip_renders_for_a_character() {
        let sheets = render_character_clips(spec());

        assert_eq!(sheets.len(), HumanClip::ALL.len());
        assert!(sheets.iter().all(|sheet| sheet.frame_count >= 8));
    }
}
