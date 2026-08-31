//! Hierarchical skeletons, poses, and the humanoid rig used by character sprites.
//!
//! Purpose: own the integer `Skeleton` / `Joint` / `Pose` / `BodyProportions`
//! hierarchy so animation drives exact subpixel geometry.
//! Owns: joint hierarchy, rest pose resolution, `Pose::blended` rotation,
//! and `humanoid_skeleton` proportions; subpixel = 1/16 px.
//! Reads: `math::Angle` and `math::scale` only.
//! Mutates: nothing persistent (pure construction); callers clone poses.
//! Does not own: clip timing, surface shading, or canvas output.
//! Invariants: joint positions in sixteenth-pixel units; small rotations
//! still move limbs predictably before pixel snap; determinism integer-only.
//! Focused tests: `src/art/rig.rs` proportion and pose blending.

use super::math::{Angle, ONE, scale};
use serde::{Deserialize, Serialize};

/// Sub-pixel units per pixel used by rig resolution.
pub const SUBPIXEL: i32 = 16;

/// Largest whole-pixel measurement representable by the rig's sub-pixel coordinate model.
pub const MAX_RIG_PIXELS: i32 = i32::MAX / SUBPIXEL;

/// Largest limb radius that still permits the standard two-radius foot length.
pub const MAX_LIMB_RADIUS_PIXELS: i32 = MAX_RIG_PIXELS / 2;

/// Converts a sub-pixel coordinate to a pixel coordinate.
#[must_use]
pub const fn to_pixels(value: i32) -> i32 {
    value.div_euclid(SUBPIXEL)
}

/// Converts a pixel coordinate to a sub-pixel coordinate.
///
/// # Panics
///
/// Panics when `value` is outside the representable sub-pixel coordinate range.
#[must_use]
pub fn to_subpixels(value: i32) -> i32 {
    scale(value, SUBPIXEL, 1)
}

/// One bone in a skeleton.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Joint {
    name: String,
    parent: Option<usize>,
    length: i32,
    rest_angle: Angle,
    thickness: i32,
    anchor_offset: (i32, i32),
}

impl Joint {
    /// Creates a joint whose length and thickness are measured in sub-pixels.
    ///
    /// # Panics
    ///
    /// Panics when `length` or `thickness` is negative.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        parent: Option<usize>,
        length: i32,
        rest_angle: Angle,
        thickness: i32,
    ) -> Self {
        assert!(length >= 0, "joint length must not be negative");
        assert!(thickness >= 0, "joint thickness must not be negative");
        Self {
            name: name.into(),
            parent,
            length,
            rest_angle,
            thickness,
            anchor_offset: (0, 0),
        }
    }

    /// Returns this joint anchored at a fixed sub-pixel offset from its parent tip.
    ///
    /// Paired limbs use this to separate at the hip and shoulder instead of sharing one point.
    #[must_use]
    pub fn with_anchor_offset(mut self, x: i32, y: i32) -> Self {
        self.anchor_offset = (x, y);
        self
    }

    /// Returns the sub-pixel offset applied to this joint's anchor.
    #[must_use]
    pub const fn anchor_offset(&self) -> (i32, i32) {
        self.anchor_offset
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn parent(&self) -> Option<usize> {
        self.parent
    }

    #[must_use]
    pub const fn length(&self) -> i32 {
        self.length
    }

    #[must_use]
    pub const fn rest_angle(&self) -> Angle {
        self.rest_angle
    }

    /// Returns the bone radius in sub-pixels.
    #[must_use]
    pub const fn thickness(&self) -> i32 {
        self.thickness
    }
}

/// A validated joint hierarchy in parent-before-child order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Skeleton {
    joints: Vec<Joint>,
}

impl Skeleton {
    /// Builds a skeleton from joints listed parent-first.
    ///
    /// # Panics
    ///
    /// Panics when the list is empty or a joint references a parent that is not already defined.
    #[must_use]
    pub fn new(joints: Vec<Joint>) -> Self {
        assert!(!joints.is_empty(), "a skeleton needs at least one joint");
        for (index, joint) in joints.iter().enumerate() {
            if let Some(parent) = joint.parent {
                assert!(
                    parent < index,
                    "joint {index} must reference an earlier parent"
                );
            }
        }
        Self { joints }
    }

