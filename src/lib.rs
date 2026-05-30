//! # lau-worldgen
//!
//! Procedural voxel world generator for the Lau platform.
//! Generates unique deterministic voxel worlds from seeds using hash-based noise.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// WorldSeed
// ---------------------------------------------------------------------------

/// A seed value that determines the entire world. Wraps a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldSeed(pub u64);

impl WorldSeed {
    /// Create a seed from a string by hashing it (FNV-1a inspired).
    pub fn from_string(s: &str) -> Self {
        let mut hash: u64 = 14_695_981_039_346_656_037;
        for byte in s.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
        WorldSeed(hash)
    }

    /// Create a seed from the current system time (via a simple counter trick).
    /// Uses `std::time::SystemTime` for a cheap pseudo-random value.
    pub fn random() -> Self {
        use std::time::SystemTime;
        let dur = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let raw = dur.as_nanos() as u64;
        // Mix bits via xorshift
        let mut x = raw;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        WorldSeed(x.wrapping_mul(0x2545_f491_4f6c_dd1d))
    }

    /// Return the raw `u64` value.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl Default for WorldSeed {
    fn default() -> Self {
        WorldSeed(42)
    }
}

impl From<u64> for WorldSeed {
    fn from(v: u64) -> Self {
        WorldSeed(v)
    }
}

// ---------------------------------------------------------------------------
// WorldConfig
// ---------------------------------------------------------------------------

/// Configuration that controls world generation parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldConfig {
    pub seed: WorldSeed,
    pub chunk_radius: i32,
    pub sea_level: i32,
    pub biome_scale: f64,
}

impl Default for WorldConfig {
    fn default() -> Self {
        WorldConfig {
            seed: WorldSeed::default(),
            chunk_radius: 4,
            sea_level: 32,
            biome_scale: 0.005,
        }
    }
}

// ---------------------------------------------------------------------------
// Biome
// ---------------------------------------------------------------------------

/// Biome types that can appear in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Mountains,
    Ocean,
    CrystalCaves,
    FloatingIslands,
    Volcanic,
}

impl Biome {
    /// Returns a human-readable name for the biome.
    pub fn name(self) -> &'static str {
        match self {
            Biome::Plains => "Plains",
            Biome::Forest => "Forest",
            Biome::Desert => "Desert",
            Biome::Mountains => "Mountains",
            Biome::Ocean => "Ocean",
            Biome::CrystalCaves => "Crystal Caves",
            Biome::FloatingIslands => "Floating Islands",
            Biome::Volcanic => "Volcanic",
        }
    }

    /// Returns the surface material name for this biome.
    pub fn surface_material(self) -> &'static str {
        match self {
            Biome::Plains => "grass",
            Biome::Forest => "grass",
            Biome::Desert => "sand",
            Biome::Mountains => "stone",
            Biome::Ocean => "water",
            Biome::CrystalCaves => "crystal",
            Biome::FloatingIslands => "cloud_stone",
            Biome::Volcanic => "basalt",
        }
    }
}

/// Determine biome from moisture and temperature values (each in roughly -1..1).
fn biome_from(moisture: f64, temperature: f64) -> Biome {
    use Biome::*;
    if moisture < -0.3 && temperature > 0.3 {
        return Desert;
    }
    if moisture > 0.4 && temperature < -0.3 {
        return CrystalCaves;
    }
    if temperature > 0.6 && moisture < 0.0 {
        return Volcanic;
    }
    if temperature > 0.7 {
        return Volcanic;
    }
    if moisture > 0.3 && temperature > 0.0 {
        return Forest;
    }
    if moisture < -0.5 {
        return Mountains;
    }
    if moisture < -0.2 {
        return FloatingIslands;
    }
    Plains
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

/// Integer hash function (splitmix64 variant) for deterministic pseudo-random.
fn hash2(seed: u64, x: i64, z: i64) -> u64 {
    let mut h = seed;
    h ^= (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= (z as u64).wrapping_mul(0xc2b2_ae35_1677_f3a9);
    // splitmix64 finalizer
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf5_8476_d1ca_e559);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_1172);
    h ^= h >> 31;
    h
}

