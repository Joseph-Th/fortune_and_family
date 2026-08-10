//! Material, light, and depth buffers that resolve into an indexed canvas.
//!
//! Procedural drawing never writes palette indices directly. Shapes write a material
//! identifier, a per-mille light value, and a depth value. Resolution then maps light onto
//! each material's ramp, applies ordered dithering between ramp steps, and darkens silhouette
//! edges. Separating form from color is what allows a pose to be relit, recolored, or
//! restyled without redrawing it.

use super::canvas::Canvas;
use super::color::{Palette, RampHandle, TRANSPARENT_INDEX};

/// Identifies a material slot in a [`MaterialTable`]. Zero means "no material".
pub type MaterialId = u8;

/// The empty material written by an untouched surface pixel.
pub const NO_MATERIAL: MaterialId = 0;

/// Neutral light with no shading applied.
pub const MID_LIGHT: i32 = 500;

/// Depth value used by background elements that everything else occludes.
pub const BACKGROUND_DEPTH: i16 = -1_000;

const PER_MILLE: i32 = 1_000;

const BAYER_4X4: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// How a material blends between adjacent ramp steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DitherMode {
    /// Hard banding with no intermediate pixels.
    None,
    /// Ordered 4x4 dithering across the whole light range.
    Ordered,
    /// Ordered dithering restricted to the darker half, keeping highlights clean.
    ShadowOnly,
}

/// How a material treats its silhouette edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineMode {
    /// No edge treatment.
    None,
    /// Selective outline drawn with the material's own darkest ramp step.
    Selective,
    /// Outline drawn with an explicit palette index.
    Fixed(u8),
}

/// A drawable material: one ramp plus its shading and edge treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    pub ramp: RampHandle,
    pub dither: DitherMode,
    pub outline: OutlineMode,
}

impl Material {
    #[must_use]
    pub const fn new(ramp: RampHandle) -> Self {
        Self {
            ramp,
            dither: DitherMode::ShadowOnly,
            outline: OutlineMode::Selective,
        }
    }

    #[must_use]
    pub const fn with_dither(mut self, dither: DitherMode) -> Self {
        self.dither = dither;
        self
    }

    #[must_use]
    pub const fn with_outline(mut self, outline: OutlineMode) -> Self {
        self.outline = outline;
        self
    }
}

/// The ordered set of materials referenced by a surface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaterialTable {
    materials: Vec<Material>,
}

impl MaterialTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a material and returns its identifier.
    ///
    /// # Panics
    ///
    /// Panics when more than 255 materials are registered.
    pub fn register(&mut self, material: Material) -> MaterialId {
        assert!(
            self.materials.len() < 255,
            "a surface supports at most 255 materials"
        );
        self.materials.push(material);
        u8::try_from(self.materials.len()).expect("material count must fit a byte")
    }

    #[must_use]
    pub fn get(&self, id: MaterialId) -> Option<Material> {
        if id == NO_MATERIAL {
            return None;
        }
        self.materials.get(usize::from(id) - 1).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

/// One drawing operation's material, light, and depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brush {
    pub material: MaterialId,
    pub light: i32,
    pub depth: i16,
    /// Whether this pixel participates in its material's dithering.
    pub dither: bool,
}

impl Brush {
    #[must_use]
    pub const fn new(material: MaterialId, light: i32, depth: i16) -> Self {
        Self {
            material,
            light,
            depth,
            dither: true,
        }
    }

    #[must_use]
    pub const fn lit(self, light: i32) -> Self {
        Self { light, ..self }
    }

    /// Returns this brush with dithering suppressed.
    ///
    /// Flat fills such as garment panels have no light gradient, so dithering them produces an
    /// even checker of two ramp steps rather than a transition. Curved forms keep it.
    #[must_use]
    pub const fn undithered(self) -> Self {
        Self {
            dither: false,
            ..self
        }
    }
}

/// A form buffer holding material, light, and depth per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    width: u32,
    height: u32,
    material: Vec<MaterialId>,
    light: Vec<i16>,
    depth: Vec<i16>,
    dither: Vec<bool>,
}

