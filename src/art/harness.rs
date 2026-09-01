//! The visual review harness: batch sprite generation, automated findings, and a
//! self-contained HTML contact sheet (Artifact Generation profile).
//!
//! Purpose: drive one `ArtReviewConfig` → `ArtReview` → `ArtReviewReport`
//! → HTML/JSON pipeline that is deterministic and self-contained. The
//! semantic input (`ArtReviewConfig` + `CharacterSpec` role seeds) is the
//! source of truth; generated HTML/PNG are derived artifacts written via
//! staged publication (`write_generated_file`) so a failed generation never
//! leaves a plausible partial at the final path.
//! Owns: `ArtReviewConfig` validation, sheet generation, lint invocation,
//! PNG data-URI embedding, and standalone report rendering.
//! Reads: `sprite::CharacterSpec` / `SpriteSheet`, `lint` findings.
//! Mutates: nothing persistent (returns owned `ArtReview` value).
//! Does not own: campaign state, simulation, or persistence.
//! Relevant invariants: every configured seed×role renders; output is one HTML file
//! with no external assets; determinism via integer math only.
//! Canonical operations: `build_art_review`, `build_art_review_report`, `render_art_review_html`.
//! Focused tests: `src/art/harness.rs` batch and encoding checks.

use super::lint::{ArtFinding, ArtSeverity, count_at_least, review_sheet, split_frames};
use super::png::encode_png_data_uri;
use super::sprite::{
    CharacterSpec, MAX_SPRITE_HEIGHT, MIN_SPRITE_HEIGHT, SpriteRole, SpriteSheet,
    render_character_clips,
};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use thiserror::Error;

/// Version of the serialized art review report.
pub const ART_REVIEW_SCHEMA_VERSION: u32 = 1;

/// Smallest magnification supported by the review page.
pub const MIN_REVIEW_SCALE: u32 = 1;

/// Largest magnification supported by the review page.
pub const MAX_REVIEW_SCALE: u32 = 16;

/// Inputs that select which sprites the harness renders.
///
/// Strict deserialization: unknown fields are rejected so a mistyped
/// `art --config` JSON cannot silently produce a truncated review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtReviewConfig {
    pub roles: Vec<SpriteRole>,
    pub start_seed: u64,
    pub seeds: u32,
    pub height: i32,
    /// Magnification used by the harness inspection views.
    pub scale: u32,
}

impl Default for ArtReviewConfig {
    fn default() -> Self {
        Self {
            roles: SpriteRole::ALL.to_vec(),
            start_seed: 1,
            seeds: 2,
            height: 48,
            scale: 6,
        }
    }
}

/// Invalid input for an art review pass.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ArtReviewError {
    #[error("an art review needs at least one role")]
    NoRoles,
    #[error("art review role {role:?} was requested more than once")]
    DuplicateRole { role: SpriteRole },
    #[error("an art review needs at least one seed")]
    NoSeeds,
    #[error("sprite height {height} is outside the supported range {minimum}..={maximum}")]
    InvalidHeight {
        height: i32,
        minimum: i32,
        maximum: i32,
    },
    #[error("review scale {scale} is outside the supported range {minimum}..={maximum}")]
    InvalidScale {
        scale: u32,
        minimum: u32,
        maximum: u32,
    },
    #[error("seed range starting at {start_seed} with {seeds} seeds exceeds u64")]
    SeedRangeOverflow { start_seed: u64, seeds: u32 },
}

/// One generated character and all of its clips.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtSubject {
    pub label: String,
    pub seed: u64,
    pub spec: CharacterSpec,
    pub sheets: Vec<SpriteSheet>,
    pub findings: Vec<ArtFinding>,
}

/// A complete review pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtReview {
    config: ArtReviewConfig,
    subjects: Vec<ArtSubject>,
}

impl ArtReview {
    /// Returns the validated configuration used to build this review.
    #[must_use]
    pub const fn config(&self) -> &ArtReviewConfig {
        &self.config
    }