    #[must_use]
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    /// Returns the pose in which every joint sits at its rest angle.
    #[must_use]
    pub fn rest_pose(&self) -> Pose {
        Pose::new(self.joints.iter().map(Joint::rest_angle).collect())
    }

    /// Resolves world-space joint segments in sub-pixel units.
    ///
    /// The root joint is placed at `root`, and each joint reports the segment from its parent
    /// anchor to its own tip.
    ///
    /// # Panics
    ///
    /// Panics when `pose` does not have one angle per joint.
    #[must_use]
    pub fn resolve_segments(&self, pose: &Pose, root: (i32, i32)) -> Vec<Segment> {
        assert_eq!(
            pose.len(),
            self.joints.len(),
            "pose must supply one angle per joint"
        );
        let mut segments: Vec<Segment> = Vec::with_capacity(self.joints.len());
        for (index, joint) in self.joints.iter().enumerate() {
            let (anchor, parent_angle) = match joint.parent {
                None => (root, Angle::ZERO),
                Some(parent) => {
                    let parent_segment = &segments[parent];
                    (parent_segment.end, parent_segment.angle)
                }
            };
            let anchor = add_point(anchor, joint.anchor_offset);
            let angle = parent_angle.rotated(i32::from(pose.angle(index).units()));
            let end = add_point(
                anchor,
                (
                    scale(angle.cos(), joint.length, ONE),
                    scale(angle.sin(), joint.length, ONE),
                ),
            );
            segments.push(Segment {
                start: anchor,
                end,
                angle,
                thickness: joint.thickness,
            });
        }
        segments
    }
}

/// A resolved bone in world space, measured in sub-pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: (i32, i32),
    pub end: (i32, i32),
    pub angle: Angle,
    pub thickness: i32,
}

impl Segment {
    /// Returns the segment endpoints snapped to whole pixels.
    #[must_use]
    pub const fn to_pixel_endpoints(self) -> ((i32, i32), (i32, i32)) {
        (
            (to_pixels(self.start.0), to_pixels(self.start.1)),
            (to_pixels(self.end.0), to_pixels(self.end.1)),
        )
    }

    /// Returns the bone radius in whole pixels, never smaller than one.
    #[must_use]
    pub const fn pixel_radius(self) -> i32 {
        let radius = to_pixels(self.thickness);
        if radius < 1 { 1 } else { radius }
    }
}

/// One angle per joint, in parent-relative space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pose {
    angles: Vec<Angle>,
}

impl Pose {
    #[must_use]
    pub fn new(angles: Vec<Angle>) -> Self {
        Self { angles }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.angles.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.angles.is_empty()
    }

    /// Returns the angle for `index`, or zero when the index is unused.
    #[must_use]
    pub fn angle(&self, index: usize) -> Angle {
        self.angles.get(index).copied().unwrap_or(Angle::ZERO)
    }

    /// Sets one joint angle, ignoring unused indexes.
    pub fn set_angle(&mut self, index: usize, angle: Angle) {
        if let Some(slot) = self.angles.get_mut(index) {
            *slot = angle;
        }
    }

    /// Returns this pose with `index` rotated by `degrees`.
    #[must_use]
    pub fn rotated(mut self, index: usize, degrees: i32) -> Self {
        let angle = self
            .angle(index)
            .rotated(i32::from(Angle::from_degrees(degrees).units()));
        self.set_angle(index, angle);
        self
    }

    /// Returns a pose interpolated toward `other` along the shortest rotation.
    ///
    /// # Panics
    ///
    /// Panics when the poses have different joint counts.
    #[must_use]
    pub fn blended(&self, other: &Self, weight: i32) -> Self {
        assert_eq!(
            self.angles.len(),
            other.angles.len(),
            "blended poses must have equal joint counts"
        );
        let weight = weight.clamp(0, 1_000);
        let angles = self
            .angles
            .iter()
            .zip(other.angles.iter())
            .map(|(from, to)| {
                let distance = from.signed_distance_to(*to);
                from.rotated(distance * weight / 1_000)
            })
            .collect();
        Self { angles }
    }
}

/// The named joints of the standard humanoid rig, in resolution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HumanJoint {
    Pelvis,
    Spine,
    Chest,
    Neck,
    Head,
    FarUpperArm,
    FarForearm,
    FarHand,
    NearUpperArm,
    NearForearm,
    NearHand,
    FarThigh,
    FarShin,
    FarFoot,
    NearThigh,
    NearShin,
    NearFoot,
}