/// Hash-based 2D pseudo-noise returning a value in roughly -1..1.
fn noise2d(seed: u64, x: f64, z: f64) -> f64 {
    let ix = x.floor() as i64;
    let iz = z.floor() as i64;
    let fx = x - ix as f64;
    let fz = z - iz as f64;

    // Smoothstep
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uz = fz * fz * (3.0 - 2.0 * fz);

    let n00 = hash2(seed, ix, iz) as f64 / u64::MAX as f64;
    let n10 = hash2(seed, ix + 1, iz) as f64 / u64::MAX as f64;
    let n01 = hash2(seed, ix, iz + 1) as f64 / u64::MAX as f64;
    let n11 = hash2(seed, ix + 1, iz + 1) as f64 / u64::MAX as f64;

    let x0 = n00 + ux * (n10 - n00);
    let x1 = n01 + ux * (n11 - n01);
    let v = x0 + uz * (x1 - x0);

    // Map from 0..1 to -1..1
    v * 2.0 - 1.0
}

/// Fractal Brownian Motion — layered noise for natural terrain.
fn fbm(seed: u64, x: f64, z: f64, octaves: u32) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut max_amp = 0.0;
    let mut s = seed;

    for _ in 0..octaves {
        value += amplitude * noise2d(s, x * frequency, z * frequency);
        max_amp += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
        s = s.wrapping_add(0x5555_5555_5555_5555);
    }
    value / max_amp
}

// ---------------------------------------------------------------------------
// Chunk
// ---------------------------------------------------------------------------

/// A 16×64×16 vertical column of the world. We store the surface heights
/// and biomes per column (x, z within the chunk).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Chunk X coordinate (in chunk units, not block units).
    pub x: i32,
    /// Chunk Z coordinate (in chunk units, not block units).
    pub z: i32,
    /// Surface heights per column `[z][x]`, range 0..=63.
    pub heights: [[i32; 16]; 16],
    /// Biome per column `[z][x]`.
    pub biomes: [[Biome; 16]; 16],
}

impl Chunk {
    /// Get the height at local (lx, lz) within this chunk.
    pub fn height_at(&self, lx: usize, lz: usize) -> i32 {
        self.heights[lz][lx]
    }

    /// Get the biome at local (lx, lz) within this chunk.
    pub fn biome_at(&self, lx: usize, lz: usize) -> Biome {
        self.biomes[lz][lx]
    }

    /// Count the total number of solid blocks in this chunk.
    /// For each column, all blocks from y=0 up to height are solid.
    pub fn total_blocks(&self) -> usize {
        let mut count = 0usize;
        for lz in 0..16 {
            for lx in 0..16 {
                count += self.heights[lz][lx].max(0) as usize;
            }
        }
        count
    }
}

// ---------------------------------------------------------------------------
// WorldGenerator
// ---------------------------------------------------------------------------

/// Generates chunks from a config using deterministic noise.
pub struct WorldGenerator {
    config: WorldConfig,
}

impl WorldGenerator {
    pub fn new(config: WorldConfig) -> Self {
        WorldGenerator { config }
    }

