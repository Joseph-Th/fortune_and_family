//! Shaded rasterization primitives that write form into a [`Surface`].
//!
//! Purpose: rasterize capsules, ellipsoids, and polygons with derived normals
//! so volume reads as volume, not flat fill.
//! Owns: `Form` / `Shading`, normal derivation, light mapping, and `Brush`
//! fill of `Surface` (material + light + depth buffers).
//! Reads: `Surface` dimensions, `MaterialTable`; no palette or canvas.
//! Mutates: `Surface` buffers via `fill_*` primitives.
//! Does not own: palette resolution, canvas export, or rig hierarchy.
//! Invariants: every primitive computes a normal from its own geometry;
//! lighting stays integer (`ONE` = 4096); depth respects painter order.
//! Focused tests: `src/art/shape.rs` normal and raster bounds.

use super::math::{ONE, perpendicular_component, scale};
use super::surface::{Brush, MaterialId, Surface};

const PER_MILLE: i32 = 1_000;

/// A unit-ish light direction in fixed point, where `z` points out of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightDirection {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl LightDirection {
    /// Returns a direction normalized to [`ONE`].
    ///
    /// # Panics
    ///
    /// Panics when all three components are zero.
    #[must_use]
    pub fn normalized(x: i32, y: i32, z: i32) -> Self {
        let x = i128::from(x);
        let y = i128::from(y);
        let z = i128::from(z);
        let magnitude = (x * x + y * y + z * z).isqrt();
        assert!(magnitude > 0, "light direction must not be zero");
        let component = |value: i128| {
            i32::try_from(value * i128::from(ONE) / magnitude)
                .expect("normalized light component must fit i32")
        };
        Self {
            x: component(x),
            y: component(y),
            z: component(z),
        }
    }

    /// Returns the conventional key light: above, to the upper left, and slightly forward.
    #[must_use]
    pub fn key() -> Self {
        Self::normalized(-3, -4, 5)
    }
}

/// Converts surface normals into per-mille light values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shading {
    pub light: LightDirection,
    /// Per-mille light applied where the key light contributes nothing.
    pub ambient: i32,
    /// Per-mille light added at full key-light incidence.
    pub gain: i32,
    /// Per-mille light added by the cool fill light opposite the key.
    pub fill: i32,
}

impl Default for Shading {
    fn default() -> Self {
        Self::key()
    }
}

impl Shading {
    #[must_use]
    pub fn key() -> Self {
        Self {
            light: LightDirection::key(),
            ambient: 300,
            gain: 560,
            fill: 110,
        }
    }

    /// Returns the per-mille light for a fixed-point normal.
    ///
    /// # Panics
    ///
    /// Panics only if fixed-point scaling produces a value outside `i32`; the normalized input
    /// range keeps ordinary shading coefficients representable.
    #[must_use]
    pub fn light_for_normal(self, normal_x: i32, normal_y: i32, normal_z: i32) -> i32 {
        let dot = (i128::from(normal_x) * i128::from(self.light.x)
            + i128::from(normal_y) * i128::from(self.light.y)
            + i128::from(normal_z) * i128::from(self.light.z))
            / i128::from(ONE);
        let dot = i32::try_from(dot.clamp(-i128::from(ONE), i128::from(ONE)))
            .expect("clamped normal dot product must fit i32");
        let key = scale(self.gain, dot.max(0), ONE);
        let fill = scale(self.fill, (-dot).max(0), ONE);
        i32::try_from(
            (i64::from(self.ambient) + i64::from(key) + i64::from(fill))
                .clamp(0, i64::from(PER_MILLE)),
        )
        .expect("clamped light must fit i32")
    }
}

/// The material and depth a primitive writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Form {
    pub material: MaterialId,
    pub depth: i16,
    /// Per-mille light offset applied after shading, used for ambient occlusion and accents.
    pub light_bias: i32,
    /// Whether pixels written by this form participate in the material's dithering.
    pub dither: bool,
}

impl Form {
    #[must_use]
    pub const fn new(material: MaterialId, depth: i16) -> Self {
        Self {
            material,
            depth,
            light_bias: 0,
            dither: true,
        }
    }

    #[must_use]
    pub const fn with_light_bias(mut self, light_bias: i32) -> Self {
        self.light_bias = light_bias;
        self
    }

