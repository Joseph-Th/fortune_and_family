//! Keyframed animation clips and deterministic pose sampling.

use super::math::ease_in_out;
use super::rig::{HumanJoint, Pose, Skeleton, to_subpixels};

/// One authored pose at a frame index, with a root translation in sub-pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyframe {
    pub frame: u32,
    pub pose: Pose,
    pub root_offset: (i32, i32),
}

impl Keyframe {
    #[must_use]
    pub const fn new(frame: u32, pose: Pose, root_offset: (i32, i32)) -> Self {
        Self {
            frame,
            pose,
            root_offset,
        }
    }
}

/// A sampled animation state ready for rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub pose: Pose,
    pub root_offset: (i32, i32),
}

/// A named, fixed-length animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    name: String,
    frame_count: u32,
    looping: bool,
    keys: Vec<Keyframe>,
}

impl Clip {
    /// Builds a clip from keyframes.
    ///
    /// # Panics
    ///
    /// Panics when there are no keyframes, when `frame_count` is zero, when keyframes are not in
    /// strictly increasing frame order, when a keyframe falls outside the clip length, or when
    /// keyframes do not contain the same number of joint angles.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        frame_count: u32,
        looping: bool,
        keys: Vec<Keyframe>,
    ) -> Self {
        assert!(frame_count > 0, "a clip needs at least one frame");
        assert!(!keys.is_empty(), "a clip needs at least one keyframe");
        for pair in keys.windows(2) {
            assert!(
                pair[0].frame < pair[1].frame,
                "keyframes must be strictly increasing"
            );
        }
        let pose_len = keys[0].pose.len();
        assert!(
            keys.iter().all(|key| key.pose.len() == pose_len),
            "keyframes must contain equal-sized poses"
        );
        assert!(
            keys.last().expect("checked above").frame < frame_count,
            "keyframes must fall inside the clip length"
        );
        Self {
            name: name.into(),
            frame_count,
            looping,
            keys,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frame_count
    }

    #[must_use]
    pub const fn is_looping(&self) -> bool {
        self.looping
    }

    /// Samples the clip at `frame`, wrapping when the clip loops and clamping when it does not.
    ///
    /// # Panics
    ///
    /// Panics when the clip has no keyframes, which construction already rejects.
    #[must_use]
    pub fn sample(&self, frame: u32) -> Sample {
        let frame = if self.looping {
            frame % self.frame_count
        } else {
            frame.min(self.frame_count - 1)
        };
        let first = self.keys.first().expect("clips always hold a keyframe");
        let last = self.keys.last().expect("clips always hold a keyframe");

        if self.keys.len() == 1 {
            return Sample {
                pose: first.pose.clone(),
                root_offset: first.root_offset,
            };
        }
        if frame <= first.frame {
            return self.blend(last, first, self.wrap_weight(frame, last, first));
        }
        for pair in self.keys.windows(2) {
            if frame < pair[1].frame {
                let span = pair[1].frame - pair[0].frame;
                let position = frame - pair[0].frame;
                return self.blend(&pair[0], &pair[1], progress(position, span));
            }
        }
        if self.looping {
            let span = self.frame_count - last.frame + first.frame;
            let position = frame - last.frame;
            return self.blend(last, first, progress(position, span));
        }
        Sample {
            pose: last.pose.clone(),
            root_offset: last.root_offset,
        }
    }

    fn wrap_weight(&self, frame: u32, last: &Keyframe, first: &Keyframe) -> i32 {
        if !self.looping || frame >= first.frame {
            return 1_000;
        }
        let span = self.frame_count - last.frame + first.frame;
        let position = self.frame_count - last.frame + frame;
        progress(position, span)
    }

    fn blend(&self, from: &Keyframe, to: &Keyframe, weight: i32) -> Sample {
        let _ = self;
        Sample {
            pose: from.pose.blended(&to.pose, weight),
            root_offset: (
                interpolate_offset(from.root_offset.0, to.root_offset.0, weight),
                interpolate_offset(from.root_offset.1, to.root_offset.1, weight),
            ),
        }
    }
}

fn interpolate_offset(from: i32, to: i32, weight: i32) -> i32 {
    let weight = i64::from(weight.clamp(0, 1_000));
    let value = i64::from(from) + (i64::from(to) - i64::from(from)) * weight / 1_000;
    i32::try_from(value).expect("interpolated root offset must fit i32")
}

fn progress(position: u32, span: u32) -> i32 {
    if span == 0 {
        return 1_000;
    }
    let weight = u64::from(position) * 1_000 / u64::from(span);
    ease_in_out(i32::try_from(weight).expect("clip progress must fit per-mille"))
}

