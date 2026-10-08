//! Boost pads (`RB-PHYSICS-001-FR-149`, ADR-0072).
//!
//! Soccar's 34 pads, six big and 28 small, in the game's order (by y, then
//! x). The pickup rule is fitted to the game's own recordings
//! (`tools/rb_tape_bot/pad_fit.py` on 153 runs, 63 small and 24 big pickups and
//! about 11800 passes without one; RB-RESEARCH-O021):
//!
//! - a pad is taken when the car's *origin* (not its hitbox centre) is within
//!   [`SMALL_PAD_RADIUS`] / [`BIG_PAD_RADIUS`] of the pad's centre in the plane
//!   and no more than [`PAD_HEIGHT`] above the pad;
//! - the position tested is 0.6 of the way from where the car was at the start of
//!   the tick to where it ends it ([`PAD_TEST_FRACTION`]): a joint fit of the test
//!   point and both radii on the 34 probe recordings puts 20 of 21 pickups on the
//!   game's tick (0.25, the first fit from the logs, left 5 late); the radii kept
//!   their values;
//! - a small pad gives 12 boost and a big pad fills the tank; a car with a full
//!   tank does not use the pad up;
//! - a taken pad is back after 4 s (small) or 10 s (big), counted in ticks.
//!
//! Not measured, so assumed: a small pad stops at a full tank
//! (`min(boost + 12, 100)`), the first car in index order wins a pad two cars
//! reach in one tick, and a pad has no lower height limit.

use rb_domain::Vec3;

use crate::drive::MAX_BOOST;

/// Planar reach of a small pad from the car's origin, uu (fitted: 175.8 to 176.5).
pub const SMALL_PAD_RADIUS: f32 = 176.0;
/// Planar reach of a big pad from the car's origin, uu (fitted: 207.7; RocketSim's 208).
pub const BIG_PAD_RADIUS: f32 = 208.0;
/// How far above a pad's centre a car's origin still takes it, uu (fitted 157 to 163,
/// the same for both sizes).
pub const PAD_HEIGHT: f32 = 160.0;
/// Where along the tick the pad test reads the car: this fraction of the way
/// from its start-of-tick position to its end-of-tick position (0.6: about 3.6 uu
/// short of the end position at a probe's 9 uu per tick).
pub const PAD_TEST_FRACTION: f32 = 0.6;
/// Boost a small pad gives.
pub const SMALL_PAD_BOOST: f32 = 12.0;
/// Seconds before a taken small pad is back.
pub const SMALL_PAD_COOLDOWN_SECS: f32 = 4.0;
/// Seconds before a taken big pad is back.
pub const BIG_PAD_COOLDOWN_SECS: f32 = 10.0;
/// A pad whose remaining cooldown is under this is active again (absorbs the
/// rounding of 480 subtractions of 1/120).
const COOLDOWN_EPSILON_SECS: f32 = 1.0e-3;

/// One pad: where it is and whether it is a big one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoostPad {
    pub position: Vec3,
    pub big: bool,
}

impl BoostPad {
    fn radius(&self) -> f32 {
        if self.big {
            BIG_PAD_RADIUS
        } else {
            SMALL_PAD_RADIUS
        }
    }

    fn cooldown_secs(&self) -> f32 {
        if self.big {
            BIG_PAD_COOLDOWN_SECS
        } else {
            SMALL_PAD_COOLDOWN_SECS
        }
    }

    /// Whether a car whose origin is at `origin` reaches this pad.
    fn reaches(&self, origin: Vec3) -> bool {
        let dx = origin.x - self.position.x;
        let dy = origin.y - self.position.y;
        let radius = self.radius();
        dx * dx + dy * dy <= radius * radius && origin.z - self.position.z <= PAD_HEIGHT
    }
}

/// A set of pads and how long each has left before it is back.
#[derive(Debug, Clone, PartialEq)]
pub struct BoostPads {
    pads: Vec<BoostPad>,
    /// Seconds left before pad `i` is back; 0 for an active pad.
    cooldown: Vec<f32>,
}

impl BoostPads {
    /// The pads of `pads`, all active.
    pub fn new(pads: Vec<BoostPad>) -> BoostPads {
        let cooldown = vec![0.0; pads.len()];
        BoostPads { pads, cooldown }
    }