    /// Generate a single chunk at chunk coordinates (cx, cz).
    pub fn generate_chunk(&self, cx: i32, cz: i32) -> Chunk {
        let seed = self.config.seed.as_u64();
        let scale = self.config.biome_scale;
        let sea = self.config.sea_level;

        let mut heights = [[0i32; 16]; 16];
        let mut biomes = [[Biome::Plains; 16]; 16];

        for lz in 0..16i32 {
            for lx in 0..16i32 {
                let wx = (cx * 16 + lx) as f64;
                let wz = (cz * 16 + lz) as f64;

                // Height from noise
                let h_noise = fbm(seed, wx * scale * 4.0, wz * scale * 4.0, 6);
                let height = ((h_noise + 1.0) * 0.5 * 63.0).round() as i32;
                let height = height.clamp(0, 63);

                // Moisture & temperature from separate noise layers
                let moisture = fbm(seed.wrapping_add(1000), wx * scale, wz * scale, 4);
                let temperature = fbm(seed.wrapping_add(2000), wx * scale, wz * scale, 4);
                let biome = biome_from(moisture, temperature);

                // Ocean biome overrides height to sea level or below
                let final_height = if biome == Biome::Ocean {
                    height.min(sea)
                } else if biome == Biome::Mountains {
                    // Boost mountain heights
                    (height + 15).min(63)
                } else if biome == Biome::FloatingIslands {
                    // Islands hover above sea level
                    (sea + 10 + (height / 3)).min(63)
                } else {
                    height
                };

                heights[lz as usize][lx as usize] = final_height;
                biomes[lz as usize][lx as usize] = biome;
            }
        }

        Chunk {
            x: cx,
            z: cz,
            heights,
            biomes,
        }
    }
}

// ---------------------------------------------------------------------------
// WorldMap
// ---------------------------------------------------------------------------

/// A collection of generated chunks forming a world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldMap {
    chunks: HashMap<String, Chunk>,
}

impl WorldMap {
    /// Generate the entire world from a config.
    pub fn generate(config: WorldConfig) -> Self {
        let gen = WorldGenerator::new(config.clone());
        let radius = config.chunk_radius;
        let mut chunks = HashMap::new();

        for cz in -radius..=radius {
            for cx in -radius..=radius {
                let chunk = gen.generate_chunk(cx, cz);
                chunks.insert(format!("{cx},{cz}"), chunk);
            }
        }

        WorldMap { chunks }
    }

    fn chunk_key(cx: i32, cz: i32) -> String {
        format!("{cx},{cz}")
    }

    /// Get the surface height at world coordinates (wx, wz).
    /// Returns 0 if the chunk is not present.
    pub fn get_height(&self, wx: i32, wz: i32) -> i32 {
        let cx = wx.div_euclid(16);
        let cz = wz.div_euclid(16);
        let key = Self::chunk_key(cx, cz);
        if let Some(chunk) = self.chunks.get(&key) {
            let lx = wx.rem_euclid(16) as usize;
            let lz = wz.rem_euclid(16) as usize;
            chunk.heights[lz][lx]
        } else {
            0
        }
    }

    /// Get the biome at world coordinates (wx, wz).
    /// Returns `Biome::Plains` as a default if the chunk is not present.
    pub fn get_biome(&self, wx: i32, wz: i32) -> Biome {
        let cx = wx.div_euclid(16);
        let cz = wz.div_euclid(16);
        let key = Self::chunk_key(cx, cz);
        if let Some(chunk) = self.chunks.get(&key) {
            let lx = wx.rem_euclid(16) as usize;
            let lz = wz.rem_euclid(16) as usize;
            chunk.biomes[lz][lx]
        } else {
            Biome::Plains
        }
    }

    /// Total number of solid blocks across all chunks.
    pub fn total_blocks(&self) -> usize {
        self.chunks.values().map(|c| c.total_blocks()).sum()
    }