    /// Returns this form with dithering suppressed, for flat panels and hard-edged detail.
    #[must_use]
    pub const fn flat(mut self) -> Self {
        self.dither = false;
        self
    }

    fn brush(self, light: i32) -> Brush {
        let light = i32::try_from(
            (i64::from(light) + i64::from(self.light_bias)).clamp(0, i64::from(PER_MILLE)),
        )
        .expect("clamped form light must fit i32");
        let brush = Brush::new(self.material, light, self.depth);
        if self.dither {
            brush
        } else {
            brush.undithered()
        }
    }
}

/// Fills an axis-aligned rectangle with flat light.
pub fn fill_rect(
    surface: &mut Surface,
    form: Form,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    light: i32,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    let Some((start_x, end_x)) = clipped_span(x, width, surface.width()) else {
        return;
    };
    let Some((start_y, end_y)) = clipped_span(y, height, surface.height()) else {
        return;
    };
    for y in start_y..end_y {
        for x in start_x..end_x {
            surface.plot(x, y, form.brush(light));
        }
    }
}

fn clipped_span(start: i32, length: i32, limit: u32) -> Option<(i32, i32)> {
    if length <= 0 {
        return None;
    }
    let limit = i64::from(limit);
    let start = i64::from(start);
    let end = start + i64::from(length);
    let clipped_start = start.max(0).min(limit);
    let clipped_end = end.max(0).min(limit);
    (clipped_start < clipped_end).then(|| {
        (
            i32::try_from(clipped_start).expect("surface coordinate must fit i32"),
            i32::try_from(clipped_end).expect("surface coordinate must fit i32"),
        )
    })
}

fn clipped_inclusive(minimum: i64, maximum: i64, limit: u32) -> Option<(i32, i32)> {
    let last = i64::from(limit).checked_sub(1)?;
    let minimum = minimum.max(0);
    let maximum = maximum.min(last);
    (minimum <= maximum).then(|| {
        (
            i32::try_from(minimum).expect("surface coordinate must fit i32"),
            i32::try_from(maximum).expect("surface coordinate must fit i32"),
        )
    })
}

/// Fills an ellipsoid, shading it as a solid volume.
///
/// # Panics
///
/// Panics only if fixed-point shading cannot be represented or `surface` violates the signed
/// coordinate invariant established by [`Surface::new`].
pub fn fill_ellipsoid(
    surface: &mut Surface,
    shading: Shading,
    form: Form,
    center_x: i32,
    center_y: i32,
    radius_x: i32,
    radius_y: i32,
) {
    if radius_x <= 0 || radius_y <= 0 {
        return;
    }
    let Some((minimum_x, maximum_x)) = clipped_inclusive(
        i64::from(center_x) - i64::from(radius_x),
        i64::from(center_x) + i64::from(radius_x),
        surface.width(),
    ) else {
        return;
    };
    let Some((minimum_y, maximum_y)) = clipped_inclusive(
        i64::from(center_y) - i64::from(radius_y),
        i64::from(center_y) + i64::from(radius_y),
        surface.height(),
    ) else {
        return;
    };
    for y in minimum_y..=maximum_y {
        for x in minimum_x..=maximum_x {
            let relative_x = i32::try_from(i64::from(x) - i64::from(center_x))
                .expect("ellipsoid x offset must fit i32");
            let relative_y = i32::try_from(i64::from(y) - i64::from(center_y))
                .expect("ellipsoid y offset must fit i32");
            let normal_x = scale(relative_x, ONE, radius_x);
            let normal_y = scale(relative_y, ONE, radius_y);
            let squared = normal_x * normal_x + normal_y * normal_y;
            if squared > ONE * ONE {
                continue;
            }
            let normal_z = perpendicular_component(squared);
            let light = shading.light_for_normal(normal_x, normal_y, normal_z);
            surface.plot(x, y, form.brush(light));
        }
    }
}

