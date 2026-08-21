//! Deterministic integer color model, hue-shifted shading ramps, and indexed palettes.
//!
//! All conversions use integer arithmetic so that identical inputs produce identical
//! output on every platform. Hue is expressed in tenth-degrees (`0..3600`); saturation,
//! lightness, and blend weights are expressed in per-mille units (`0..=1000`).

use serde::{Deserialize, Serialize};

/// Reserved palette index used for fully transparent pixels.
pub const TRANSPARENT_INDEX: u8 = 0;

/// Maximum number of colors an indexed palette may hold, including the transparent entry.
pub const MAX_PALETTE_COLORS: usize = 256;

const PER_MILLE: i32 = 1_000;
const HUE_TURN: i32 = 3_600;
const HUE_SECTOR: i32 = 600;

/// An opaque 24-bit color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Rgb8 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb8 {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);

    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// Returns the lowercase `#rrggbb` form used by the review harness.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }

    /// Returns perceptual luminance in per-mille units using integer Rec. 709 weights.
    #[must_use]
    pub fn luminance(self) -> i32 {
        let red = i32::from(self.red);
        let green = i32::from(self.green);
        let blue = i32::from(self.blue);
        (2_126 * red + 7_152 * green + 722 * blue) / (10 * 255)
    }
}

/// A hue, saturation, lightness triple in integer units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hsl {
    /// Hue in tenth-degrees, normalized to `0..3600`.
    pub hue: i32,
    /// Saturation in per-mille, clamped to `0..=1000`.
    pub saturation: i32,
    /// Lightness in per-mille, clamped to `0..=1000`.
    pub lightness: i32,
}

impl Hsl {
    #[must_use]
    pub fn new(hue: i32, saturation: i32, lightness: i32) -> Self {
        Self {
            hue: hue.rem_euclid(HUE_TURN),
            saturation: saturation.clamp(0, PER_MILLE),
            lightness: lightness.clamp(0, PER_MILLE),
        }
    }

    /// Returns the color shifted by hue, saturation, and lightness deltas.
    ///
    /// # Panics
    ///
    /// Panics only if normalized hue or per-mille channel values cannot fit `i32`, which their
    /// bounded ranges prevent.
    #[must_use]
    pub fn shifted(self, hue_delta: i32, saturation_delta: i32, lightness_delta: i32) -> Self {
        let hue = (i64::from(self.hue) + i64::from(hue_delta)).rem_euclid(i64::from(HUE_TURN));
        let saturation = (i64::from(self.saturation) + i64::from(saturation_delta))
            .clamp(0, i64::from(PER_MILLE));
        let lightness =
            (i64::from(self.lightness) + i64::from(lightness_delta)).clamp(0, i64::from(PER_MILLE));
        Self {
            hue: i32::try_from(hue).expect("wrapped hue must fit i32"),
            saturation: i32::try_from(saturation).expect("clamped saturation must fit i32"),
            lightness: i32::try_from(lightness).expect("clamped lightness must fit i32"),
        }
    }

    #[must_use]
    pub fn from_rgb(color: Rgb8) -> Self {
        let red = i32::from(color.red);
        let green = i32::from(color.green);
        let blue = i32::from(color.blue);
        let maximum = red.max(green).max(blue);
        let minimum = red.min(green).min(blue);
        let lightness = (maximum + minimum) * PER_MILLE / 510;
        let span = maximum - minimum;
        if span == 0 {
            return Self::new(0, 0, lightness);
        }
        let denominator = if maximum + minimum <= 255 {
            maximum + minimum
        } else {
            510 - maximum - minimum
        };
        let saturation = span * PER_MILLE / denominator.max(1);
        let hue = if maximum == red {
            (green - blue) * HUE_SECTOR / span
        } else if maximum == green {
            (blue - red) * HUE_SECTOR / span + 2 * HUE_SECTOR
        } else {
            (red - green) * HUE_SECTOR / span + 4 * HUE_SECTOR
        };
        Self::new(hue, saturation, lightness)
    }