    /// Soccar's 34 pads in the game's order (by y, then x), all active.
    pub fn standard() -> BoostPads {
        const SMALL_Z: f32 = 0.082_016;
        const BIG_Z: f32 = 8.0;
        let small = |x: f32, y: f32| BoostPad {
            position: Vec3::new(x, y, SMALL_Z),
            big: false,
        };
        let big = |x: f32, y: f32| BoostPad {
            position: Vec3::new(x, y, BIG_Z),
            big: true,
        };
        BoostPads::new(vec![
            small(0.0, -4240.0),
            small(-1792.0, -4184.0),
            small(1792.0, -4184.0),
            big(-3072.0, -4096.0),
            big(3072.0, -4096.0),
            small(-940.0, -3308.0),
            small(940.0, -3308.0),
            small(0.0, -2816.0),
            small(-3584.0, -2484.0),
            small(3584.0, -2484.0),
            small(-1788.0, -2302.0),
            small(1788.0, -2302.0),
            small(-2048.0, -1036.0),
            small(2048.0, -1036.0),
            small(0.0, -1024.0),
            big(-3584.0, 0.0),
            small(-1024.0, 0.0),
            small(1024.0, 0.0),
            big(3584.0, 0.0),
            small(0.0, 1024.0),
            small(-2048.0, 1036.0),
            small(2048.0, 1036.0),
            small(-1788.0, 2302.0),
            small(1788.0, 2302.0),
            small(-3584.0, 2484.0),
            small(3584.0, 2484.0),
            small(0.0, 2816.0),
            small(-940.0, 3308.0),
            small(940.0, 3308.0),
            big(-3072.0, 4096.0),
            big(3072.0, 4096.0),
            small(-1792.0, 4184.0),
            small(1792.0, 4184.0),
            small(0.0, 4240.0),
        ])
    }

    /// How many pads there are.
    pub fn len(&self) -> usize {
        self.pads.len()
    }

    /// Whether there are no pads.
    pub fn is_empty(&self) -> bool {
        self.pads.is_empty()
    }

    /// Pad `index`.
    pub fn pad(&self, index: usize) -> Option<&BoostPad> {
        self.pads.get(index)
    }

    /// Whether pad `index` can be taken now (false for an unknown index).
    pub fn is_active(&self, index: usize) -> bool {
        self.cooldown
            .get(index)
            .is_some_and(|left| *left <= COOLDOWN_EPSILON_SECS)
    }

    /// Seconds before pad `index` is back (0 when active).
    pub fn cooldown_left(&self, index: usize) -> f32 {
        self.cooldown.get(index).copied().unwrap_or(0.0).max(0.0)
    }

    /// Counts every taken pad's cooldown down by one tick.
    pub fn tick(&mut self, dt: f32) {
        for left in &mut self.cooldown {
            if *left > 0.0 {
                *left = (*left - dt).max(0.0);
            }
        }
    }