    /// Returns the generated subjects in deterministic review order.
    #[must_use]
    pub fn subjects(&self) -> &[ArtSubject] {
        &self.subjects
    }

    /// Returns the number of generated subjects.
    #[must_use]
    pub fn subject_count(&self) -> usize {
        self.subjects.len()
    }

    /// Returns every finding across every subject.
    #[must_use]
    pub fn findings(&self) -> Vec<ArtFinding> {
        self.subjects
            .iter()
            .flat_map(|subject| subject.findings.iter().cloned())
            .collect()
    }

    /// Returns the number of findings at or above `severity`.
    #[must_use]
    pub fn count_at_least(&self, severity: ArtSeverity) -> usize {
        self.subjects
            .iter()
            .map(|subject| count_at_least(&subject.findings, severity))
            .sum()
    }
}

/// Renders every configured character and reviews the result.
///
/// # Errors
///
/// Returns [`ArtReviewError`] when roles, seed count/range, sprite height, or review scale are
/// invalid.
pub fn build_art_review(config: ArtReviewConfig) -> Result<ArtReview, ArtReviewError> {
    validate_art_review_config(&config)?;

    let mut subjects = Vec::new();
    for role in &config.roles {
        for offset in 0..config.seeds {
            let seed = config.start_seed.checked_add(u64::from(offset)).ok_or(
                ArtReviewError::SeedRangeOverflow {
                    start_seed: config.start_seed,
                    seeds: config.seeds,
                },
            )?;
            let spec = CharacterSpec::from_seed(seed, *role, config.height);
            let sheets = render_character_clips(spec);
            let findings = sheets.iter().flat_map(review_sheet).collect();
            subjects.push(ArtSubject {
                label: format!("{} #{seed}", role.name()),
                seed,
                spec,
                sheets,
                findings,
            });
        }
    }
    Ok(ArtReview { config, subjects })
}

fn validate_art_review_config(config: &ArtReviewConfig) -> Result<(), ArtReviewError> {
    if config.roles.is_empty() {
        return Err(ArtReviewError::NoRoles);
    }
    for (index, role) in config.roles.iter().enumerate() {
        if config.roles[..index].contains(role) {
            return Err(ArtReviewError::DuplicateRole { role: *role });
        }
    }
    if config.seeds == 0 {
        return Err(ArtReviewError::NoSeeds);
    }
    if !(MIN_SPRITE_HEIGHT..=MAX_SPRITE_HEIGHT).contains(&config.height) {
        return Err(ArtReviewError::InvalidHeight {
            height: config.height,
            minimum: MIN_SPRITE_HEIGHT,
            maximum: MAX_SPRITE_HEIGHT,
        });
    }
    if !(MIN_REVIEW_SCALE..=MAX_REVIEW_SCALE).contains(&config.scale) {
        return Err(ArtReviewError::InvalidScale {
            scale: config.scale,
            minimum: MIN_REVIEW_SCALE,
            maximum: MAX_REVIEW_SCALE,
        });
    }
    config
        .start_seed
        .checked_add(u64::from(config.seeds - 1))
        .ok_or(ArtReviewError::SeedRangeOverflow {
            start_seed: config.start_seed,
            seeds: config.seeds,
        })?;
    Ok(())
}

/// The serializable summary of a review pass.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtReviewReport {
    pub schema_version: u32,
    pub config: ArtReviewConfig,
    pub subjects: Vec<ArtSubjectReport>,
    pub critical_findings: usize,
    pub warning_findings: usize,
}

/// The serializable summary of one subject.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtSubjectReport {
    pub label: String,
    pub seed: u64,
    pub role: SpriteRole,
    pub frame_width: u32,
    pub frame_height: u32,
    pub palette_colors: usize,
    pub clips: Vec<String>,
    pub findings: Vec<ArtFinding>,
}

