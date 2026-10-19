//! Saving a map to disk and loading it back, with a cache keyed by seed and generator version.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::{generate, WorldConfig, WorldMap, GENERATOR_VERSION};

const MAGIC: &[u8; 4] = b"RCWM";

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
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect())
    }
}

impl WorldMap {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.height.len() * 8 + 64);
        out.extend(MAGIC);
        out.extend(self.version.to_le_bytes());
        out.extend(self.seed.to_le_bytes());
        out.extend(self.config.size.to_le_bytes());
        out.extend(self.config.map_cell.to_le_bytes());
        put_layer(&mut out, &self.height);
        put_layer(&mut out, &self.continentalness);
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
        put_bytes(&mut out, &self.ford);
        out.extend((self.features.len() as u64).to_le_bytes());
        for f in &self.features {
            out.push(f.kind as u8);
            out.extend(f.x.to_le_bytes());
            out.extend(f.z.to_le_bytes());
            out.extend(f.value.to_le_bytes());
        }
        let sum = checksum(&out);
        out.extend(sum.to_le_bytes());
        out
    }

    /// Refuses a map made by another generator version, which would not match the terrain
    /// players' edits were made against.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
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
        };
        config.validate().map_err(bad)?;
        let cells = config.dim() * config.dim();
        Ok(Self {
            version,
            seed,
            config,
            height: r.layer(cells)?,
            continentalness: r.layer(cells)?,
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
            ford: r.bytes(cells)?,
            features: r.features()?,
        })
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
pub fn cache_path(dir: &Path, seed: u32, config: WorldConfig) -> PathBuf {
    dir.join(format!(
        "map-v{GENERATOR_VERSION}-seed{seed}-{}m-{}cell.rcmap",
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
            },
        )
    }

    #[test]
    fn test_map_round_trips_through_bytes() {
        let map = small();
        assert_eq!(map, WorldMap::from_bytes(&map.to_bytes()).unwrap());
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
        };
        let first = load_or_generate(&dir, 5, config);
        assert!(cache_path(&dir, 5, config).exists());
        assert_eq!(first, load_or_generate(&dir, 5, config));
        std::fs::remove_dir_all(dir).ok();
    }
}
