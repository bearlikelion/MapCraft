//! Noise heightmaps from FastNoise Lite, set up as Godot's FastNoiseLite resource sets it up.

use fastnoise_lite::{CellularDistanceFunction, FastNoiseLite};
pub use fastnoise_lite::{FractalType, NoiseType};

/// Pixels across the longer side of the area noise covers, as in Godot's default 512 by 512 NoiseTexture2D.
pub const NOISE_PIXELS: f64 = 512.0;

/// A noise heightmap; frequency and offset are in pixels of a [`NOISE_PIXELS`] wide texture, as in Godot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightNoise {
    pub kind: NoiseType,
    pub seed: u32,
    pub frequency: f64,
    pub offset: [f64; 2],
    pub fractal: FractalType,
    pub octaves: u32,
    pub lacunarity: f64,
    pub gain: f64,
    pub weighted_strength: f64,
    /// Share of the area's width and depth over which each side eases down to black, up to 0.5.
    pub edge_falloff: f64,
    /// Steps the heights climb in; below 2 keeps them smooth.
    pub terraces: u32,
}

impl Default for HeightNoise {
    fn default() -> Self {
        Self {
            kind: NoiseType::OpenSimplex2S,
            seed: 0,
            frequency: 0.01,
            offset: [0.0; 2],
            fractal: FractalType::FBm,
            octaves: 5,
            lacunarity: 2.0,
            gain: 0.5,
            weighted_strength: 0.0,
            edge_falloff: 0.0,
            terraces: 0,
        }
    }
}

impl HeightNoise {
    /// Starting points to tweak.
    pub fn presets() -> [(&'static str, HeightNoise); 5] {
        let d = Self::default();
        [
            ("Hills", Self { frequency: 0.004, octaves: 3, gain: 0.4, ..d }),
            ("Mountains", Self { kind: NoiseType::OpenSimplex2, frequency: 0.005, fractal: FractalType::Ridged, weighted_strength: 0.3, ..d }),
            ("Dunes", Self { frequency: 0.008, fractal: FractalType::PingPong, octaves: 2, gain: 0.3, ..d }),
            ("Craters", Self { kind: NoiseType::Cellular, frequency: 0.012, octaves: 2, gain: 0.3, ..d }),
            ("Mesas", Self { frequency: 0.005, octaves: 3, terraces: 4, ..d }),
        ]
    }

    /// The noise in -1..1 at a pixel; settings out of Godot's ranges are clamped so the generator never overflows.
    pub(crate) fn sampler(&self) -> impl Fn(f64, f64) -> f64 {
        let d = Self::default();
        let fin = |v: f64, fallback: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { fallback };
        let frequency = fin(self.frequency, d.frequency, 1e-4, 1.0);
        let octaves = self.octaves.clamp(1, 10);
        let lacunarity = fin(self.lacunarity, d.lacunarity, 0.0, 16.0);
        let mut n = FastNoiseLite::with_seed(self.seed.min(1 << 30) as i32);
        n.set_noise_type(Some(self.kind));
        n.set_frequency(Some(frequency as f32));
        n.set_fractal_type(Some(self.fractal));
        n.set_fractal_octaves(Some(octaves as i32));
        n.set_fractal_lacunarity(Some(lacunarity as f32));
        n.set_fractal_gain(Some(fin(self.gain, d.gain, 0.0, 4.0) as f32));
        n.set_fractal_weighted_strength(Some(fin(self.weighted_strength, 0.0, 0.0, 1.0) as f32));
        n.set_cellular_distance_function(Some(CellularDistanceFunction::Euclidean));
        // Keeps the generator's integer lattice coordinates far inside i32 at every octave.
        let reach = 1e9 / (frequency * 2.0 * lacunarity.max(1.0).powi(octaves as i32 - 1));
        let [ox, oy] = self.offset.map(|o| fin(o, 0.0, -reach, reach));
        move |x, y| {
            let v = f64::from(n.get_noise_2d((x + ox).clamp(-reach, reach) as f32, (y + oy).clamp(-reach, reach) as f32));
            if v.is_finite() { v } else { 0.0 }
        }
    }
}