/// Fills a capsule between two points, shading it as a cylinder.
///
/// # Panics
///
/// Panics only if fixed-point shading cannot be represented or `surface` violates the signed
/// coordinate invariant established by [`Surface::new`].
pub fn fill_capsule(
    surface: &mut Surface,
    shading: Shading,
    form: Form,
    start: (i32, i32),
    end: (i32, i32),
    radius: i32,
) {
    if radius <= 0 {
        return;
    }
    let axis_x = i64::from(end.0) - i64::from(start.0);
    let axis_y = i64::from(end.1) - i64::from(start.1);
    let axis_length = i64::try_from(
        (i128::from(axis_x) * i128::from(axis_x) + i128::from(axis_y) * i128::from(axis_y)).isqrt(),
    )
    .expect("capsule axis length must fit i64");
    let divisor = axis_length.max(1);
    let unit = |component: i64| {
        i64::try_from(i128::from(component) * i128::from(ONE) / i128::from(divisor))
            .expect("capsule unit component must fit i64")
    };
    let unit_x = unit(axis_x);
    let unit_y = unit(axis_y);
    let radius_i64 = i64::from(radius);
    let Some((minimum_x, maximum_x)) = clipped_inclusive(
        i64::from(start.0.min(end.0)) - radius_i64,
        i64::from(start.0.max(end.0)) + radius_i64,
        surface.width(),
    ) else {
        return;
    };
    let Some((minimum_y, maximum_y)) = clipped_inclusive(
        i64::from(start.1.min(end.1)) - radius_i64,
        i64::from(start.1.max(end.1)) + radius_i64,
        surface.height(),
    ) else {
        return;
    };

    for y in minimum_y..=maximum_y {
        for x in minimum_x..=maximum_x {
            let relative_x = i64::from(x) - i64::from(start.0);
            let relative_y = i64::from(y) - i64::from(start.1);
            let along = i64::try_from(
                (i128::from(relative_x) * i128::from(unit_x)
                    + i128::from(relative_y) * i128::from(unit_y))
                    / i128::from(ONE),
            )
            .expect("capsule projection must fit i64");
            let clamped = along.clamp(0, axis_length);
            let closest_x = i64::from(start.0)
                + i64::try_from(i128::from(unit_x) * i128::from(clamped) / i128::from(ONE))
                    .expect("capsule closest x must fit i64");
            let closest_y = i64::from(start.1)
                + i64::try_from(i128::from(unit_y) * i128::from(clamped) / i128::from(ONE))
                    .expect("capsule closest y must fit i64");
            let offset_x = i64::from(x) - closest_x;
            let offset_y = i64::from(y) - closest_y;
            let distance_squared = i128::from(offset_x) * i128::from(offset_x)
                + i128::from(offset_y) * i128::from(offset_y);
            if distance_squared > i128::from(radius) * i128::from(radius) {
                continue;
            }
            let across = i64::try_from(
                (-i128::from(relative_x) * i128::from(unit_y)
                    + i128::from(relative_y) * i128::from(unit_x))
                    / i128::from(ONE),
            )
            .expect("capsule perpendicular distance must fit i64");
            let normal_scalar = i32::try_from(
                i128::from(across.clamp(-radius_i64, radius_i64)) * i128::from(ONE)
                    / i128::from(radius),
            )
            .expect("capsule normal scalar must fit i32");
            let unit_x = i32::try_from(unit_x).expect("capsule unit x must fit i32");
            let unit_y = i32::try_from(unit_y).expect("capsule unit y must fit i32");
            let normal_x = scale(-unit_y, normal_scalar, ONE);
            let normal_y = scale(unit_x, normal_scalar, ONE);
            let normal_z = perpendicular_component(normal_scalar * normal_scalar);
            let light = shading.light_for_normal(normal_x, normal_y, normal_z);
            surface.plot(x, y, form.brush(light));
        }
    }
}

/// Draws a one-pixel line with flat light.
///
/// # Panics
///
/// Panics only if `surface` violates the signed-coordinate invariant established by
/// [`Surface::new`].
pub fn stroke_line(
    surface: &mut Surface,
    form: Form,
    start: (i32, i32),
    end: (i32, i32),
    light: i32,
) {
    let Some((start, end)) = clip_line_to_surface(surface, start, end) else {
        return;
    };
    let mut x = i64::from(start.0);
    let mut y = i64::from(start.1);
    let end_x = i64::from(end.0);
    let end_y = i64::from(end.1);
    let step_x = if end_x > x { 1 } else { -1 };
    let step_y = if end_y > y { 1 } else { -1 };
    let span_x = (end_x - x).abs();
    let span_y = -(end_y - y).abs();
    let mut error = span_x + span_y;

    loop {
        surface.plot(
            i32::try_from(x).expect("clipped line x must fit i32"),
            i32::try_from(y).expect("clipped line y must fit i32"),
            form.brush(light),
        );
        if x == end_x && y == end_y {
            return;
        }
        let doubled = 2 * error;
        if doubled >= span_y {
            if x == end_x {
                return;
            }
            error += span_y;
            x += step_x;
        }
        if doubled <= span_x {
            if y == end_y {
                return;
            }
            error += span_x;
            y += step_y;
        }
    }
}