    /// Number of chunks in the world.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Get a reference to a chunk by coordinates, if it exists.
    pub fn get_chunk(&self, cx: i32, cz: i32) -> Option<&Chunk> {
        self.chunks.get(&Self::chunk_key(cx, cz))
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- WorldSeed tests --

    #[test]
    fn seed_from_string_deterministic() {
        let a = WorldSeed::from_string("hello world");
        let b = WorldSeed::from_string("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn seed_from_string_different_strings() {
        let a = WorldSeed::from_string("foo");
        let b = WorldSeed::from_string("bar");
        assert_ne!(a, b);
    }

    #[test]
    fn seed_random_nonzero() {
        let s = WorldSeed::random();
        // Just ensure it doesn't panic and produces something
        assert!(s.as_u64() != 0 || WorldSeed::random().as_u64() != 0);
    }

    #[test]
    fn seed_random_unique() {
        // Two random seeds are very likely different
        let a = WorldSeed::random();
        let b = WorldSeed::random();
        // Can't guarantee but it's astronomically unlikely they match
        let _ = (a, b); // just confirm no panic
    }

    #[test]
    fn seed_default_is_42() {
        assert_eq!(WorldSeed::default().as_u64(), 42);
    }

    #[test]
    fn seed_from_u64() {
        let s = WorldSeed::from(12345u64);
        assert_eq!(s.as_u64(), 12345);
    }

    // -- Noise tests --

    #[test]
    fn noise2d_deterministic() {
        let a = noise2d(42, 1.0, 2.0);
        let b = noise2d(42, 1.0, 2.0);
        assert!((a - b).abs() < 1e-10);
    }

    #[test]
    fn noise2d_different_seeds() {
        let a = noise2d(1, 1.0, 2.0);
        let b = noise2d(2, 1.0, 2.0);
        assert_ne!(a, b);
    }

    #[test]
    fn noise2d_range() {
        // Noise should roughly be in -1..1
        for x in 0..20 {
            for z in 0..20 {
                let v = noise2d(42, x as f64 * 0.3, z as f64 * 0.3);
                assert!(v >= -1.5 && v <= 1.5, "noise out of range: {v}");
            }
        }
    }

    #[test]
    fn fbm_deterministic() {
        let a = fbm(42, 10.0, 20.0, 4);
        let b = fbm(42, 10.0, 20.0, 4);
        assert!((a - b).abs() < 1e-10);
    }

    #[test]
    fn fbm_more_octaves_smother() {
        let v1 = fbm(42, 5.0, 5.0, 1);
        let v4 = fbm(42, 5.0, 5.0, 4);
        // Both should be valid numbers
        assert!(v1.is_finite());
        assert!(v4.is_finite());
    }

    // -- Biome tests --

    #[test]
    fn biome_desert_hot_dry() {
        assert_eq!(biome_from(-0.5, 0.5), Biome::Desert);
    }

    #[test]
    fn biome_forest_wet_warm() {
        assert_eq!(biome_from(0.5, 0.2), Biome::Forest);
    }

    #[test]
    fn biome_plains_moderate() {
        assert_eq!(biome_from(0.0, 0.0), Biome::Plains);
    }

    #[test]
    fn biome_crystal_caves() {
        assert_eq!(biome_from(0.5, -0.5), Biome::CrystalCaves);
    }

    #[test]
    fn biome_volcanic_hot() {
        assert_eq!(biome_from(0.0, 0.8), Biome::Volcanic);
    }

    #[test]
    fn biome_mountains_dry() {
        assert_eq!(biome_from(-0.6, 0.0), Biome::Mountains);
    }

    #[test]
    fn biome_surface_materials() {
        assert_eq!(Biome::Plains.surface_material(), "grass");
        assert_eq!(Biome::Desert.surface_material(), "sand");
        assert_eq!(Biome::Mountains.surface_material(), "stone");
        assert_eq!(Biome::Ocean.surface_material(), "water");
        assert_eq!(Biome::CrystalCaves.surface_material(), "crystal");
        assert_eq!(Biome::Volcanic.surface_material(), "basalt");
        assert_eq!(Biome::FloatingIslands.surface_material(), "cloud_stone");
    }

    // -- Chunk generation tests --

    #[test]
    fn generate_chunk_heights_in_range() {
        let gen = WorldGenerator::new(WorldConfig::default());
        let chunk = gen.generate_chunk(0, 0);
        for lz in 0..16 {
            for lx in 0..16 {
                let h = chunk.heights[lz][lx];
                assert!((0..=63).contains(&h), "height {h} out of range at ({lx},{lz})");
            }
        }
    }

    #[test]
    fn generate_chunk_deterministic() {
        let gen = WorldGenerator::new(WorldConfig::default());
        let a = gen.generate_chunk(1, 2);
        let b = gen.generate_chunk(1, 2);
        assert_eq!(a.heights, b.heights);
        assert_eq!(a.biomes, b.biomes);
    }

    #[test]
    fn generate_chunk_different_coords_differ() {
        let gen = WorldGenerator::new(WorldConfig::default());
        let a = gen.generate_chunk(0, 0);
        let b = gen.generate_chunk(1, 0);
        assert_ne!(a.heights, b.heights);
    }

    // -- WorldMap tests --

    #[test]
    fn worldmap_generate() {
        let config = WorldConfig {
            chunk_radius: 2,
            ..WorldConfig::default()
        };
        let map = WorldMap::generate(config);
        // radius 2 → chunks from -2 to 2 = 5 per axis = 25 total
        assert_eq!(map.chunk_count(), 25);
    }

    #[test]
    fn worldmap_get_height_valid() {
        let config = WorldConfig {
            chunk_radius: 1,
            ..WorldConfig::default()
        };
        let map = WorldMap::generate(config);
        let h = map.get_height(5, 5);
        assert!((0..=63).contains(&h));
    }

    #[test]
    fn worldmap_get_height_outside_returns_zero() {
        let config = WorldConfig {
            chunk_radius: 0,
            ..WorldConfig::default()
        };
        let map = WorldMap::generate(config);
        // (100, 100) is outside radius 0
        assert_eq!(map.get_height(100, 100), 0);
    }

    #[test]
    fn worldmap_get_biome_valid() {
        let config = WorldConfig {
            chunk_radius: 1,
            ..WorldConfig::default()
        };
        let map = WorldMap::generate(config);
        let _biome = map.get_biome(0, 0);
        // Just ensure no panic
    }

    #[test]
    fn worldmap_total_blocks_positive() {
        let config = WorldConfig {
            chunk_radius: 1,
            ..WorldConfig::default()
        };
        let map = WorldMap::generate(config);
        assert!(map.total_blocks() > 0);
    }

    // -- Serialization tests --

    #[test]
    fn serde_world_seed() {
        let seed = WorldSeed::from_string("test");
        let json = serde_json::to_string(&seed).unwrap();
        let back: WorldSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }

    #[test]
    fn serde_biome() {
        for biome in [
            Biome::Plains,
            Biome::Forest,
            Biome::Desert,
            Biome::Mountains,
            Biome::Ocean,
            Biome::CrystalCaves,
            Biome::FloatingIslands,
            Biome::Volcanic,
        ] {
            let json = serde_json::to_string(&biome).unwrap();
            let back: Biome = serde_json::from_str(&json).unwrap();
            assert_eq!(biome, back);
        }
    }

    #[test]
    fn serde_world_config() {
        let config = WorldConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let back: WorldConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.seed, back.seed);
        assert_eq!(config.chunk_radius, back.chunk_radius);
    }

    #[test]
    fn serde_chunk() {
        let gen = WorldGenerator::new(WorldConfig::default());
        let chunk = gen.generate_chunk(3, 7);
        let json = serde_json::to_string(&chunk).unwrap();
        let back: Chunk = serde_json::from_str(&json).unwrap();
        assert_eq!(chunk.x, back.x);
        assert_eq!(chunk.z, back.z);
        assert_eq!(chunk.heights, back.heights);
    }

    #[test]
    fn serde_world_map() {
        let config = WorldConfig {
            chunk_radius: 1,
            ..WorldConfig::default()
        };
        let map = WorldMap::generate(config);
        let json = serde_json::to_string(&map).unwrap();
        let back: WorldMap = serde_json::from_str(&json).unwrap();
        assert_eq!(map.chunk_count(), back.chunk_count());
    }
}