    /// Lets a car whose origin went from `start` to `end` this tick take the
    /// first active pad it reaches. Returns the boost the car has afterwards
    /// (`boost` unchanged if it took nothing).
    pub fn collect(&mut self, start: Vec3, end: Vec3, boost: f32) -> f32 {
        if boost >= MAX_BOOST {
            return boost;
        }
        let tested = start + (end - start) * PAD_TEST_FRACTION;
        for index in 0..self.pads.len() {
            if !self.is_active(index) || !self.pads[index].reaches(tested) {
                continue;
            }
            self.cooldown[index] = self.pads[index].cooldown_secs();
            return if self.pads[index].big {
                MAX_BOOST
            } else {
                (boost + SMALL_PAD_BOOST).min(MAX_BOOST)
            };
        }
        boost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 120.0;

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// Pad 14: the small pad at (0, -1024).
    fn small_pad_world() -> BoostPads {
        BoostPads::standard()
    }

    #[test]
    fn the_standard_set_is_the_games_34_pads_six_of_them_big() {
        let pads = BoostPads::standard();
        assert_eq!(pads.len(), 34);
        assert_eq!(
            (0..34)
                .filter(|i| pads.pad(*i).is_some_and(|p| p.big))
                .count(),
            6
        );
        assert!((0..34).all(|i| pads.is_active(i)));
        // The game's order is by y, then x.
        let keys: Vec<(f32, f32)> = (0..34)
            .filter_map(|i| pads.pad(i))
            .map(|p| (p.position.y, p.position.x))
            .collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
    }

    #[test]
    fn a_small_pad_gives_twelve_and_goes_inactive() {
        let mut pads = small_pad_world();
        let boost = pads.collect(at(0.0, -1030.0, 17.0), at(0.0, -1028.0, 17.0), 30.0);
        assert_eq!(boost, 42.0);
        assert!(!pads.is_active(14));
        // The same spot a tick later: used up.
        assert_eq!(
            pads.collect(at(0.0, -1030.0, 17.0), at(0.0, -1028.0, 17.0), boost),
            42.0
        );
    }

    #[test]
    fn a_big_pad_fills_the_tank() {
        let mut pads = BoostPads::standard();
        let boost = pads.collect(at(3584.0, 0.0, 17.0), at(3584.0, 4.0, 17.0), 5.0);
        assert_eq!(boost, MAX_BOOST);
        assert!(!pads.is_active(18));
    }

    #[test]
    fn a_full_tank_leaves_the_pad_alone() {
        let mut pads = small_pad_world();
        let boost = pads.collect(at(0.0, -1030.0, 17.0), at(0.0, -1028.0, 17.0), MAX_BOOST);
        assert_eq!(boost, MAX_BOOST);
        assert!(pads.is_active(14));
    }

    #[test]
    fn a_small_pad_cannot_overfill_the_tank() {
        let mut pads = small_pad_world();
        let boost = pads.collect(at(0.0, -1030.0, 17.0), at(0.0, -1030.0, 17.0), 95.0);
        assert_eq!(boost, MAX_BOOST);
    }

    #[test]
    fn the_radius_is_176_for_small_and_208_for_big_from_the_origin() {
        // Small: just inside and just outside, sideways from pad 14.
        let mut pads = small_pad_world();
        assert_eq!(
            pads.collect(at(177.0, -1024.0, 17.0), at(177.0, -1024.0, 17.0), 0.0),
            0.0
        );
        assert_eq!(
            pads.collect(at(175.0, -1024.0, 17.0), at(175.0, -1024.0, 17.0), 0.0),
            12.0
        );
        // Big pad 18 at (3584, 0).
        let mut pads = BoostPads::standard();
        assert_eq!(
            pads.collect(
                at(3584.0 - 209.0, 0.0, 17.0),
                at(3584.0 - 209.0, 0.0, 17.0),
                0.0
            ),
            0.0
        );
        assert_eq!(
            pads.collect(
                at(3584.0 - 207.0, 0.0, 17.0),
                at(3584.0 - 207.0, 0.0, 17.0),
                0.0
            ),
            100.0
        );
    }

    #[test]
    fn a_car_too_high_does_not_take_the_pad() {
        let mut pads = small_pad_world();
        assert_eq!(
            pads.collect(at(0.0, -1024.0, 200.0), at(0.0, -1024.0, 200.0), 0.0),
            0.0
        );
        assert_eq!(
            pads.collect(at(0.0, -1024.0, 150.0), at(0.0, -1024.0, 150.0), 0.0),
            12.0
        );
    }

    #[test]
    fn the_test_reads_three_fifths_of_the_way_through_the_tick() {
        // Starts out of reach (200 uu), ends at 150: 0.6 along is 170, in reach; from
        // 200 to 190 it is 194, out of reach.
        let mut pads = small_pad_world();
        assert_eq!(
            pads.collect(at(200.0, -1024.0, 17.0), at(190.0, -1024.0, 17.0), 0.0),
            0.0
        );
        assert_eq!(
            pads.collect(at(200.0, -1024.0, 17.0), at(150.0, -1024.0, 17.0), 0.0),
            12.0
        );
    }

    #[test]
    fn a_small_pad_is_back_after_480_ticks_and_a_big_one_after_1200() {
        let mut pads = BoostPads::standard();
        pads.collect(at(0.0, -1024.0, 17.0), at(0.0, -1024.0, 17.0), 0.0);
        pads.collect(at(3584.0, 0.0, 17.0), at(3584.0, 0.0, 17.0), 0.0);
        for tick in 1..=1200 {
            pads.tick(DT);
            assert_eq!(pads.is_active(14), tick >= 480, "small pad at tick {tick}");
            assert_eq!(pads.is_active(18), tick >= 1200, "big pad at tick {tick}");
        }
    }

    #[test]
    fn the_first_pad_in_order_wins_when_a_car_reaches_two() {
        let pads = vec![
            BoostPad {
                position: at(0.0, 0.0, 0.0),
                big: false,
            },
            BoostPad {
                position: at(50.0, 0.0, 0.0),
                big: false,
            },
        ];
        let mut pads = BoostPads::new(pads);
        assert_eq!(
            pads.collect(at(25.0, 0.0, 17.0), at(25.0, 0.0, 17.0), 0.0),
            12.0
        );
        assert!(!pads.is_active(0) && pads.is_active(1));
    }

    #[test]
    fn unknown_pads_are_inactive_and_have_no_cooldown() {
        let pads = BoostPads::standard();
        assert!(!pads.is_active(99));
        assert_eq!(pads.cooldown_left(99), 0.0);
        assert!(pads.pad(99).is_none());
    }
}