impl HumanJoint {
    /// Every joint in rig order.
    pub const ALL: [Self; 17] = [
        Self::Pelvis,
        Self::Spine,
        Self::Chest,
        Self::Neck,
        Self::Head,
        Self::FarUpperArm,
        Self::FarForearm,
        Self::FarHand,
        Self::NearUpperArm,
        Self::NearForearm,
        Self::NearHand,
        Self::FarThigh,
        Self::FarShin,
        Self::FarFoot,
        Self::NearThigh,
        Self::NearShin,
        Self::NearFoot,
    ];

    /// Returns the joint's index in a humanoid skeleton.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pelvis => "pelvis",
            Self::Spine => "spine",
            Self::Chest => "chest",
            Self::Neck => "neck",
            Self::Head => "head",
            Self::FarUpperArm => "far_upper_arm",
            Self::FarForearm => "far_forearm",
            Self::FarHand => "far_hand",
            Self::NearUpperArm => "near_upper_arm",
            Self::NearForearm => "near_forearm",
            Self::NearHand => "near_hand",
            Self::FarThigh => "far_thigh",
            Self::FarShin => "far_shin",
            Self::FarFoot => "far_foot",
            Self::NearThigh => "near_thigh",
            Self::NearShin => "near_shin",
            Self::NearFoot => "near_foot",
        }
    }

    /// Returns whether the joint belongs to the limb nearer the camera.
    #[must_use]
    pub const fn is_near_side(self) -> bool {
        matches!(
            self,
            Self::NearUpperArm
                | Self::NearForearm
                | Self::NearHand
                | Self::NearThigh
                | Self::NearShin
                | Self::NearFoot
        )
    }
}

/// Overall body shape, expressed in whole pixels before sub-pixel conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyProportions {
    /// Standing height from the ground to the top of the head, in pixels.
    pub height: i32,
    /// Head height in pixels, which sets the classic heads-tall ratio.
    pub head_height: i32,
    /// Shoulder half-width in pixels.
    pub shoulder_width: i32,
    /// Hip half-width in pixels.
    pub hip_width: i32,
    /// Limb radius in pixels.
    pub limb_radius: i32,
}

impl BodyProportions {
    /// Returns adult proportions for a sprite of the given pixel height.
    ///
    /// # Panics
    ///
    /// Panics when `height` is below sixteen pixels or cannot be represented by the rig's
    /// sub-pixel coordinate model.
    #[must_use]
    pub fn adult(height: i32) -> Self {
        let proportions = Self {
            height,
            head_height: scale(height, 180, 1_000).max(4),
            shoulder_width: scale(height, 120, 1_000).max(3),
            hip_width: scale(height, 90, 1_000).max(2),
            limb_radius: scale(height, 62, 1_000).max(2),
        };
        proportions.assert_valid();
        proportions
    }

    /// Returns the same proportions with limbs and shoulders scaled by a per-mille factor.
    #[must_use]
    pub fn with_build(mut self, per_mille: i32) -> Self {
        self.assert_valid();
        let per_mille = per_mille.clamp(600, 1_600);
        self.shoulder_width = scale(self.shoulder_width, per_mille, 1_000).max(2);
        self.hip_width = scale(self.hip_width, per_mille, 1_000).max(2);
        self.limb_radius = scale(self.limb_radius, per_mille, 1_000).max(1);
        self.assert_valid();
        self
    }

    fn assert_valid(self) {
        assert!(
            (16..=MAX_RIG_PIXELS).contains(&self.height),
            "humanoid height must be in 16..={MAX_RIG_PIXELS} pixels"
        );
        assert!(
            (1..=MAX_RIG_PIXELS).contains(&self.head_height),
            "head height must be in 1..={MAX_RIG_PIXELS} pixels"
        );
        assert!(
            (0..=MAX_RIG_PIXELS).contains(&self.shoulder_width),
            "shoulder width must be in 0..={MAX_RIG_PIXELS} pixels"
        );
        assert!(
            (0..=MAX_RIG_PIXELS).contains(&self.hip_width),
            "hip width must be in 0..={MAX_RIG_PIXELS} pixels"
        );
        assert!(
            (1..=MAX_LIMB_RADIUS_PIXELS).contains(&self.limb_radius),
            "limb radius must be in 1..={MAX_LIMB_RADIUS_PIXELS} pixels"
        );
    }
}