fn clip_line_to_surface(
    surface: &Surface,
    start: (i32, i32),
    end: (i32, i32),
) -> Option<((i32, i32), (i32, i32))> {
    const LEFT: u8 = 1;
    const RIGHT: u8 = 2;
    const TOP: u8 = 4;
    const BOTTOM: u8 = 8;

    let max_x = i64::from(surface.width()).checked_sub(1)?;
    let max_y = i64::from(surface.height()).checked_sub(1)?;
    let outcode = |x: i64, y: i64| {
        let mut code = 0;
        if x < 0 {
            code |= LEFT;
        } else if x > max_x {
            code |= RIGHT;
        }
        if y < 0 {
            code |= TOP;
        } else if y > max_y {
            code |= BOTTOM;
        }
        code
    };
    let mut x0 = i64::from(start.0);
    let mut y0 = i64::from(start.1);
    let mut x1 = i64::from(end.0);
    let mut y1 = i64::from(end.1);

    loop {
        let code0 = outcode(x0, y0);
        let code1 = outcode(x1, y1);
        if code0 | code1 == 0 {
            return Some((
                (
                    i32::try_from(x0).expect("clipped line x must fit i32"),
                    i32::try_from(y0).expect("clipped line y must fit i32"),
                ),
                (
                    i32::try_from(x1).expect("clipped line x must fit i32"),
                    i32::try_from(y1).expect("clipped line y must fit i32"),
                ),
            ));
        }
        if code0 & code1 != 0 {
            return None;
        }

        let code = if code0 != 0 { code0 } else { code1 };
        let (x, y) = if code & TOP != 0 {
            (line_intersection(x0, x1, y0, y1, 0), 0)
        } else if code & BOTTOM != 0 {
            (line_intersection(x0, x1, y0, y1, max_y), max_y)
        } else if code & RIGHT != 0 {
            (max_x, line_intersection(y0, y1, x0, x1, max_x))
        } else {
            (0, line_intersection(y0, y1, x0, x1, 0))
        };
        if code == code0 {
            x0 = x;
            y0 = y;
        } else {
            x1 = x;
            y1 = y;
        }
    }
}

fn line_intersection(from: i64, to: i64, axis_from: i64, axis_to: i64, boundary: i64) -> i64 {
    let numerator = i128::from(to - from) * i128::from(boundary - axis_from);
    let denominator = i128::from(axis_to - axis_from);
    assert!(
        denominator != 0,
        "line clipping intersection must have a span"
    );
    i64::try_from(i128::from(from) + numerator / denominator)
        .expect("line clipping coordinate must fit i64")
}