/// Builds the JSON-facing report for a review pass.
#[must_use]
pub fn build_art_review_report(review: &ArtReview) -> ArtReviewReport {
    let subjects = review
        .subjects
        .iter()
        .map(|subject| {
            let (frame_width, frame_height) = subject.spec.frame_size();
            ArtSubjectReport {
                label: subject.label.clone(),
                seed: subject.seed,
                role: subject.spec.role,
                frame_width,
                frame_height,
                palette_colors: subject
                    .sheets
                    .first()
                    .map_or(0, |sheet| sheet.palette.len()),
                clips: subject
                    .sheets
                    .iter()
                    .map(|sheet| sheet.clip_name.clone())
                    .collect(),
                findings: subject.findings.clone(),
            }
        })
        .collect();
    ArtReviewReport {
        schema_version: ART_REVIEW_SCHEMA_VERSION,
        config: review.config.clone(),
        subjects,
        critical_findings: review.count_at_least(ArtSeverity::Critical),
        warning_findings: review.count_at_least(ArtSeverity::Warning),
    }
}

/// Renders the self-contained HTML review sheet.
#[must_use]
pub fn render_art_review_html(review: &ArtReview) -> String {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str("<title>Sprite review</title>\n<style>\n");
    html.push_str(STYLE);
    html.push_str("</style>\n</head>\n<body>\n");

    let _ = writeln!(
        html,
        "<header><h1>Sprite review</h1><p>{} subjects &middot; {} critical &middot; {} warning or worse &middot; {}px sprites at {}&times;</p>\
<label class=\"toggle\"><input type=\"checkbox\" id=\"silhouette\"> silhouette</label>\
<label class=\"toggle\"><input type=\"checkbox\" id=\"grid\"> pixel grid</label></header>",
        review.subjects.len(),
        review.count_at_least(ArtSeverity::Critical),
        review.count_at_least(ArtSeverity::Warning),
        review.config.height,
        review.config.scale
    );

    for subject in &review.subjects {
        render_subject(&mut html, subject, review.config.scale);
    }

    html.push_str(SCRIPT);
    html.push_str("</body>\n</html>\n");
    html
}

fn render_subject(html: &mut String, subject: &ArtSubject, scale: u32) {
    let _ = write!(
        html,
        "<section class=\"subject\"><h2>{}</h2>",
        escape(&subject.label)
    );
    render_palette(html, subject);
    for sheet in &subject.sheets {
        render_clip(html, sheet, scale);
    }
    render_findings(html, &subject.findings);
    html.push_str("</section>\n");
}

fn render_palette(html: &mut String, subject: &ArtSubject) {
    let Some(sheet) = subject.sheets.first() else {
        return;
    };
    html.push_str("<div class=\"palette\">");
    for color in sheet.palette.colors().iter().skip(1) {
        let _ = write!(
            html,
            "<span class=\"swatch\" style=\"background:{0}\" title=\"{0}\"></span>",
            color.to_hex()
        );
    }
    let _ = write!(
        html,
        "<span class=\"meta\">{} colors &middot; build {}&permil;</span></div>",
        sheet.palette.len(),
        subject.spec.build
    );
}

fn render_clip(html: &mut String, sheet: &SpriteSheet, scale: u32) {
    let sheet_uri = encode_png_data_uri(&sheet.canvas, &sheet.palette);
    let display_width = sheet.frame_width * scale;
    let display_height = sheet.frame_height * scale;
    let duration_ms = sheet.frame_count * 110;

    let _ = write!(
        html,
        "<div class=\"clip\"><h3>{} <span class=\"meta\">{} frames &middot; {}&times;{}</span></h3>\
<div class=\"row\"><div class=\"player\" style=\"width:{display_width}px;height:{display_height}px;\
background-image:url('{sheet_uri}');background-size:{}px {display_height}px;\
animation:play-{} {}ms steps({}) infinite\"></div>",
        escape(&sheet.clip_name),
        sheet.frame_count,
        sheet.frame_width,
        sheet.frame_height,
        sheet.frame_width * sheet.frame_count * scale,
        sheet.frame_count,
        duration_ms,
        sheet.frame_count
    );

    html.push_str("<div class=\"strip\">");
    for (index, frame) in split_frames(sheet).iter().enumerate() {
        let uri = encode_png_data_uri(frame, &sheet.palette);
        let _ = write!(
            html,
            "<figure><img src=\"{uri}\" width=\"{display_width}\" height=\"{display_height}\" alt=\"frame {index}\"><figcaption>{index}</figcaption></figure>"
        );
    }
    html.push_str("</div></div>");

    let _ = write!(
        html,
        "<style>@keyframes play-{} {{ from {{ background-position-x: 0; }} to {{ background-position-x: -{}px; }} }}</style></div>",
        sheet.frame_count,
        sheet.frame_width * sheet.frame_count * scale
    );
}