    #[must_use]
    pub fn to_rgb(self) -> Rgb8 {
        let normalized = Self::new(self.hue, self.saturation, self.lightness);
        if normalized.saturation == 0 {
            let value = channel_byte(normalized.lightness);
            return Rgb8::new(value, value, value);
        }
        let chroma = (PER_MILLE - (2 * normalized.lightness - PER_MILLE).abs())
            * normalized.saturation
            / PER_MILLE;
        let position = normalized.hue % (2 * HUE_SECTOR);
        let secondary = chroma * (HUE_SECTOR - (position - HUE_SECTOR).abs()) / HUE_SECTOR;
        let base = normalized.lightness - chroma / 2;
        let (red, green, blue) = match normalized.hue / HUE_SECTOR {
            0 => (chroma, secondary, 0),
            1 => (secondary, chroma, 0),
            2 => (0, chroma, secondary),
            3 => (0, secondary, chroma),
            4 => (secondary, 0, chroma),
            _ => (chroma, 0, secondary),
        };
        Rgb8::new(
            channel_byte(base + red),
            channel_byte(base + green),
            channel_byte(base + blue),
        )
    }
}

fn channel_byte(per_mille: i32) -> u8 {
    let value = (per_mille.clamp(0, PER_MILLE) * 255 + PER_MILLE / 2) / PER_MILLE;
    u8::try_from(value).expect("clamped per-mille channel must fit a byte")
}

/// Describes how a base color is expanded into a shading ramp.
///
/// Shadows shift toward the cool side of the spectrum and gain saturation; highlights shift
/// toward the warm side and lose saturation. This hue rotation is what separates hand-authored
/// semi-realistic pixel art from a mechanical lightness gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadeProfile {
    /// Number of ramp steps, from darkest to lightest.
    pub steps: u8,
    /// Tenth-degree hue rotation applied at the darkest step.
    pub shadow_hue_shift: i32,
    /// Tenth-degree hue rotation applied at the lightest step.
    pub highlight_hue_shift: i32,
    /// Per-mille lightness distance from the base color to the darkest step.
    pub shadow_depth: i32,
    /// Per-mille lightness distance from the base color to the lightest step.
    pub highlight_reach: i32,
    /// Per-mille saturation added at the darkest step.
    pub shadow_saturation_gain: i32,
    /// Per-mille saturation removed at the lightest step.
    pub highlight_saturation_loss: i32,
}

impl ShadeProfile {
    /// Returns the default nine-step profile used for skin, cloth, and stone.
    #[must_use]
    pub const fn material() -> Self {
        Self {
            steps: 9,
            shadow_hue_shift: -280,
            highlight_hue_shift: 180,
            shadow_depth: 300,
            highlight_reach: 240,
            shadow_saturation_gain: 180,
            highlight_saturation_loss: 260,
        }
    }

    /// Returns a shorter, higher-contrast profile suited to metal and glass.
    #[must_use]
    pub const fn specular() -> Self {
        Self {
            steps: 7,
            shadow_hue_shift: -420,
            highlight_hue_shift: 120,
            shadow_depth: 380,
            highlight_reach: 420,
            shadow_saturation_gain: 120,
            highlight_saturation_loss: 520,
        }
    }
}

/// An ordered run of colors from darkest to lightest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Ramp {
    colors: Vec<Rgb8>,
}

impl Ramp {
    /// Builds a hue-shifted ramp around `base`.
    ///
    /// # Panics
    ///
    /// Panics when `profile.steps` is less than two.
    #[must_use]
    pub fn build(base: Rgb8, profile: ShadeProfile) -> Self {
        assert!(profile.steps >= 2, "a ramp needs at least two steps");
        let steps = i32::from(profile.steps);
        let midpoint = (steps - 1) / 2;
        let base_hsl = Hsl::from_rgb(base);
        let colors = (0..steps)
            .map(|step| {
                let distance = step - midpoint;
                let (span, hue_shift, lightness_span, saturation_delta) = if distance <= 0 {
                    (
                        midpoint.max(1),
                        i64::from(profile.shadow_hue_shift),
                        -i64::from(profile.shadow_depth),
                        i64::from(profile.shadow_saturation_gain),
                    )
                } else {
                    (
                        (steps - 1 - midpoint).max(1),
                        i64::from(profile.highlight_hue_shift),
                        i64::from(profile.highlight_reach),
                        -i64::from(profile.highlight_saturation_loss),
                    )
                };
                let weight = i64::from(distance.abs() * PER_MILLE / span);
                base_hsl
                    .shifted(
                        scaled_profile_delta(hue_shift, weight),
                        scaled_profile_delta(saturation_delta, weight),
                        scaled_profile_delta(lightness_span, weight),
                    )
                    .to_rgb()
            })
            .collect();
        Self { colors }
    }

