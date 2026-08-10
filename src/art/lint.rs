//! Automated review checks that turn common pixel-art defects into reportable findings.
//!
//! The visual harness is only efficient if the obvious failures are found mechanically, leaving
//! human review for judgment calls about form, weight, and readability.

use super::canvas::{Canvas, Rect};
use super::color::{Palette, TRANSPARENT_INDEX};
use super::sprite::SpriteSheet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Upper bound on distinct colors used by one sprite before the palette is flagged.
pub const PALETTE_BUDGET: usize = 40;

/// Minimum luminance span a sprite must cover to read at one-to-one scale.
pub const MINIMUM_LUMINANCE_SPAN: i32 = 260;

/// The class of defect a finding reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ArtCheck {
    /// The sprite touches the edge of its frame and may be clipped.
    FrameClipping,
    /// The silhouette occupies too little or too much of the frame.
    SilhouetteDensity,
    /// An opaque pixel is isolated from the rest of the sprite.
    OrphanPixel,
    /// The sprite uses more distinct colors than the palette budget allows.
    PaletteBudget,
    /// The sprite's lightest and darkest pixels are too close to read at small sizes.
    LuminanceSpan,
    /// Two consecutive frames are identical, so the animation stalls.
    StalledFrame,
    /// The silhouette area changes sharply between frames, which reads as a volume pop.
    VolumeDrift,
    /// The bounding box jumps between frames, which reads as jitter.
    AnchorDrift,
}

impl ArtCheck {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::FrameClipping => "frame-clipping",
            Self::SilhouetteDensity => "silhouette-density",
            Self::OrphanPixel => "orphan-pixel",
            Self::PaletteBudget => "palette-budget",
            Self::LuminanceSpan => "luminance-span",
            Self::StalledFrame => "stalled-frame",
            Self::VolumeDrift => "volume-drift",
            Self::AnchorDrift => "anchor-drift",
        }
    }
}

/// How seriously a finding should be treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ArtSeverity {
    Advisory,
    Warning,
    Critical,
}

impl ArtSeverity {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Advisory => "advisory",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// One reported defect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtFinding {
    pub check: ArtCheck,
    pub severity: ArtSeverity,
    pub subject: String,
    pub frame: Option<u32>,
    pub detail: String,
}

impl ArtFinding {
    fn new(
        check: ArtCheck,
        severity: ArtSeverity,
        subject: &str,
        frame: Option<u32>,
        detail: String,
    ) -> Self {
        Self {
            check,
            severity,
            subject: subject.to_owned(),
            frame,
            detail,
        }
    }
}

/// Reviews every frame of a sheet and returns findings in a stable order.
///
/// # Panics
///
/// Panics only when `sheet` violates the dimension and frame-layout invariants established by
/// the sprite-sheet renderer.
#[must_use]
pub fn review_sheet(sheet: &SpriteSheet) -> Vec<ArtFinding> {
    let frames = split_frames(sheet);
    let mut findings = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let number = u32::try_from(index).expect("frame index must fit u32");
        findings.extend(review_frame(
            frame,
            &sheet.palette,
            &sheet.clip_name,
            number,
        ));
    }
    findings.extend(review_sequence(&frames, &sheet.clip_name, sheet.looping));
    findings.sort_by(|left, right| {
        right
            .severity
            .cmp(&left.severity)
            .then(left.check.cmp(&right.check))
            .then(left.frame.cmp(&right.frame))
    });
    findings
}

/// Splits a horizontal sheet into its individual frames.
///
/// # Panics
///
/// Panics only when `sheet` violates the dimension and frame-layout invariants established by
/// the sprite-sheet renderer.
#[must_use]
pub fn split_frames(sheet: &SpriteSheet) -> Vec<Canvas> {
    (0..sheet.frame_count)
        .map(|index| {
            let mut frame = Canvas::new(sheet.frame_width, sheet.frame_height);
            let origin = i64::from(index) * i64::from(sheet.frame_width);
            for y in 0..i32::try_from(sheet.frame_height).expect("frame height must fit i32") {
                for x in 0..i32::try_from(sheet.frame_width).expect("frame width must fit i32") {
                    let source_x =
                        i32::try_from(origin + i64::from(x)).expect("frame origin must fit i32");
                    frame.set(x, y, sheet.canvas.get(source_x, y));
                }
            }
            frame
        })
        .collect()
}