/// The standard clips every humanoid sprite supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HumanClip {
    Idle,
    Walk,
    Work,
    Carry,
}

impl HumanClip {
    pub const ALL: [Self; 4] = [Self::Idle, Self::Walk, Self::Work, Self::Carry];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Work => "work",
            Self::Carry => "carry",
        }
    }
}

/// Builds every standard clip for a humanoid skeleton.
#[must_use]
pub fn humanoid_clip(skeleton: &Skeleton, clip: HumanClip) -> Clip {
    let rest = skeleton.rest_pose();
    match clip {
        HumanClip::Idle => build_idle(&rest),
        HumanClip::Walk => build_walk(&rest),
        HumanClip::Work => build_work(&rest),
        HumanClip::Carry => build_carry(&rest),
    }
}

/// Builds every standard humanoid clip in a stable order.
#[must_use]
pub fn humanoid_clip_library(skeleton: &Skeleton) -> Vec<Clip> {
    HumanClip::ALL
        .into_iter()
        .map(|clip| humanoid_clip(skeleton, clip))
        .collect()
}

fn build_idle(rest: &Pose) -> Clip {
    let settled = rest
        .clone()
        .rotated(HumanJoint::Spine.index(), 2)
        .rotated(HumanJoint::NearUpperArm.index(), 3)
        .rotated(HumanJoint::FarUpperArm.index(), -3)
        .rotated(HumanJoint::Head.index(), 2);
    Clip::new(
        HumanClip::Idle.name(),
        8,
        true,
        vec![
            Keyframe::new(0, rest.clone(), (0, 0)),
            Keyframe::new(4, settled, (0, to_subpixels(1) / 2)),
        ],
    )
}

fn build_walk(rest: &Pose) -> Clip {
    let stride = |near: i32, far: i32, arm: i32| {
        rest.clone()
            .rotated(HumanJoint::NearThigh.index(), near)
            .rotated(HumanJoint::FarThigh.index(), far)
            .rotated(HumanJoint::NearShin.index(), if near > 0 { -18 } else { 6 })
            .rotated(HumanJoint::FarShin.index(), if far > 0 { -18 } else { 6 })
            .rotated(HumanJoint::NearUpperArm.index(), -arm)
            .rotated(HumanJoint::FarUpperArm.index(), arm)
            .rotated(HumanJoint::Spine.index(), 2)
    };
    Clip::new(
        HumanClip::Walk.name(),
        8,
        true,
        vec![
            Keyframe::new(0, stride(22, -22, 16), (0, 0)),
            Keyframe::new(2, rest.clone(), (0, -to_subpixels(1))),
            Keyframe::new(4, stride(-22, 22, -16), (0, 0)),
            Keyframe::new(6, rest.clone(), (0, -to_subpixels(1))),
        ],
    )
}

fn build_work(rest: &Pose) -> Clip {
    let raised = rest
        .clone()
        .rotated(HumanJoint::NearUpperArm.index(), -40)
        .rotated(HumanJoint::NearForearm.index(), -38)
        .rotated(HumanJoint::FarUpperArm.index(), -26)
        .rotated(HumanJoint::FarForearm.index(), -22)
        .rotated(HumanJoint::Spine.index(), -4);
    let struck = rest
        .clone()
        .rotated(HumanJoint::NearUpperArm.index(), 25)
        .rotated(HumanJoint::NearForearm.index(), 10)
        .rotated(HumanJoint::FarUpperArm.index(), 15)
        .rotated(HumanJoint::FarForearm.index(), 10)
        .rotated(HumanJoint::Spine.index(), 10)
        .rotated(HumanJoint::Head.index(), 6);
    Clip::new(
        HumanClip::Work.name(),
        12,
        true,
        vec![
            Keyframe::new(0, raised, (0, 0)),
            Keyframe::new(5, struck, (0, to_subpixels(1))),
            Keyframe::new(8, rest.clone(), (0, 0)),
        ],
    )
}