/// Fills a closed polygon using an even-odd scanline rule and flat light.
///
/// # Panics
///
/// Panics only if `surface` violates the signed-coordinate invariant established by
/// [`Surface::new`].
pub fn fill_polygon(surface: &mut Surface, form: Form, points: &[(i32, i32)], light: i32) {
    if points.len() < 3 {
        return;
    }
    let Some((minimum_y, maximum_y)) = clipped_inclusive(
        i64::from(
            points
                .iter()
                .map(|point| point.1)
                .min()
                .expect("polygon has at least three points"),
        ),
        i64::from(
            points
                .iter()
                .map(|point| point.1)
                .max()
                .expect("polygon has at least three points"),
        ),
        surface.height(),
    ) else {
        return;
    };
    let maximum_x = i64::from(surface.width()) - 1;

    for y in minimum_y..=maximum_y {
        let mut crossings = Vec::new();
        for index in 0..points.len() {
            let (start_x, start_y) = points[index];
            let (end_x, end_y) = points[(index + 1) % points.len()];
            if start_y == end_y {
                continue;
            }
            let (low_y, high_y) = (start_y.min(end_y), start_y.max(end_y));
            if y < low_y || y >= high_y {
                continue;
            }
            let crossing = i128::from(start_x)
                + (i128::from(end_x) - i128::from(start_x)) * (i128::from(y) - i128::from(start_y))
                    / (i128::from(end_y) - i128::from(start_y));
            let crossing = i64::try_from(crossing).expect("polygon crossing must fit i64");
            crossings.push(crossing);
        }
        crossings.sort_unstable();
        let (pairs, remainder) = crossings.as_chunks::<2>();
        debug_assert!(remainder.is_empty(), "polygon crossings must pair evenly");
        for pair in pairs {
            let start_x = pair[0].max(0);
            let end_x = pair[1].min(maximum_x);
            if start_x > end_x {
                continue;
            }
            for x in start_x..=end_x {
                surface.plot(
                    i32::try_from(x).expect("polygon x must fit i32"),
                    y,
                    form.brush(light),
                );
            }
        }
    }
}

/// Adds light to drawn pixels whose neighbor in `(step_x, step_y)` is empty and whose current
/// light is at or below `ceiling`.
///
/// Used for rim light along the lit edge and for contact darkening along the ground edge. The
/// ceiling keeps already-bright surfaces from blowing out to the top of their ramp, which is the
/// most common way a procedural rim light reads as noise rather than as light.
///
/// # Panics
///
/// Panics only if `surface` violates the signed-coordinate invariant established by
/// [`Surface::new`].
pub fn add_edge_light(surface: &mut Surface, step_x: i32, step_y: i32, delta: i32, ceiling: i32) {
    let width = i32::try_from(surface.width()).expect("surface width must fit i32");
    let height = i32::try_from(surface.height()).expect("surface height must fit i32");
    let mut targets = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let neighbor = offset_point(x, y, step_x, step_y);
            if surface.material_at(x, y) != 0
                && neighbor.is_none_or(|(neighbor_x, neighbor_y)| {
                    surface.material_at(neighbor_x, neighbor_y) == 0
                })
                && surface.light_at(x, y) <= ceiling
            {
                targets.push((x, y));
            }
        }
    }
    for (x, y) in targets {
        surface.add_light(x, y, delta);
    }
}

/// Darkens drawn pixels that sit beneath another drawn pixel at least `minimum_gap` nearer.
///
/// This approximates contact occlusion where limbs and garments overlap the body. The gap
/// threshold keeps a form from shadowing itself: adjacent pieces of the same volume sit only a
/// step or two apart in depth, and darkening those produces speckle rather than shadow.
///
/// # Panics
///
/// Panics only if `surface` violates the signed-coordinate invariant established by
/// [`Surface::new`].
pub fn add_contact_occlusion(surface: &mut Surface, reach: i32, delta: i32, minimum_gap: i16) {
    if reach <= 0 || delta <= 0 {
        return;
    }
    let width = i32::try_from(surface.width()).expect("surface width must fit i32");
    let height = i32::try_from(surface.height()).expect("surface height must fit i32");
    let effective_reach = reach.min(height);
    let mut targets = Vec::new();
    for y in 0..height {
        for x in 0..width {
            if surface.material_at(x, y) == 0 {
                continue;
            }
            let depth = surface.depth_at(x, y);
            for step in 1..=effective_reach {
                let above_y = y - step;
                if surface.material_at(x, above_y) != 0
                    && surface.depth_at(x, above_y).saturating_sub(depth) >= minimum_gap
                {
                    let falloff = scale(delta, reach - step + 1, reach);
                    targets.push((x, y, falloff));
                    break;
                }
            }
        }
    }
    for (x, y, falloff) in targets {
        surface.add_light(x, y, -falloff);
    }
}