fn review_frame(canvas: &Canvas, palette: &Palette, subject: &str, frame: u32) -> Vec<ArtFinding> {
    let mut findings = Vec::new();
    let Some(bounds) = canvas.opaque_bounds() else {
        findings.push(ArtFinding::new(
            ArtCheck::SilhouetteDensity,
            ArtSeverity::Critical,
            subject,
            Some(frame),
            "the frame is empty".to_owned(),
        ));
        return findings;
    };

    let sides = touched_borders(canvas, bounds);
    if !sides.is_empty() {
        findings.push(ArtFinding::new(
            ArtCheck::FrameClipping,
            ArtSeverity::Critical,
            subject,
            Some(frame),
            format!(
                "the silhouette reaches the {} frame border",
                sides.join(" and ")
            ),
        ));
    }

    let area = usize::try_from(canvas.width())
        .ok()
        .and_then(|width| {
            usize::try_from(canvas.height())
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .expect("canvas area must fit usize");
    let coverage = percentage(canvas.opaque_count(), area.max(1));
    if !(10..=70).contains(&coverage) {
        findings.push(ArtFinding::new(
            ArtCheck::SilhouetteDensity,
            ArtSeverity::Warning,
            subject,
            Some(frame),
            format!("the silhouette covers {coverage}% of the frame"),
        ));
    }

    let orphans = count_orphans(canvas);
    if orphans > 0 {
        findings.push(ArtFinding::new(
            ArtCheck::OrphanPixel,
            ArtSeverity::Warning,
            subject,
            Some(frame),
            format!("{orphans} pixels are isolated from the silhouette"),
        ));
    }

    let used = used_indexes(canvas);
    if used.len() > PALETTE_BUDGET {
        findings.push(ArtFinding::new(
            ArtCheck::PaletteBudget,
            ArtSeverity::Advisory,
            subject,
            Some(frame),
            format!(
                "{} distinct colors exceed the budget of {PALETTE_BUDGET}",
                used.len()
            ),
        ));
    }

    if let Some(span) = luminance_span(&used, palette)
        && span < MINIMUM_LUMINANCE_SPAN
    {
        findings.push(ArtFinding::new(
            ArtCheck::LuminanceSpan,
            ArtSeverity::Warning,
            subject,
            Some(frame),
            format!("luminance spans only {span} per-mille"),
        ));
    }

    findings
}

fn review_sequence(frames: &[Canvas], subject: &str, looping: bool) -> Vec<ArtFinding> {
    let mut findings = Vec::new();
    if frames.len() < 2 {
        return findings;
    }
    for (index, pair) in frames.windows(2).enumerate() {
        findings.extend(review_transition(&pair[0], &pair[1], subject, index));
    }
    if looping {
        findings.extend(review_transition(
            frames.last().expect("checked frame count"),
            frames.first().expect("checked frame count"),
            subject,
            frames.len() - 1,
        ));
    }
    findings
}

fn review_transition(
    current: &Canvas,
    next: &Canvas,
    subject: &str,
    index: usize,
) -> Vec<ArtFinding> {
    let mut findings = Vec::new();
    let number = u32::try_from(index).expect("frame index must fit u32");

    if current == next {
        findings.push(ArtFinding::new(
            ArtCheck::StalledFrame,
            ArtSeverity::Advisory,
            subject,
            Some(number),
            "this frame is identical to the next one".to_owned(),
        ));
    }

    let current_area = current.opaque_count();
    let next_area = next.opaque_count();
    let change = percentage(current_area.abs_diff(next_area), current_area.max(1));
    if change > 20 {
        findings.push(ArtFinding::new(
            ArtCheck::VolumeDrift,
            ArtSeverity::Warning,
            subject,
            Some(number),
            format!("the silhouette area changes {change}% before the next frame"),
        ));
    }

    if let (Some(current_anchor), Some(next_anchor)) = (ground_anchor(current), ground_anchor(next))
    {
        let drift = (current_anchor - next_anchor).abs();
        if drift > 2 {
            findings.push(ArtFinding::new(
                ArtCheck::AnchorDrift,
                ArtSeverity::Warning,
                subject,
                Some(number),
                format!("the ground contact shifts {drift} pixels horizontally"),
            ));
        }
    }
    findings
}

/// Returns the horizontal center of the lowest opaque row, which stands in for the point where
/// the subject meets the ground.
///
/// # Panics
///
/// Panics only when `canvas` violates the dimension invariants established by [`Canvas::new`].
#[must_use]
pub fn ground_anchor(canvas: &Canvas) -> Option<i32> {
    let bounds = canvas.opaque_bounds()?;
    let row = bounds.y + i32::try_from(bounds.height).expect("opaque height must fit i32") - 1;
    let columns: Vec<i32> = (0..i32::try_from(canvas.width()).expect("canvas width must fit i32"))
        .filter(|x| canvas.get(*x, row) != TRANSPARENT_INDEX)
        .collect();
    if columns.is_empty() {
        return None;
    }
    let total: i64 = columns.iter().map(|column| i64::from(*column)).sum();
    let average = total / i64::try_from(columns.len()).expect("column count must fit i64");
    Some(i32::try_from(average).expect("ground anchor must fit i32"))
}

fn touched_borders(canvas: &Canvas, bounds: Rect) -> Vec<&'static str> {
    let right = u32::try_from(bounds.x)
        .expect("opaque x must be nonnegative")
        .checked_add(bounds.width)
        .expect("opaque right edge must fit u32");
    let bottom = u32::try_from(bounds.y)
        .expect("opaque y must be nonnegative")
        .checked_add(bounds.height)
        .expect("opaque bottom edge must fit u32");
    let mut sides = Vec::new();
    if bounds.y == 0 {
        sides.push("top");
    }
    if bottom >= canvas.height() {
        sides.push("bottom");
    }
    if bounds.x == 0 {
        sides.push("left");
    }
    if right >= canvas.width() {
        sides.push("right");
    }
    sides
}

fn count_orphans(canvas: &Canvas) -> usize {
    let mut orphans = 0;
    for y in 0..i32::try_from(canvas.height()).expect("canvas height must fit i32") {
        for x in 0..i32::try_from(canvas.width()).expect("canvas width must fit i32") {
            if canvas.get(x, y) == TRANSPARENT_INDEX {
                continue;
            }
            let neighbors = [(0, -1), (0, 1), (-1, 0), (1, 0)]
                .into_iter()
                .filter(|(step_x, step_y)| canvas.get(x + step_x, y + step_y) != TRANSPARENT_INDEX)
                .count();
            if neighbors == 0 {
                orphans += 1;
            }
        }
    }
    orphans
}

/// Returns every palette index the canvas actually uses, excluding transparency.
#[must_use]
pub fn used_indexes(canvas: &Canvas) -> BTreeSet<u8> {
    canvas
        .pixels()
        .iter()
        .copied()
        .filter(|index| *index != TRANSPARENT_INDEX)
        .collect()
}

fn luminance_span(used: &BTreeSet<u8>, palette: &Palette) -> Option<i32> {
    let mut minimum = i32::MAX;
    let mut maximum = i32::MIN;
    for index in used {
        let luminance = palette.color(*index).luminance();
        minimum = minimum.min(luminance);
        maximum = maximum.max(luminance);
    }
    if maximum < minimum {
        return None;
    }
    Some(maximum - minimum)
}

/// Returns the count of findings at or above `severity`.
#[must_use]
pub fn count_at_least(findings: &[ArtFinding], severity: ArtSeverity) -> usize {
    findings
        .iter()
        .filter(|finding| finding.severity >= severity)
        .count()
}

fn percentage(numerator: usize, denominator: usize) -> usize {
    let value = u128::try_from(numerator).expect("usize must fit u128") * 100
        / u128::try_from(denominator).expect("usize must fit u128");
    usize::try_from(value).expect("percentage must fit usize")
}

#[cfg(test)]
mod tests {
    use super::super::anim::{HumanClip, humanoid_clip};
    use super::super::color::{Ramp, Rgb8, ShadeProfile};
    use super::super::sprite::{
        CharacterRole, CharacterSpec, render_character_clips, render_character_sheet,
    };
    use super::*;

    fn flat_sheet(fill: u8, frames: u32) -> SpriteSheet {
        let mut palette = Palette::new();
        palette.insert_ramp(&Ramp::build(
            Rgb8::new(120, 120, 120),
            ShadeProfile::material(),
        ));
        let mut canvas = Canvas::new(8 * frames, 8);
        for y in 2..6 {
            for x in 0..i32::try_from(8 * frames).unwrap_or(0) {
                if x % 8 >= 2 && x % 8 < 6 {
                    canvas.set(x, y, fill);
                }
            }
        }
        SpriteSheet {
            clip_name: "test".to_owned(),
            looping: true,
            frame_width: 8,
            frame_height: 8,
            frame_count: frames,
            canvas,
            palette,
        }
    }

    #[test]
    fn empty_frames_are_reported_as_critical() {
        let mut sheet = flat_sheet(1, 1);
        sheet.canvas.clear();

        let findings = review_sheet(&sheet);

        assert!(
            findings
                .iter()
                .any(|finding| finding.severity == ArtSeverity::Critical)
        );
    }

    #[test]
    fn clipped_silhouettes_are_reported() {
        let mut sheet = flat_sheet(1, 1);
        sheet.canvas.fill_rect(Rect::new(0, 0, 8, 8), 1);

        let findings = review_sheet(&sheet);

        assert!(
            findings
                .iter()
                .any(|finding| finding.check == ArtCheck::FrameClipping)
        );
    }

    #[test]
    fn isolated_pixels_are_reported() {
        let mut sheet = flat_sheet(1, 1);
        sheet.canvas.set(7, 0, 2);

        let findings = review_sheet(&sheet);

        assert!(
            findings
                .iter()
                .any(|finding| finding.check == ArtCheck::OrphanPixel)
        );
    }

    #[test]
    fn identical_consecutive_frames_are_reported() {
        let sheet = flat_sheet(1, 2);

        let findings = review_sheet(&sheet);

        assert!(
            findings
                .iter()
                .any(|finding| finding.check == ArtCheck::StalledFrame)
        );
    }

    #[test]
    fn non_looping_sequences_do_not_compare_the_last_frame_to_the_first() {
        let mut first = Canvas::new(8, 8);
        first.fill_rect(Rect::new(2, 2, 3, 4), 1);
        let mut middle = Canvas::new(8, 8);
        middle.fill_rect(Rect::new(3, 2, 3, 4), 1);
        let frames = vec![first.clone(), middle, first];

        let non_looping = review_sequence(&frames, "test", false);
        let looping = review_sequence(&frames, "test", true);

        assert!(!non_looping.iter().any(|finding| {
            finding.check == ArtCheck::StalledFrame && finding.frame == Some(2)
        }));
        assert!(looping.iter().any(|finding| {
            finding.check == ArtCheck::StalledFrame && finding.frame == Some(2)
        }));
    }

    #[test]
    fn frames_split_at_the_declared_width() {
        let sheet = flat_sheet(1, 3);

        let frames = split_frames(&sheet);

        assert_eq!(frames.len(), 3);
        assert!(frames.iter().all(|frame| frame.width() == 8));
    }

    #[test]
    fn generated_characters_report_no_critical_findings() {
        for role in CharacterRole::ALL {
            let spec = CharacterSpec::from_seed(5, role, 48);
            for sheet in render_character_clips(spec) {
                let findings = review_sheet(&sheet);
                let critical = count_at_least(&findings, ArtSeverity::Critical);

                assert_eq!(
                    critical,
                    0,
                    "{} {} reported {critical} critical findings: {findings:?}",
                    role.name(),
                    sheet.clip_name
                );
            }
        }
    }

    #[test]
    fn review_output_is_deterministic() {
        let spec = CharacterSpec::from_seed(9, CharacterRole::Merchant, 48);
        let clip = humanoid_clip(&spec.skeleton(), HumanClip::Walk);
        let sheet = render_character_sheet(spec, &clip);

        assert_eq!(review_sheet(&sheet), review_sheet(&sheet));
    }
}