impl Surface {
    /// Creates an empty surface.
    ///
    /// # Panics
    ///
    /// Panics when either dimension is zero, a dimension cannot be addressed by the signed
    /// coordinate API, or the pixel count overflows `usize`.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        assert!(
            width > 0 && height > 0,
            "surface dimensions must be positive"
        );
        assert!(
            i32::try_from(width).is_ok() && i32::try_from(height).is_ok(),
            "surface dimensions must fit i32 coordinates"
        );
        let count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .expect("surface pixel count must fit usize");
        Self {
            width,
            height,
            material: vec![NO_MATERIAL; count],
            light: vec![0; count],
            depth: vec![i16::MIN; count],
            dither: vec![true; count],
        }
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    fn offset(&self, x: i32, y: i32) -> Option<usize> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        if x >= self.width || y >= self.height {
            return None;
        }
        let width = usize::try_from(self.width).ok()?;
        Some(usize::try_from(y).ok()? * width + usize::try_from(x).ok()?)
    }

    /// Writes one pixel when the brush depth is at least the stored depth.
    ///
    /// # Panics
    ///
    /// Panics when the clamped light value does not fit `i16`, which clamping prevents.
    pub fn plot(&mut self, x: i32, y: i32, brush: Brush) {
        let Some(offset) = self.offset(x, y) else {
            return;
        };
        if brush.depth < self.depth[offset] {
            return;
        }
        self.material[offset] = brush.material;
        self.light[offset] =
            i16::try_from(brush.light.clamp(0, PER_MILLE)).expect("clamped light must fit i16");
        self.depth[offset] = brush.depth;
        self.dither[offset] = brush.dither;
    }

    /// Adds `delta` to the stored light of an already-drawn pixel.
    ///
    /// # Panics
    ///
    /// Panics when the clamped light value does not fit `i16`, which clamping prevents.
    pub fn add_light(&mut self, x: i32, y: i32, delta: i32) {
        let Some(offset) = self.offset(x, y) else {
            return;
        };
        if self.material[offset] == NO_MATERIAL {
            return;
        }
        let value =
            (i64::from(self.light[offset]) + i64::from(delta)).clamp(0, i64::from(PER_MILLE));
        self.light[offset] = i16::try_from(value).expect("clamped light must fit i16");
    }

    #[must_use]
    pub fn material_at(&self, x: i32, y: i32) -> MaterialId {
        self.offset(x, y)
            .map_or(NO_MATERIAL, |offset| self.material[offset])
    }

    #[must_use]
    pub fn light_at(&self, x: i32, y: i32) -> i32 {
        self.offset(x, y)
            .map_or(0, |offset| i32::from(self.light[offset]))
    }

    #[must_use]
    pub fn depth_at(&self, x: i32, y: i32) -> i16 {
        self.offset(x, y)
            .map_or(i16::MIN, |offset| self.depth[offset])
    }

    /// Returns whether the pixel participates in its material's dithering.
    #[must_use]
    pub fn dithers_at(&self, x: i32, y: i32) -> bool {
        self.offset(x, y).is_some_and(|offset| self.dither[offset])
    }

    #[must_use]
    fn is_edge(&self, x: i32, y: i32) -> bool {
        [(0, -1), (0, 1), (-1, 0), (1, 0)]
            .into_iter()
            .any(|(step_x, step_y)| {
                x.checked_add(step_x).zip(y.checked_add(step_y)).is_none_or(
                    |(neighbor_x, neighbor_y)| {
                        self.material_at(neighbor_x, neighbor_y) == NO_MATERIAL
                    },
                )
            })
    }

    /// Resolves material and light into palette indices.
    ///
    /// # Panics
    ///
    /// Panics when a drawn pixel references a material that is not registered in `materials`.
    #[must_use]
    pub fn resolve(&self, materials: &MaterialTable) -> Canvas {
        let mut canvas = Canvas::new(self.width, self.height);
        for y in 0..i32::try_from(self.height).expect("surface height must fit i32") {
            for x in 0..i32::try_from(self.width).expect("surface width must fit i32") {
                let id = self.material_at(x, y);
                if id == NO_MATERIAL {
                    continue;
                }
                let material = materials
                    .get(id)
                    .expect("drawn pixels must reference a registered material");
                let dithers = self.dithers_at(x, y);
                let index = if self.is_edge(x, y) {
                    match material.outline {
                        OutlineMode::None => {
                            resolve_step(material, self.light_at(x, y), dithers, x, y)
                        }
                        OutlineMode::Selective => material.ramp.darkest(),
                        OutlineMode::Fixed(index) => index,
                    }
                } else {
                    resolve_step(material, self.light_at(x, y), dithers, x, y)
                };
                canvas.set(x, y, index);
            }
        }
        canvas
    }
}

fn resolve_step(material: Material, light: i32, dithers: bool, x: i32, y: i32) -> u8 {
    let last = i32::from(material.ramp.length()) - 1;
    let scaled = light.clamp(0, PER_MILLE) * last;
    let step = scaled / PER_MILLE;
    let fraction = scaled % PER_MILLE;
    let dithers = dithers
        && match material.dither {
            DitherMode::None => false,
            DitherMode::Ordered => true,
            DitherMode::ShadowOnly => light <= MID_LIGHT,
        };
    if dithers && fraction > ordered_threshold(x, y) {
        material.ramp.index(step + 1)
    } else {
        material.ramp.index(step)
    }
}