    /// Builds a ramp directly from authored colors, darkest first.
    ///
    /// # Panics
    ///
    /// Panics when fewer than two colors are supplied.
    #[must_use]
    pub fn from_colors(colors: Vec<Rgb8>) -> Self {
        assert!(colors.len() >= 2, "a ramp needs at least two steps");
        Self { colors }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    #[must_use]
    pub fn colors(&self) -> &[Rgb8] {
        &self.colors
    }

    /// Returns the color at `step`, clamped to the ramp bounds.
    ///
    /// # Panics
    ///
    /// Panics when the ramp length does not fit `i32`, which construction already rejects.
    #[must_use]
    pub fn color(&self, step: i32) -> Rgb8 {
        let last = i32::try_from(self.colors.len() - 1).expect("ramp length must fit i32");
        let index = usize::try_from(step.clamp(0, last)).expect("clamped step must fit usize");
        self.colors[index]
    }
}

fn scaled_profile_delta(value: i64, weight: i64) -> i32 {
    let scaled = value * weight / i64::from(PER_MILLE);
    i32::try_from(scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
        .expect("clamped profile delta must fit i32")
}

/// Identifies a ramp that has been inserted into a palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RampHandle {
    start: u8,
    length: u8,
}

impl RampHandle {
    #[must_use]
    pub const fn length(self) -> u8 {
        self.length
    }

    /// Returns the palette index for `step`, clamped to the ramp bounds.
    ///
    /// # Panics
    ///
    /// Panics when the clamped step does not fit a byte, which the ramp length prevents.
    #[must_use]
    pub fn index(self, step: i32) -> u8 {
        let last = i32::from(self.length) - 1;
        let offset = u8::try_from(step.clamp(0, last)).expect("clamped step must fit a byte");
        self.start + offset
    }

    /// Returns the darkest palette index in the ramp.
    #[must_use]
    pub const fn darkest(self) -> u8 {
        self.start
    }

    /// Returns the lightest palette index in the ramp.
    #[must_use]
    pub const fn lightest(self) -> u8 {
        self.start + self.length - 1
    }
}

/// An indexed palette whose entry zero is always transparent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Palette {
    colors: Vec<Rgb8>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}

impl Palette {
    #[must_use]
    pub fn new() -> Self {
        Self {
            colors: vec![Rgb8::BLACK],
        }
    }

    /// Appends a ramp and returns its handle.
    ///
    /// # Panics
    ///
    /// Panics when the palette would exceed [`MAX_PALETTE_COLORS`].
    pub fn insert_ramp(&mut self, ramp: &Ramp) -> RampHandle {
        let new_len = self
            .colors
            .len()
            .checked_add(ramp.len())
            .expect("palette length must not overflow usize");
        assert!(
            new_len <= MAX_PALETTE_COLORS,
            "palette must not exceed {MAX_PALETTE_COLORS} colors"
        );
        let start = u8::try_from(self.colors.len()).expect("palette length must fit a byte");
        let length = u8::try_from(ramp.len()).expect("ramp length must fit a byte");
        self.colors.extend_from_slice(ramp.colors());
        RampHandle { start, length }
    }