fn add_point(left: (i32, i32), right: (i32, i32)) -> (i32, i32) {
    let add = |left: i32, right: i32| {
        i32::try_from(i64::from(left) + i64::from(right))
            .expect("resolved joint coordinate must fit i32")
    };
    (add(left.0, right.0), add(left.1, right.1))
}

/// Builds the standard seventeen-joint humanoid skeleton for the given proportions.
#[must_use]
pub fn humanoid_skeleton(proportions: BodyProportions) -> Skeleton {
    proportions.assert_valid();
    let mut joints = torso_bones(proportions);
    joints.extend(arm_bones(proportions, Side::Far));
    joints.extend(arm_bones(proportions, Side::Near));
    joints.extend(leg_bones(proportions, Side::Far));
    joints.extend(leg_bones(proportions, Side::Near));
    Skeleton::new(joints)
}

/// Which of a paired limb is being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Far,
    Near,
}

impl Side {
    const fn sign(self) -> i32 {
        match self {
            Self::Far => -1,
            Self::Near => 1,
        }
    }
}

fn bone(
    name: &str,
    parent: Option<HumanJoint>,
    length: i32,
    angle: Angle,
    thickness: i32,
) -> Joint {
    Joint::new(
        name,
        parent.map(HumanJoint::index),
        to_subpixels(length),
        angle,
        thickness,
    )
}

fn torso_bones(proportions: BodyProportions) -> Vec<Joint> {
    let height = proportions.height;
    let torso = scale(height, 300, 1_000);
    let radius = to_subpixels(proportions.limb_radius);
    let up = Angle::from_degrees(270);
    let down = Angle::ZERO;

    vec![
        bone("pelvis", None, 0, up, scale(radius, 13, 10)),
        bone(
            "spine",
            Some(HumanJoint::Pelvis),
            torso / 2,
            down,
            scale(radius, 15, 10),
        ),
        bone(
            "chest",
            Some(HumanJoint::Spine),
            torso / 2,
            down,
            scale(radius, 16, 10),
        ),
        bone(
            "neck",
            Some(HumanJoint::Chest),
            scale(height, 40, 1_000),
            down,
            radius,
        ),
        bone(
            "head",
            Some(HumanJoint::Neck),
            proportions.head_height,
            down,
            scale(radius, 17, 10),
        ),
    ]
}

fn arm_bones(proportions: BodyProportions, side: Side) -> Vec<Joint> {
    let arm = scale(proportions.height, 400, 1_000);
    let radius = to_subpixels(proportions.limb_radius);
    let offset = scale(
        to_subpixels(scale(proportions.shoulder_width, 9, 10)),
        side.sign(),
        1,
    );
    let (prefix, upper, forearm, spread) = match side {
        Side::Far => ("far", HumanJoint::FarUpperArm, HumanJoint::FarForearm, 190),
        Side::Near => (
            "near",
            HumanJoint::NearUpperArm,
            HumanJoint::NearForearm,
            170,
        ),
    };

    vec![
        bone(
            &format!("{prefix}_upper_arm"),
            Some(HumanJoint::Chest),
            arm / 2,
            Angle::from_degrees(spread),
            scale(radius, 8, 10),
        )
        .with_anchor_offset(offset, 0),
        bone(
            &format!("{prefix}_forearm"),
            Some(upper),
            arm / 2,
            Angle::from_degrees(-side.sign() * 10),
            scale(radius, 7, 10),
        ),
        bone(
            &format!("{prefix}_hand"),
            Some(forearm),
            proportions.limb_radius,
            Angle::ZERO,
            scale(radius, 7, 10),
        ),
    ]
}

