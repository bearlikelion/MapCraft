//! Bhop platform layouts, ported from SurfsUp's `bhop_generator.gd` into map units with Z up.

use glam::DVec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Linear,
    Grid,
    Spiral,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bhop {
    pub pattern: Pattern,
    pub count: usize,
    /// Platform width, depth and height.
    pub size: DVec3,
    /// Each platform's size is scaled by up to this fraction, up or down.
    pub variation: f64,
    pub seed: u64,
    pub spacing: f64,
    pub rise: f64,
    /// Linear direction in degrees from +X, counter-clockwise seen from above.
    pub heading: f64,
    pub zigzag: f64,
    pub columns: usize,
    pub rows: usize,
    pub grid_spacing: f64,
    pub row_rise: f64,
    pub stagger: bool,
    pub radius: f64,
    pub radius_per_turn: f64,
    pub angle_step: f64,
    pub rise_per_turn: f64,
    pub clockwise: bool,
    /// Each platform moves sideways by up to this much.
    pub jitter: f64,
}

impl Default for Bhop {
    fn default() -> Self {
        Self {
            pattern: Pattern::Linear,
            count: 30,
            size: DVec3::new(192.0, 192.0, 16.0),
            variation: 0.0,
            seed: 1,
            spacing: 320.0,
            rise: 64.0,
            heading: 0.0,
            zigzag: 0.0,
            columns: 5,
            rows: 6,
            grid_spacing: 384.0,
            row_rise: 128.0,
            stagger: false,
            radius: 192.0,
            radius_per_turn: 64.0,
            angle_step: 45.0,
            rise_per_turn: 384.0,
            clockwise: true,
            jitter: 0.0,
        }
    }
}

/// xorshift64*, so the same seed always gives the same layout.
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Center and size of every platform.
pub fn platforms(b: &Bhop) -> Vec<(DVec3, DVec3)> {
    let count = b.count.min(4096);
    let centers: Vec<DVec3> = match b.pattern {
        Pattern::Linear => {
            let h = b.heading.to_radians();
            let dir = DVec3::new(h.cos(), h.sin(), 0.0);
            let side = DVec3::new(-dir.y, dir.x, 0.0);
            (0..count)
                .map(|i| {
                    let i = i as f64;
                    let wobble = if b.zigzag != 0.0 { (i * std::f64::consts::PI / 3.0).sin() * b.zigzag } else { 0.0 };
                    dir * (i * b.spacing) + DVec3::Z * (i * b.rise) + side * wobble
                })
                .collect()
        }
        Pattern::Grid => {
            let (cols, rows) = (b.columns.clamp(1, 256), b.rows.clamp(1, 256));
            (0..rows)
                .flat_map(|row| (0..cols).map(move |col| (row, col)))
                .take(count)
                .map(|(row, col)| {
                    let mut x = (col as f64 - cols as f64 / 2.0) * b.grid_spacing;
                    if b.stagger && row % 2 == 1 {
                        x += b.grid_spacing * 0.5;
                    }
                    DVec3::new(x, (row as f64 - rows as f64 / 2.0) * b.grid_spacing, row as f64 * b.row_rise)
                })
                .collect()
        }
        Pattern::Spiral => {
            let step = b.angle_step.to_radians() * if b.clockwise { -1.0 } else { 1.0 };
            (0..count)
                .map(|i| {
                    let angle = step * i as f64;
                    let turns = angle.abs() / std::f64::consts::TAU;
                    let r = b.radius + turns * b.radius_per_turn;
                    DVec3::new(angle.cos() * r, angle.sin() * r, turns * b.rise_per_turn)
                })
                .collect()
        }
    };
    let mut rng = Rng(b.seed.max(1));
    centers
        .into_iter()
        .map(|c| {
            let scale = if b.variation > 0.0 { 1.0 + (rng.unit() * 2.0 - 1.0) * b.variation } else { 1.0 };
            let shift = if b.jitter > 0.0 { DVec3::new(rng.unit() * 2.0 - 1.0, rng.unit() * 2.0 - 1.0, 0.0) * b.jitter } else { DVec3::ZERO };
            (c + shift, (b.size * scale).max(DVec3::ONE))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        let linear = platforms(&Bhop::default());
        assert_eq!(linear.len(), 30);
        assert_eq!(linear[1].0, DVec3::new(320.0, 0.0, 64.0));
        let grid = platforms(&Bhop { pattern: Pattern::Grid, count: 7, ..Bhop::default() });
        assert_eq!(grid.len(), 7);
        assert_eq!(grid[5].0.z, 128.0);
        let spiral = platforms(&Bhop { pattern: Pattern::Spiral, count: 9, ..Bhop::default() });
        assert!((spiral[8].0.z - 384.0).abs() < 1e-9, "eight 45 degree steps make one turn");
        let a = platforms(&Bhop { variation: 0.3, ..Bhop::default() });
        let b = platforms(&Bhop { variation: 0.3, ..Bhop::default() });
        assert_eq!(a, b);
        assert!(a.iter().any(|(_, s)| *s != DVec3::new(192.0, 192.0, 16.0)));
    }
}