/// Darkens pixels that border a neighbor at least `minimum_gap` nearer in depth.
///
/// Limbs drawn in the same material as the body merge into one silhouette with no internal
/// contour. Seaming the depth discontinuity separates an arm from a torso without an outline
/// color, a second material, or any change to the silhouette.
///
/// # Panics
///
/// Panics only if `surface` violates the signed-coordinate invariant established by
/// [`Surface::new`].
pub fn add_depth_seam(surface: &mut Surface, delta: i32, minimum_gap: i16) {
    if delta <= 0 {
        return;
    }
    let width = i32::try_from(surface.width()).expect("surface width must fit i32");
    let height = i32::try_from(surface.height()).expect("surface height must fit i32");
    let mut targets = Vec::new();
    for y in 0..height {
        for x in 0..width {
            if surface.material_at(x, y) == 0 {
                continue;
            }
            let depth = surface.depth_at(x, y);
            let seamed = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .any(|(step_x, step_y)| {
                    offset_point(x, y, step_x, step_y).is_some_and(|(neighbor_x, neighbor_y)| {
                        surface.material_at(neighbor_x, neighbor_y) != 0
                            && surface
                                .depth_at(neighbor_x, neighbor_y)
                                .saturating_sub(depth)
                                >= minimum_gap
                    })
                });
            if seamed {
                targets.push((x, y));
            }
        }
    }
    for (x, y) in targets {
        surface.add_light(x, y, -delta);
    }
}

fn offset_point(x: i32, y: i32, step_x: i32, step_y: i32) -> Option<(i32, i32)> {
    Some((x.checked_add(step_x)?, y.checked_add(step_y)?))
}

#[cfg(test)]
mod tests {
    use super::super::color::{Palette, Ramp, Rgb8, ShadeProfile};
    use super::super::surface::{Material, MaterialTable, Surface};
    use super::*;

    fn test_material() -> (MaterialTable, MaterialId) {
        let mut palette = Palette::new();
        let ramp = palette.insert_ramp(&Ramp::build(
            Rgb8::new(150, 96, 72),
            ShadeProfile::material(),
        ));
        let mut materials = MaterialTable::new();
        let id = materials.register(Material::new(ramp));
        (materials, id)
    }

    #[test]
    fn normalized_light_has_unit_magnitude() {
        let light = LightDirection::normalized(-3, -4, 5);
        let magnitude = {
            let x = i128::from(light.x);
            let y = i128::from(light.y);
            let z = i128::from(light.z);
            i32::try_from((x * x + y * y + z * z).isqrt()).expect("magnitude must fit i32")
        };

        assert!((magnitude - ONE).abs() <= 64, "magnitude was {magnitude}");
    }

    #[test]
    fn normalized_light_handles_extreme_components() {
        let light = LightDirection::normalized(i32::MIN, i32::MAX, i32::MIN);

        assert!((-ONE..=ONE).contains(&light.x));
        assert!((-ONE..=ONE).contains(&light.y));
        assert!((-ONE..=ONE).contains(&light.z));
    }

    #[test]
    fn shading_brightens_normals_facing_the_key_light() {
        let shading = Shading::key();
        let toward =
            shading.light_for_normal(-shading.light.x.abs(), -shading.light.y.abs(), ONE / 2);
        let away = shading.light_for_normal(shading.light.x.abs(), shading.light.y.abs(), 0);

        assert!(toward > away, "{toward} must exceed {away}");
    }

    #[test]
    fn ellipsoid_light_falls_off_from_the_lit_side_to_the_shadow_side() {
        let (_, id) = test_material();
        let mut surface = Surface::new(32, 32);

        fill_ellipsoid(
            &mut surface,
            Shading::key(),
            Form::new(id, 0),
            16,
            16,
            10,
            10,
        );

        let lit = surface.light_at(11, 11);
        let shadowed = surface.light_at(21, 21);

        assert!(lit > shadowed, "lit {lit} must exceed shadowed {shadowed}");
    }

    #[test]
    fn ellipsoid_stays_inside_its_radii() {
        let (_, id) = test_material();
        let mut surface = Surface::new(32, 32);

        fill_ellipsoid(&mut surface, Shading::key(), Form::new(id, 0), 16, 16, 6, 9);
        let canvas = surface.resolve(&test_material().0);
        let bounds = canvas.opaque_bounds().expect("ellipsoid must draw pixels");

        assert!(bounds.width <= 13 && bounds.height <= 19);
    }

    #[test]
    fn capsule_covers_both_endpoints() {
        let (_, id) = test_material();
        let mut surface = Surface::new(32, 32);

        fill_capsule(
            &mut surface,
            Shading::key(),
            Form::new(id, 0),
            (6, 6),
            (24, 20),
            3,
        );

        assert_ne!(surface.material_at(6, 6), 0);
        assert_ne!(surface.material_at(24, 20), 0);
        assert_eq!(surface.material_at(0, 31), 0);
    }