fn leg_bones(proportions: BodyProportions, side: Side) -> Vec<Joint> {
    let leg = scale(proportions.height, 470, 1_000);
    let radius = to_subpixels(proportions.limb_radius);
    let offset = scale(
        to_subpixels(scale(proportions.hip_width, 6, 10)),
        side.sign(),
        1,
    );
    let (prefix, thigh, shin, spread) = match side {
        Side::Far => ("far", HumanJoint::FarThigh, HumanJoint::FarShin, 185),
        Side::Near => ("near", HumanJoint::NearThigh, HumanJoint::NearShin, 175),
    };

    vec![
        bone(
            &format!("{prefix}_thigh"),
            Some(HumanJoint::Pelvis),
            leg / 2,
            Angle::from_degrees(spread),
            scale(radius, 11, 10),
        )
        .with_anchor_offset(offset, 0),
        bone(
            &format!("{prefix}_shin"),
            Some(thigh),
            leg / 2,
            Angle::from_degrees(side.sign() * 5),
            scale(radius, 9, 10),
        ),
        bone(
            &format!("{prefix}_foot"),
            Some(shin),
            scale(proportions.limb_radius, 2, 1),
            Angle::from_degrees(-85),
            scale(radius, 8, 10),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton() -> Skeleton {
        humanoid_skeleton(BodyProportions::adult(48))
    }

    #[test]
    fn humanoid_rig_exposes_every_named_joint_in_order() {
        let skeleton = skeleton();

        assert_eq!(skeleton.len(), HumanJoint::ALL.len());
        for joint in HumanJoint::ALL {
            assert!(
                skeleton
                    .joints
                    .iter()
                    .any(|skeleton_joint| skeleton_joint.name == joint.name())
            );
        }
    }

    #[test]
    fn resolved_segments_chain_from_parent_tips() {
        let skeleton = skeleton();
        let segments = skeleton.resolve_segments(&skeleton.rest_pose(), (0, 0));

        assert_eq!(
            segments[HumanJoint::Spine.index()].start,
            segments[HumanJoint::Pelvis.index()].end
        );
        assert_eq!(
            segments[HumanJoint::Chest.index()].start,
            segments[HumanJoint::Spine.index()].end
        );
    }

    #[test]
    fn rest_pose_places_the_head_above_the_pelvis() {
        let skeleton = skeleton();
        let segments = skeleton.resolve_segments(&skeleton.rest_pose(), (0, 0));

        assert!(
            segments[HumanJoint::Head.index()].end.1 < segments[HumanJoint::Pelvis.index()].start.1,
            "the head must resolve above the pelvis"
        );
    }

    #[test]
    fn rest_pose_places_the_feet_below_the_pelvis() {
        let skeleton = skeleton();
        let segments = skeleton.resolve_segments(&skeleton.rest_pose(), (0, 0));

        assert!(segments[HumanJoint::NearFoot.index()].end.1 > 0);
    }

    #[test]
    fn rotating_a_parent_moves_every_descendant() {
        let skeleton = skeleton();
        let rest = skeleton.rest_pose();
        let bent = rest.clone().rotated(HumanJoint::Spine.index(), 30);
        let rest_segments = skeleton.resolve_segments(&rest, (0, 0));
        let bent_segments = skeleton.resolve_segments(&bent, (0, 0));

        assert_ne!(
            rest_segments[HumanJoint::Head.index()].end,
            bent_segments[HumanJoint::Head.index()].end
        );
        assert_eq!(
            rest_segments[HumanJoint::NearFoot.index()].end,
            bent_segments[HumanJoint::NearFoot.index()].end
        );
    }

    #[test]
    fn pose_blending_takes_the_shortest_rotation() {
        let from = Pose::new(vec![Angle::from_degrees(350)]);
        let to = Pose::new(vec![Angle::from_degrees(10)]);

        let middle = from.blended(&to, 500);
        let offset = Angle::from_degrees(0).signed_distance_to(middle.angle(0));

        assert!(offset.abs() <= 2, "midpoint drifted by {offset} units");
    }

    #[test]
    fn segment_radius_is_never_zero() {
        let skeleton = humanoid_skeleton(BodyProportions::adult(16));
        let segments = skeleton.resolve_segments(&skeleton.rest_pose(), (0, 0));

        assert!(segments.iter().all(|segment| segment.pixel_radius() >= 1));
    }

    #[test]
    #[should_panic(expected = "limb radius must be in")]
    fn fabricated_invalid_proportions_are_rejected_at_the_rig_boundary() {
        let mut invalid = BodyProportions::adult(48);
        invalid.limb_radius = MAX_LIMB_RADIUS_PIXELS + 1;

        let _ = humanoid_skeleton(invalid);
    }

    #[test]
    fn segment_resolution_is_deterministic() {
        let skeleton = skeleton();
        let pose = skeleton
            .rest_pose()
            .rotated(HumanJoint::NearThigh.index(), -20);

        assert_eq!(
            skeleton.resolve_segments(&pose, (100, 100)),
            skeleton.resolve_segments(&pose, (100, 100))
        );
    }
}