fn ordered_threshold(x: i32, y: i32) -> i32 {
    let column = usize::try_from(x.rem_euclid(4)).expect("wrapped column must fit usize");
    let row = usize::try_from(y.rem_euclid(4)).expect("wrapped row must fit usize");
    BAYER_4X4[row][column] * PER_MILLE / 16
}

/// Builds a canvas and its palette in one step.
#[must_use]
pub fn resolve_with_palette(
    surface: &Surface,
    materials: &MaterialTable,
    palette: &Palette,
) -> (Canvas, Palette) {
    (surface.resolve(materials), palette.clone())
}

/// Returns whether `index` denotes transparency.
#[must_use]
pub const fn is_transparent(index: u8) -> bool {
    index == TRANSPARENT_INDEX
}

#[cfg(test)]
mod tests {
    use super::super::color::{Ramp, Rgb8, ShadeProfile};
    use super::*;

    fn material_table() -> (Palette, MaterialTable, MaterialId) {
        let mut palette = Palette::new();
        let ramp = palette.insert_ramp(&Ramp::build(
            Rgb8::new(150, 96, 72),
            ShadeProfile::material(),
        ));
        let mut materials = MaterialTable::new();
        let id = materials.register(Material::new(ramp).with_dither(DitherMode::None));
        (palette, materials, id)
    }

    #[test]
    fn depth_testing_keeps_the_nearest_write() {
        let (_, _, id) = material_table();
        let mut surface = Surface::new(4, 4);

        surface.plot(1, 1, Brush::new(id, 500, 10));
        surface.plot(1, 1, Brush::new(id, 900, 5));

        assert_eq!(surface.light_at(1, 1), 500);

        surface.plot(1, 1, Brush::new(id, 900, 20));

        assert_eq!(surface.light_at(1, 1), 900);
    }

    #[test]
    fn light_is_clamped_to_the_representable_range() {
        let (_, _, id) = material_table();
        let mut surface = Surface::new(2, 2);

        surface.plot(0, 0, Brush::new(id, 5_000, 0));
        surface.add_light(0, 0, 5_000);

        assert_eq!(surface.light_at(0, 0), 1_000);
    }

    #[test]
    fn added_light_ignores_undrawn_pixels() {
        let mut surface = Surface::new(2, 2);

        surface.add_light(0, 0, 400);

        assert_eq!(surface.light_at(0, 0), 0);
    }

    #[test]
    fn resolution_maps_light_onto_the_ramp() {
        let (_, materials, id) = material_table();
        let mut surface = Surface::new(5, 5);
        for y in 0..5 {
            for x in 0..5 {
                surface.plot(x, y, Brush::new(id, 1_000, 0));
            }
        }

        let canvas = surface.resolve(&materials);
        let material = materials.get(id).expect("material must exist");

        assert_eq!(canvas.get(2, 2), material.ramp.lightest());
    }

    #[test]
    fn selective_outlines_darken_only_silhouette_pixels() {
        let (_, materials, id) = material_table();
        let mut surface = Surface::new(5, 5);
        for y in 1..4 {
            for x in 1..4 {
                surface.plot(x, y, Brush::new(id, 1_000, 0));
            }
        }

        let canvas = surface.resolve(&materials);
        let material = materials.get(id).expect("material must exist");

        assert_eq!(canvas.get(1, 1), material.ramp.darkest());
        assert_eq!(canvas.get(2, 2), material.ramp.lightest());
    }

    #[test]
    fn ordered_dithering_produces_two_ramp_steps_at_a_boundary() {
        let mut palette = Palette::new();
        let ramp = palette.insert_ramp(&Ramp::build(
            Rgb8::new(150, 96, 72),
            ShadeProfile::material(),
        ));
        let mut materials = MaterialTable::new();
        let id = materials.register(
            Material::new(ramp)
                .with_dither(DitherMode::Ordered)
                .with_outline(OutlineMode::None),
        );
        let mut surface = Surface::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                surface.plot(x, y, Brush::new(id, 190, 0));
            }
        }

        let canvas = surface.resolve(&materials);
        let mut observed: Vec<u8> = canvas.pixels().to_vec();
        observed.sort_unstable();
        observed.dedup();

        assert_eq!(observed.len(), 2, "dithering must mix two adjacent steps");
    }

    #[test]
    fn resolution_is_deterministic() {
        let (palette, materials, id) = material_table();
        let mut surface = Surface::new(6, 6);
        for step in 0..6 {
            surface.plot(step, step, Brush::new(id, step * 150, 0));
        }

        let first = resolve_with_palette(&surface, &materials, &palette);
        let second = resolve_with_palette(&surface, &materials, &palette);

        assert_eq!(first, second);
    }
}