fn build_carry(rest: &Pose) -> Clip {
    let held = rest
        .clone()
        .rotated(HumanJoint::NearUpperArm.index(), -42)
        .rotated(HumanJoint::NearForearm.index(), -52)
        .rotated(HumanJoint::FarUpperArm.index(), -38)
        .rotated(HumanJoint::FarForearm.index(), -56)
        .rotated(HumanJoint::Spine.index(), -6);
    let shifted = held
        .clone()
        .rotated(HumanJoint::Spine.index(), 3)
        .rotated(HumanJoint::NearThigh.index(), 6)
        .rotated(HumanJoint::FarThigh.index(), -6);
    Clip::new(
        HumanClip::Carry.name(),
        10,
        true,
        vec![
            Keyframe::new(0, held, (0, 0)),
            Keyframe::new(5, shifted, (0, to_subpixels(1) / 2)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::super::math::Angle;
    use super::super::rig::{BodyProportions, humanoid_skeleton};
    use super::*;

    fn skeleton() -> Skeleton {
        humanoid_skeleton(BodyProportions::adult(48))
    }

    #[test]
    fn sampling_a_keyframe_returns_that_pose() {
        let start = Pose::new(vec![Angle::from_degrees(0)]);
        let held = Pose::new(vec![Angle::from_degrees(20)]);
        let clip = Clip::new(
            "test",
            8,
            false,
            vec![
                Keyframe::new(0, start, (0, 0)),
                Keyframe::new(4, held.clone(), (0, 0)),
            ],
        );

        assert_eq!(clip.sample(4).pose, held);
    }

    #[test]
    fn sampling_between_keyframes_stays_between_them() {
        let first = Pose::new(vec![Angle::from_degrees(0)]);
        let second = Pose::new(vec![Angle::from_degrees(40)]);
        let clip = Clip::new(
            "test",
            8,
            false,
            vec![
                Keyframe::new(0, first, (0, 0)),
                Keyframe::new(4, second, (0, 0)),
            ],
        );

        let middle = clip.sample(2).pose.angle(0).to_milli_degrees();

        assert!((0..=40_000).contains(&middle), "sampled {middle}");
    }

    #[test]
    #[should_panic(expected = "keyframes must contain equal-sized poses")]
    fn construction_rejects_mismatched_pose_sizes() {
        let _ = Clip::new(
            "invalid",
            8,
            true,
            vec![
                Keyframe::new(0, Pose::new(vec![Angle::ZERO]), (0, 0)),
                Keyframe::new(4, Pose::new(vec![Angle::ZERO, Angle::ZERO]), (0, 0)),
            ],
        );
    }

    #[test]
    fn large_frame_spans_compute_progress_without_overflow() {
        assert_eq!(progress(u32::MAX / 2, u32::MAX), 498);
    }

    #[test]
    fn root_offset_interpolation_handles_full_i32_range() {
        assert_eq!(interpolate_offset(i32::MIN, i32::MAX, 500), -1);
    }

    #[test]
    fn looping_clips_wrap_back_to_the_first_keyframe() {
        let skeleton = skeleton();
        let clip = humanoid_clip(&skeleton, HumanClip::Walk);

        assert_eq!(clip.sample(0), clip.sample(clip.frame_count()));
    }

    #[test]
    fn non_looping_clips_hold_the_final_keyframe() {
        let pose = Pose::new(vec![Angle::from_degrees(15)]);
        let clip = Clip::new(
            "test",
            6,
            false,
            vec![
                Keyframe::new(0, Pose::new(vec![Angle::ZERO]), (0, 0)),
                Keyframe::new(3, pose.clone(), (0, 0)),
            ],
        );

        assert_eq!(clip.sample(99).pose, pose);
    }

    #[test]
    fn every_standard_clip_is_present_and_looping() {
        let skeleton = skeleton();
        let library = humanoid_clip_library(&skeleton);

        assert_eq!(library.len(), HumanClip::ALL.len());
        assert!(library.iter().all(Clip::is_looping));
        assert!(library.iter().all(|clip| clip.frame_count() >= 8));
    }

    #[test]
    fn walk_swings_the_legs_in_opposition() {
        let skeleton = skeleton();
        let clip = humanoid_clip(&skeleton, HumanClip::Walk);
        let pose = clip.sample(0).pose;
        let rest = skeleton.rest_pose();

        let near = rest
            .angle(HumanJoint::NearThigh.index())
            .signed_distance_to(pose.angle(HumanJoint::NearThigh.index()));
        let far = rest
            .angle(HumanJoint::FarThigh.index())
            .signed_distance_to(pose.angle(HumanJoint::FarThigh.index()));

        assert!(near > 0 && far < 0, "legs must swing in opposition");
    }

    #[test]
    fn sampling_is_deterministic_across_calls() {
        let skeleton = skeleton();
        let clip = humanoid_clip(&skeleton, HumanClip::Work);

        for frame in 0..clip.frame_count() {
            assert_eq!(clip.sample(frame), clip.sample(frame));
        }
    }
}