    #[test]
    fn lines_connect_their_endpoints() {
        let (_, id) = test_material();
        let mut surface = Surface::new(16, 16);

        stroke_line(&mut surface, Form::new(id, 0), (1, 1), (14, 9), 500);

        assert_ne!(surface.material_at(1, 1), 0);
        assert_ne!(surface.material_at(14, 9), 0);
    }

    #[test]
    fn extreme_lines_are_clipped_before_rasterization() {
        let (_, id) = test_material();
        let mut surface = Surface::new(16, 16);

        stroke_line(
            &mut surface,
            Form::new(id, 0),
            (i32::MIN, i32::MIN),
            (i32::MAX, i32::MAX),
            500,
        );

        assert_ne!(surface.material_at(0, 0), 0);
        assert_ne!(surface.material_at(15, 15), 0);
    }

    #[test]
    fn polygons_fill_their_interior_and_exclude_the_exterior() {
        let (_, id) = test_material();
        let mut surface = Surface::new(16, 16);

        fill_polygon(
            &mut surface,
            Form::new(id, 0),
            &[(2, 2), (12, 3), (10, 12), (3, 10)],
            500,
        );

        assert_ne!(surface.material_at(7, 7), 0);
        assert_eq!(surface.material_at(0, 15), 0);
    }

    #[test]
    fn extreme_offscreen_geometry_is_bounded_by_the_surface() {
        let (_, id) = test_material();
        let form = Form::new(id, 0);
        let mut surface = Surface::new(8, 8);

        fill_rect(
            &mut surface,
            form,
            i32::MIN,
            i32::MIN,
            i32::MAX,
            i32::MAX,
            400,
        );
        fill_ellipsoid(
            &mut surface,
            Shading::key(),
            form,
            i32::MIN,
            i32::MIN,
            i32::MAX,
            i32::MAX,
        );
        fill_capsule(
            &mut surface,
            Shading::key(),
            form,
            (i32::MIN, i32::MAX),
            (i32::MAX, i32::MIN),
            i32::MAX,
        );
        fill_polygon(
            &mut surface,
            form,
            &[
                (i32::MIN, i32::MIN),
                (i32::MAX, i32::MIN),
                (i32::MAX, i32::MAX),
                (i32::MIN, i32::MAX),
            ],
            400,
        );

        assert!(surface.material_at(0, 0) != 0 || surface.material_at(7, 7) != 0);
    }

    #[test]
    fn edge_light_only_affects_the_requested_side() {
        let (_, id) = test_material();
        let mut surface = Surface::new(8, 8);
        fill_rect(&mut surface, Form::new(id, 0), 2, 2, 4, 4, 400);

        add_edge_light(&mut surface, 0, -1, 200, 1_000);

        assert_eq!(surface.light_at(3, 2), 600);
        assert_eq!(surface.light_at(3, 5), 400);
    }

    #[test]
    fn depth_seams_separate_touching_forms_of_the_same_material() {
        let (_, id) = test_material();
        let mut surface = Surface::new(10, 10);
        fill_rect(&mut surface, Form::new(id, 0), 1, 1, 8, 8, 600);
        fill_rect(&mut surface, Form::new(id, 40), 6, 1, 3, 8, 600);

        add_depth_seam(&mut surface, 200, 8);

        assert_eq!(surface.light_at(5, 4), 400, "the far side must be seamed");
        assert_eq!(
            surface.light_at(2, 4),
            600,
            "the interior must be untouched"
        );
    }

    #[test]
    fn contact_occlusion_darkens_pixels_under_nearer_geometry() {
        let (_, id) = test_material();
        let mut surface = Surface::new(8, 8);
        fill_rect(&mut surface, Form::new(id, 0), 1, 1, 6, 6, 600);
        fill_rect(&mut surface, Form::new(id, 10), 1, 1, 6, 2, 600);

        add_contact_occlusion(&mut surface, 2, 200, 8);

        assert!(surface.light_at(3, 3) < 600);
        assert_eq!(surface.light_at(3, 6), 600);
    }
}