    /// Appends a single color and returns its index.
    ///
    /// # Panics
    ///
    /// Panics when the palette would exceed [`MAX_PALETTE_COLORS`].
    pub fn insert_color(&mut self, color: Rgb8) -> u8 {
        assert!(
            self.colors.len() < MAX_PALETTE_COLORS,
            "palette must not exceed {MAX_PALETTE_COLORS} colors"
        );
        let index = u8::try_from(self.colors.len()).expect("palette length must fit a byte");
        self.colors.push(color);
        index
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    #[must_use]
    pub fn colors(&self) -> &[Rgb8] {
        &self.colors
    }

    /// Returns the color stored at `index`, or black when the index is unused.
    #[must_use]
    pub fn color(&self, index: u8) -> Rgb8 {
        self.colors
            .get(usize::from(index))
            .copied()
            .unwrap_or(Rgb8::BLACK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trip_preserves_saturated_colors_within_tolerance() {
        for color in [
            Rgb8::new(191, 87, 54),
            Rgb8::new(38, 64, 96),
            Rgb8::new(122, 140, 84),
            Rgb8::new(220, 198, 160),
        ] {
            let restored = Hsl::from_rgb(color).to_rgb();
            assert!(
                i32::from(color.red).abs_diff(i32::from(restored.red)) <= 3
                    && i32::from(color.green).abs_diff(i32::from(restored.green)) <= 3
                    && i32::from(color.blue).abs_diff(i32::from(restored.blue)) <= 3,
                "{color:?} round-tripped to {restored:?}"
            );
        }
    }

    #[test]
    fn grayscale_conversion_reports_zero_saturation() {
        let hsl = Hsl::from_rgb(Rgb8::new(128, 128, 128));

        assert_eq!(hsl.saturation, 0);
    }

    #[test]
    fn public_hsl_values_are_normalized_before_conversion() {
        let unnormalized = Hsl {
            hue: i32::MAX,
            saturation: i32::MAX,
            lightness: i32::MIN,
        };

        assert_eq!(
            unnormalized.to_rgb(),
            Hsl::new(i32::MAX, i32::MAX, i32::MIN).to_rgb()
        );
    }

    #[test]
    fn extreme_shade_profiles_do_not_overflow() {
        let profile = ShadeProfile {
            steps: u8::MAX,
            shadow_hue_shift: i32::MIN,
            highlight_hue_shift: i32::MAX,
            shadow_depth: i32::MIN,
            highlight_reach: i32::MAX,
            shadow_saturation_gain: i32::MAX,
            highlight_saturation_loss: i32::MIN,
        };

        let first = Ramp::build(Rgb8::new(120, 90, 60), profile);
        let second = Ramp::build(Rgb8::new(120, 90, 60), profile);

        assert_eq!(first, second);
        assert_eq!(first.len(), usize::from(u8::MAX));
    }

    #[test]
    fn ramp_steps_increase_in_luminance_from_shadow_to_highlight() {
        let ramp = Ramp::build(Rgb8::new(150, 96, 72), ShadeProfile::material());

        for step in 1..ramp.len() {
            let previous = ramp.color(i32::try_from(step).unwrap() - 1).luminance();
            let current = ramp.color(i32::try_from(step).unwrap()).luminance();
            assert!(
                current > previous,
                "step {step} luminance {current} did not exceed {previous}"
            );
        }
    }

    #[test]
    fn ramp_shadows_rotate_cool_and_highlights_rotate_warm() {
        let base = Rgb8::new(150, 96, 72);
        let ramp = Ramp::build(base, ShadeProfile::material());
        let base_hue = Hsl::from_rgb(base).hue;
        let rotation = |hue: i32| (hue - base_hue + 5_400).rem_euclid(3_600) - 1_800;
        let shadow = rotation(Hsl::from_rgb(ramp.color(0)).hue);
        let highlight = rotation(Hsl::from_rgb(ramp.color(8)).hue);

        assert!(
            shadow < 0,
            "shadow hue must rotate toward cool, got {shadow}"
        );
        assert!(
            highlight > 0,
            "highlight hue must rotate toward warm, got {highlight}"
        );
    }

    #[test]
    fn ramp_construction_is_deterministic() {
        let first = Ramp::build(Rgb8::new(90, 110, 140), ShadeProfile::specular());
        let second = Ramp::build(Rgb8::new(90, 110, 140), ShadeProfile::specular());

        assert_eq!(first, second);
    }

    #[test]
    fn palette_reserves_index_zero_for_transparency() {
        let mut palette = Palette::new();
        let handle = palette.insert_ramp(&Ramp::build(
            Rgb8::new(80, 80, 80),
            ShadeProfile::material(),
        ));

        assert_eq!(handle.darkest(), 1);
        assert_ne!(handle.index(0), TRANSPARENT_INDEX);
    }

    #[test]
    fn ramp_handle_clamps_out_of_range_steps() {
        let mut palette = Palette::new();
        let handle = palette.insert_ramp(&Ramp::build(
            Rgb8::new(80, 80, 80),
            ShadeProfile::material(),
        ));

        assert_eq!(handle.index(-5), handle.darkest());
        assert_eq!(handle.index(99), handle.lightest());
    }
}