fn render_findings(html: &mut String, findings: &[ArtFinding]) {
    if findings.is_empty() {
        html.push_str("<p class=\"clean\">No automated findings.</p>");
        return;
    }
    html.push_str("<table class=\"findings\"><thead><tr><th>severity</th><th>check</th><th>clip</th><th>frame</th><th>detail</th></tr></thead><tbody>");
    for finding in findings {
        let _ = write!(
            html,
            "<tr class=\"{0}\"><td>{0}</td><td>{1}</td><td>{2}</td><td>{3}</td><td>{4}</td></tr>",
            finding.severity.name(),
            finding.check.name(),
            escape(&finding.subject),
            finding
                .frame
                .map_or_else(|| "-".to_owned(), |frame| frame.to_string()),
            escape(&finding.detail)
        );
    }
    html.push_str("</tbody></table>");
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const STYLE: &str = "\
:root { color-scheme: dark; }
body { margin: 0; padding: 24px; background: #14151a; color: #d8d8e0;
  font: 14px/1.5 system-ui, sans-serif; }
h1 { margin: 0 0 4px; font-size: 20px; }
h2 { margin: 0 0 8px; font-size: 16px; }
h3 { margin: 0 0 6px; font-size: 13px; font-weight: 600; text-transform: uppercase;
  letter-spacing: 0.08em; color: #9aa0b4; }
header { display: flex; flex-wrap: wrap; gap: 16px; align-items: baseline;
  border-bottom: 1px solid #2a2c36; padding-bottom: 12px; margin-bottom: 20px; }
header p { margin: 0; color: #9aa0b4; }
.toggle { color: #9aa0b4; }
.subject { border: 1px solid #2a2c36; border-radius: 8px; padding: 16px; margin-bottom: 20px;
  background: #1a1b22; }
.clip { margin: 14px 0; }
.row { display: flex; gap: 20px; align-items: flex-start; flex-wrap: wrap; }
.player { image-rendering: pixelated; background-repeat: no-repeat;
  border: 1px solid #2a2c36; border-radius: 4px; background-color: #101116; }
.strip { display: flex; gap: 8px; flex-wrap: wrap; }
figure { margin: 0; text-align: center; }
figcaption { color: #6e7385; font-size: 11px; }
img { image-rendering: pixelated; display: block; border: 1px solid #2a2c36;
  border-radius: 4px; background: #101116; }
.palette { display: flex; align-items: center; gap: 3px; flex-wrap: wrap; margin-bottom: 8px; }
.swatch { width: 16px; height: 16px; border-radius: 3px; border: 1px solid #00000055; }
.meta { color: #6e7385; font-size: 12px; margin-left: 8px; text-transform: none;
  letter-spacing: 0; }
.findings { border-collapse: collapse; width: 100%; margin-top: 10px; font-size: 12px; }
.findings th { text-align: left; color: #6e7385; font-weight: 600; padding: 4px 8px;
  border-bottom: 1px solid #2a2c36; }
.findings td { padding: 4px 8px; border-bottom: 1px solid #23252e; }
.findings tr.critical td:first-child { color: #ff7a7a; }
.findings tr.warning td:first-child { color: #f0b849; }
.findings tr.advisory td:first-child { color: #7aa2ff; }
.clean { color: #6fbf73; font-size: 12px; }
body.silhouette img, body.silhouette .player { filter: brightness(0) invert(1); }
body.grid img { outline: 1px dashed #3a3d4a; }
";

const SCRIPT: &str = "\
<script>
for (const id of ['silhouette', 'grid']) {
  document.getElementById(id).addEventListener('change', (event) => {
    document.body.classList.toggle(id, event.target.checked);
  });
}
</script>
";

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config() -> ArtReviewConfig {
        ArtReviewConfig {
            roles: vec![SpriteRole::Baker],
            start_seed: 4,
            seeds: 1,
            height: 40,
            scale: 4,
        }
    }

    fn review(config: ArtReviewConfig) -> ArtReview {
        build_art_review(config).expect("test config must be valid")
    }

    #[test]
    fn a_review_covers_every_role_and_seed() {
        let config = ArtReviewConfig {
            roles: vec![SpriteRole::Baker, SpriteRole::Official],
            seeds: 3,
            ..small_config()
        };

        let review = review(config);

        assert_eq!(review.subjects.len(), 6);
    }

    #[test]
    fn every_subject_renders_all_standard_clips() {
        let review = review(small_config());

        assert_eq!(review.subjects[0].sheets.len(), 4);
    }

    #[test]
    fn generated_sprites_pass_every_critical_check() {
        let review = review(ArtReviewConfig::default());

        assert_eq!(review.count_at_least(ArtSeverity::Critical), 0);
    }

    #[test]
    fn the_html_sheet_is_self_contained() {
        let review = review(small_config());

        let html = render_art_review_html(&review);

        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("data:image/png;base64,"));
        assert!(!html.contains("src=\"http"));
        assert!(html.ends_with("</html>\n"));
    }

    #[test]
    fn the_html_sheet_labels_every_subject_and_clip() {
        let review = review(small_config());

        let html = render_art_review_html(&review);

        assert!(html.contains("baker #4"));
        for clip in ["idle", "walk", "work", "carry"] {
            assert!(html.contains(clip), "missing clip {clip}");
        }
    }

    #[test]
    fn the_report_carries_a_schema_version_and_findings() {
        let review = review(small_config());

        let report = build_art_review_report(&review);

        assert_eq!(report.schema_version, ART_REVIEW_SCHEMA_VERSION);
        assert_eq!(report.subjects.len(), 1);
        assert_eq!(report.critical_findings, 0);
    }

    #[test]
    fn review_construction_is_deterministic() {
        assert_eq!(
            build_art_review(small_config()),
            build_art_review(small_config())
        );
    }

    #[test]
    fn review_configuration_rejects_invalid_boundaries() {
        let mut config = small_config();
        config.roles.clear();
        assert_eq!(build_art_review(config), Err(ArtReviewError::NoRoles));

        let mut config = small_config();
        config.roles.push(SpriteRole::Baker);
        assert_eq!(
            build_art_review(config),
            Err(ArtReviewError::DuplicateRole {
                role: SpriteRole::Baker,
            })
        );

        let mut config = small_config();
        config.seeds = 0;
        assert_eq!(build_art_review(config), Err(ArtReviewError::NoSeeds));

        let mut config = small_config();
        config.height = MIN_SPRITE_HEIGHT - 1;
        assert!(matches!(
            build_art_review(config),
            Err(ArtReviewError::InvalidHeight { .. })
        ));

        let mut config = small_config();
        config.scale = 0;
        assert!(matches!(
            build_art_review(config),
            Err(ArtReviewError::InvalidScale { .. })
        ));
    }

    #[test]
    fn review_configuration_rejects_seed_range_overflow() {
        let config = ArtReviewConfig {
            start_seed: u64::MAX,
            seeds: 2,
            ..small_config()
        };

        assert_eq!(
            build_art_review(config),
            Err(ArtReviewError::SeedRangeOverflow {
                start_seed: u64::MAX,
                seeds: 2,
            })
        );
    }

    #[test]
    fn html_escaping_neutralizes_markup() {
        assert_eq!(escape("<b>&\"</b>"), "&lt;b&gt;&amp;&quot;&lt;/b&gt;");
    }
}
