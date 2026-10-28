//! Saving a map to disk and loading it back, with a cache keyed by seed and generator version.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::{generate, WorldConfig, WorldMap, GENERATOR_VERSION};

const MAGIC: &[u8; 4] = b"RCWM";

fn put_config(out: &mut Vec<u8>, config: WorldConfig) {
    out.extend(config.size.to_le_bytes());
    out.extend(config.map_cell.to_le_bytes());
    out.extend(config.bounds.min_y.to_le_bytes());
    out.extend(config.bounds.max_y.to_le_bytes());
    out.push(config.tectonics.plates);
    for value in [
        config.tectonics.relief,
        config.climate.warmth,
        config.climate.wind_x,
        config.climate.wind_z,
    ] {
        out.extend(value.to_le_bytes());
    }
}

fn checksum(bytes: &[u8]) -> u64 {
    // FNV-1a, to catch a truncated or damaged file
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

fn put_layer(out: &mut Vec<u8>, layer: &[f32]) {
    out.extend((layer.len() as u64).to_le_bytes());
    for v in layer {
        out.extend(v.to_le_bytes());
    }
}

fn put_bytes(out: &mut Vec<u8>, layer: &[u8]) {
    out.extend((layer.len() as u64).to_le_bytes());
    out.extend(layer);
}

fn bad(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.0.len() < n {
            return Err(bad("map file is truncated"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn bytes(&mut self, expected: usize) -> io::Result<Vec<u8>> {
        let len = u64::from_le_bytes(self.take(8)?.try_into().unwrap()) as usize;
        if len != expected {
            return Err(bad(format!("layer has {len} cells, expected {expected}")));
        }
        Ok(self.take(len)?.to_vec())
    }

    fn features(&mut self) -> io::Result<Vec<super::Feature>> {
        let count = u64::from_le_bytes(self.take(8)?.try_into().unwrap()) as usize;
        let mut features = Vec::new();
        for _ in 0..count {
            let kind = super::FeatureKind::from_u8(self.take(1)?[0])
                .ok_or_else(|| bad("unknown feature"))?;
            let x = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
            let z = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
            let value = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
            features.push(super::Feature { kind, x, z, value });
        }
        Ok(features)
    }

    fn layer(&mut self, expected: usize) -> io::Result<Vec<f32>> {
        let len = u64::from_le_bytes(self.take(8)?.try_into().unwrap()) as usize;
        if len != expected {
            return Err(bad(format!("layer has {len} cells, expected {expected}")));
        }
        Ok(self
            .take(len * 4)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect())
    }
}

impl WorldMap {
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.height.len() * 8 + 64);
        out.extend(MAGIC);
        out.extend(self.version.to_le_bytes());
        out.extend(self.seed.to_le_bytes());
        put_config(&mut out, self.config);
        put_layer(&mut out, &self.height);
        put_layer(&mut out, &self.continentalness);
        put_bytes(&mut out, &self.plate);
        put_bytes(&mut out, &self.boundary);
        put_layer(&mut out, &self.uplift);
        put_layer(&mut out, &self.precipitation);
        put_layer(&mut out, &self.flow);
        put_bytes(&mut out, &self.down);
        put_bytes(&mut out, &self.water_kind);
        put_layer(&mut out, &self.water_surface);
        put_layer(&mut out, &self.river_width);
        put_layer(&mut out, &self.temperature);
        put_layer(&mut out, &self.moisture);
        put_bytes(&mut out, &self.biome);
        put_bytes(&mut out, &self.trees);
        put_bytes(&mut out, &self.roughness);
        put_bytes(&mut out, &self.range);
        put_bytes(&mut out, &self.ford);
        out.extend((self.features.len() as u64).to_le_bytes());
        for f in &self.features {
            out.push(f.kind as u8);
            out.extend(f.x.to_le_bytes());
            out.extend(f.z.to_le_bytes());
            out.extend(f.value.to_le_bytes());
        }
        // Geology follows the original map layers.
        put_bytes(&mut out, &self.geology);
        put_layer(&mut out, &self.strata_offset);
        put_bytes(&mut out, &self.geological_age);
        let sum = checksum(&out);
        out.extend(sum.to_le_bytes());
        out
    }

    /// Refuses a map made by another generator version, which would not match the terrain
    /// players' edits were made against.
    pub(crate) fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 8 {
            return Err(bad("map file is truncated"));
        }
        let (body, sum) = bytes.split_at(bytes.len() - 8);
        if checksum(body).to_le_bytes() != sum {
            return Err(bad("map file is damaged: checksum does not match"));
        }
        let mut r = Reader(body);
        if r.take(4)? != MAGIC {
            return Err(bad("not a world map file"));
        }
        let version = r.u32()?;
        if version != GENERATOR_VERSION {
            return Err(bad(format!(
                "map was made by generator version {version}, this build is version {GENERATOR_VERSION}"
            )));
        }
        let seed = r.u32()?;
        let config = WorldConfig {
            size: r.i64()?,
            map_cell: r.i64()?,
            bounds: ochre_base::WorldBounds {
                min_y: r.i64()?,
                max_y: r.i64()?,
            },
            tectonics: super::tectonics::TectonicConfig {
                plates: r.take(1)?[0],
                relief: r.f32()?,
            },
            climate: super::climate::ClimateConfig {
                warmth: r.f32()?,
                wind_x: r.f32()?,
                wind_z: r.f32()?,
            },
        };
        config.validate().map_err(bad)?;
        let cells = config.dim() * config.dim();
        let mut map = Self {
            version,
            seed,
            config,
            height: r.layer(cells)?,
            continentalness: r.layer(cells)?,
            plate: r.bytes(cells)?,
            boundary: r.bytes(cells)?,
            uplift: r.layer(cells)?,
            geological_age: vec![0; cells],
            geology: vec![0; cells],
            strata_offset: vec![0.0; cells],
            precipitation: r.layer(cells)?,
            flow: r.layer(cells)?,
            down: r.bytes(cells)?,
            water_kind: r.bytes(cells)?,
            water_surface: r.layer(cells)?,
            river_width: r.layer(cells)?,
            temperature: r.layer(cells)?,
            moisture: r.layer(cells)?,
            biome: r.bytes(cells)?,
            trees: r.bytes(cells)?,
            roughness: r.bytes(cells)?,
            range: r.bytes(cells)?,
            ford: r.bytes(cells)?,
            features: r.features()?,
        };
        map.geology = r.bytes(cells)?;
        map.strata_offset = r.layer(cells)?;
        map.geological_age = r.bytes(cells)?;
        if !r.0.is_empty() {
            return Err(bad("unexpected data after geology layers"));
        }
        Ok(map)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // written whole under another name, so a crash never leaves half a map to be loaded
        let tmp = path.with_extension("tmp");
        std::fs::File::create(&tmp)?.write_all(&self.to_bytes())?;
        std::fs::rename(tmp, path)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }
}

/// Where the map for `seed` is cached under `dir`.
pub(crate) fn cache_path(dir: &Path, seed: u32, config: WorldConfig) -> PathBuf {
    let mut identity = Vec::new();
    put_config(&mut identity, config);
    let identity = checksum(&identity);
    dir.join(format!(
        "map-v{GENERATOR_VERSION}-seed{seed}-{}m-{}cell-{identity:016x}.rcmap",
        config.size, config.map_cell
    ))
}

/// The cached map for `seed`, or a freshly generated one, cached for next time. A cache file that
/// won't load (another version, damaged) is made again.
pub fn load_or_generate(dir: &Path, seed: u32, config: WorldConfig) -> WorldMap {
    let path = cache_path(dir, seed, config);
    match WorldMap::load(&path) {
        Ok(map) if map.seed == seed && map.config == config => return map,
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("regenerating the map: {e}"),
    }
    let map = generate(seed, config);
    if let Err(e) = map.save(&path) {
        tracing::warn!("could not cache the map at {}: {e}", path.display());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> WorldMap {
        generate(
            3,
            WorldConfig {
                size: 2048,
                map_cell: 32,
                ..Default::default()
            },
        )
    }

    #[test]
    fn test_map_round_trips_through_bytes() {
        let mut map = small();
        map.geological_age[0] = 73;
        map.geological_age[1] = 255;
        assert_eq!(map, WorldMap::from_bytes(&map.to_bytes()).unwrap());
    }

    #[test]
    fn old_map_with_different_terrain_version_is_rejected() {
        let map = small();
        let mut old = map.to_bytes();
        // Reproduce the pre-geology version-40 layout, with a valid checksum.
        old.truncate(old.len() - 8 - 16 - map.height.len() * 6);
        old[4..8].copy_from_slice(&40u32.to_le_bytes());
        old.extend(checksum(&old).to_le_bytes());
        assert!(WorldMap::from_bytes(&old)
            .unwrap_err()
            .to_string()
            .contains("generator version"));
    }

    #[test]
    fn test_damaged_and_foreign_maps_are_refused() {
        let map = small();
        let mut bytes = map.to_bytes();
        bytes[40] ^= 1;
        assert!(WorldMap::from_bytes(&bytes).is_err());
        assert!(WorldMap::from_bytes(&bytes[..20]).is_err());

        // another generator version, with a checksum that fits
        let mut other = map.to_bytes();
        other[4..8].copy_from_slice(&(GENERATOR_VERSION + 1).to_le_bytes());
        let n = other.len() - 8;
        let sum = checksum(&other[..n]).to_le_bytes();
        other[n..].copy_from_slice(&sum);
        let error = WorldMap::from_bytes(&other).unwrap_err();
        assert!(error.to_string().contains("generator version"), "{error}");
    }

    #[test]
    fn test_cache_is_reused() {
        let dir = std::env::temp_dir().join(format!("rcmap-test-{}", std::process::id()));
        let config = WorldConfig {
            size: 2048,
            map_cell: 32,
            ..Default::default()
        };
        let first = load_or_generate(&dir, 5, config);
        assert!(cache_path(&dir, 5, config).exists());
        assert_eq!(first, load_or_generate(&dir, 5, config));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cache_identity_includes_vertical_tectonic_and_climate_settings() {
        let base = WorldConfig::default();
        let original = cache_path(Path::new("worlds"), 42, base);
        let mut variants = [base; 4];
        variants[0].bounds.min_y = -2048;
        variants[1].tectonics.plates = 16;
        variants[2].tectonics.relief = 1.5;
        variants[3].climate.wind_x = -1.0;
        for config in variants {
            assert_ne!(original, cache_path(Path::new("worlds"), 42, config));
        }
    }
}
