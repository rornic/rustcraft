use bevy::math::{I64Vec2, Vec2};

use super::noise::NoiseGenerator;

pub struct ErosionParams {
    pub droplet_count: u32,
    pub max_lifetime: u32,
    pub inertia: f32,
    pub sediment_capacity_factor: f32,
    pub min_sediment_capacity: f32,
    pub erode_speed: f32,
    pub deposit_speed: f32,
    pub evaporate_speed: f32,
    pub gravity: f32,
    pub initial_water: f32,
    pub initial_speed: f32,
    pub erosion_radius: i32,
}

impl Default for ErosionParams {
    fn default() -> Self {
        Self {
            droplet_count: 30_000,
            max_lifetime: 30,
            inertia: 0.05,
            sediment_capacity_factor: 4.0,
            min_sediment_capacity: 0.01,
            erode_speed: 0.3,
            deposit_speed: 0.3,
            evaporate_speed: 0.01,
            gravity: 4.0,
            initial_water: 1.0,
            initial_speed: 1.0,
            erosion_radius: 3,
        }
    }
}

// Plain row-major height buffer used only during simulation, in real block-height
// units (not raw 0..10 noise units) so gravity/erode_speed have sensible magnitudes.
pub struct HeightGrid {
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl HeightGrid {
    pub fn from_noise(
        noise: &NoiseGenerator,
        origin: I64Vec2,
        width: u32,
        height: u32,
        world_height: u64,
    ) -> Self {
        let mut data = Vec::with_capacity((width * height) as usize);
        for z in 0..height {
            for x in 0..width {
                let pos = I64Vec2::new(origin.x + x as i64, origin.y + z as i64);
                let noise_val = noise.get(pos);
                data.push((noise_val * world_height as f64) as f32);
            }
        }
        Self { width, height, data }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn data(&self) -> &[f32] {
        &self.data
    }

    fn index(&self, x: i32, z: i32) -> usize {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let z = z.clamp(0, self.height as i32 - 1) as u32;
        (z * self.width + x) as usize
    }

    // Clamps to the nearest edge cell rather than panicking - a droplet's erosion
    // brush can probe slightly outside bounds near the grid's border.
    pub fn get(&self, x: i32, z: i32) -> f32 {
        self.data[self.index(x, z)]
    }

    fn set(&mut self, x: i32, z: i32, value: f32) {
        let idx = self.index(x, z);
        self.data[idx] = value;
    }

    fn contains(&self, pos: Vec2) -> bool {
        pos.x >= 0.0
            && pos.x < (self.width - 1) as f32
            && pos.y >= 0.0
            && pos.y < (self.height - 1) as f32
    }

    // Bilinear height + analytic gradient of that interpolation, matching Lague's
    // reference erosion implementation.
    fn bilinear_height_and_gradient(&self, pos: Vec2) -> (f32, Vec2) {
        let x0 = pos.x.floor() as i32;
        let z0 = pos.y.floor() as i32;
        let fx = pos.x - x0 as f32;
        let fz = pos.y - z0 as f32;

        let h00 = self.get(x0, z0);
        let h10 = self.get(x0 + 1, z0);
        let h01 = self.get(x0, z0 + 1);
        let h11 = self.get(x0 + 1, z0 + 1);

        let height = h00 * (1.0 - fx) * (1.0 - fz)
            + h10 * fx * (1.0 - fz)
            + h01 * (1.0 - fx) * fz
            + h11 * fx * fz;

        let gradient_x = (h10 - h00) * (1.0 - fz) + (h11 - h01) * fz;
        let gradient_z = (h01 - h00) * (1.0 - fx) + (h11 - h10) * fx;

        (height, Vec2::new(gradient_x, gradient_z))
    }
}

// Side length (in world blocks) of the deterministic lattice cell each spawn
// candidate is derived from - fixed rather than computed from `count`/grid size, so
// a given world-space cell always proposes the same candidate point regardless of
// which window later samples it (load-bearing once a future milestone tiles erosion
// across overlapping region windows; not exercised by anything in this milestone).
const SPAWN_CELL_SIZE: i64 = 2;

// FNV-1a over (seed, a, b) — fixed algorithm with guaranteed stability, unlike
// DefaultHasher (algorithm unspecified) or StdRng (algorithm may change with rand).
fn stable_hash(seed: u32, a: i64, b: i64) -> u64 {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let mut h = FNV_OFFSET;
    for byte in seed
        .to_le_bytes()
        .iter()
        .chain(a.to_le_bytes().iter())
        .chain(b.to_le_bytes().iter())
    {
        h ^= *byte as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

// One jittered candidate per world-space lattice cell, in world coordinates.
fn jitter_in_cell(seed: u32, cell: I64Vec2) -> Vec2 {
    let h = stable_hash(seed, cell.x, cell.y);
    // Split into two independent fractions in [0, SPAWN_CELL_SIZE)
    let jx = (h as u32) as f32 / 4294967296.0 * SPAWN_CELL_SIZE as f32;
    let jz = (h >> 32) as f32 / 4294967296.0 * SPAWN_CELL_SIZE as f32;
    Vec2::new(jx, jz)
}

// Deterministic droplet spawn positions (grid-local coordinates) covering
// `[0,width) x [0,height)` of a window whose world-space origin is `world_origin`.
// `count` caps how many of the lattice's candidates are used; cell density (not
// `count`) drives spatial distribution, which is what keeps a given cell's candidate
// independent of `count`/window size - see `SPAWN_CELL_SIZE`.
pub fn spawn_points(seed: u32, world_origin: I64Vec2, width: u32, height: u32, count: u32) -> Vec<Vec2> {
    let cell_min_x = world_origin.x.div_euclid(SPAWN_CELL_SIZE);
    let cell_min_z = world_origin.y.div_euclid(SPAWN_CELL_SIZE);
    let cell_max_x = (world_origin.x + width as i64).div_euclid(SPAWN_CELL_SIZE);
    let cell_max_z = (world_origin.y + height as i64).div_euclid(SPAWN_CELL_SIZE);

    let mut points = Vec::new();
    for cell_z in cell_min_z..=cell_max_z {
        for cell_x in cell_min_x..=cell_max_x {
            if points.len() >= count as usize {
                return points;
            }
            let cell = I64Vec2::new(cell_x, cell_z);
            let jitter = jitter_in_cell(seed, cell);
            let world_point = Vec2::new(
                (cell_x * SPAWN_CELL_SIZE) as f32 + jitter.x,
                (cell_z * SPAWN_CELL_SIZE) as f32 + jitter.y,
            );
            let local = world_point - Vec2::new(world_origin.x as f32, world_origin.y as f32);
            if local.x >= 0.0 && local.x < width as f32 && local.y >= 0.0 && local.y < height as f32 {
                points.push(local);
            }
        }
    }
    points
}

// Distributes `amount` over a weighted disk of cells around `pos` (closer cells get
// more weight), rather than a single cell, to avoid spiky single-cell artifacts.
fn apply_brush(grid: &mut HeightGrid, pos: Vec2, radius: i32, amount: f32, sign: f32) {
    if amount <= 0.0 {
        return;
    }
    let cx = pos.x.floor() as i32;
    let cz = pos.y.floor() as i32;
    let r = radius.max(0);
    let r2 = (r * r) as f32;

    let mut weights = Vec::new();
    let mut total_weight = 0.0f32;
    for dz in -r..=r {
        for dx in -r..=r {
            let dist_sq = (dx * dx + dz * dz) as f32;
            if dist_sq > r2 {
                continue;
            }
            let weight = 1.0 - dist_sq.sqrt() / r.max(1) as f32;
            if weight <= 0.0 {
                continue;
            }
            weights.push((dx, dz, weight));
            total_weight += weight;
        }
    }
    if total_weight <= 0.0 {
        return;
    }
    for (dx, dz, weight) in weights {
        let delta = sign * amount * (weight / total_weight);
        let x = cx + dx;
        let z = cz + dz;
        grid.set(x, z, grid.get(x, z) + delta);
    }
}

fn deposit_at(grid: &mut HeightGrid, pos: Vec2, amount: f32, radius: i32) {
    apply_brush(grid, pos, radius, amount, 1.0);
}

fn erode_at(grid: &mut HeightGrid, pos: Vec2, amount: f32, radius: i32) {
    apply_brush(grid, pos, radius, amount, -1.0);
}

// Simulates one droplet's full lifetime, mutating `grid` in place. Any sediment
// still carried when the droplet stops (off the edge, water exhausted, or stuck in a
// pit) is deposited at its final position - without this, droplets that exit early
// would silently destroy mass, and the dump tool's eroded/deposited balance check
// (see examples/erosion_dump.rs) would never close.
fn simulate_droplet(grid: &mut HeightGrid, spawn: Vec2, params: &ErosionParams) {
    let mut pos = spawn;
    let mut dir = Vec2::ZERO;
    let mut speed = params.initial_speed;
    let mut water = params.initial_water;
    let mut sediment = 0.0f32;

    for _ in 0..params.max_lifetime {
        let (height, gradient) = grid.bilinear_height_and_gradient(pos);

        dir = dir * params.inertia - gradient * (1.0 - params.inertia);
        if dir.length_squared() < f32::EPSILON {
            break;
        }
        dir = dir.normalize();

        let new_pos = pos + dir;
        if !grid.contains(new_pos) {
            break;
        }

        let (new_height, _) = grid.bilinear_height_and_gradient(new_pos);
        let delta_height = new_height - height;

        let capacity = (-delta_height * speed * water * params.sediment_capacity_factor)
            .max(params.min_sediment_capacity);

        if sediment > capacity || delta_height > 0.0 {
            let deposit = if delta_height > 0.0 {
                delta_height.min(sediment)
            } else {
                (sediment - capacity) * params.deposit_speed
            };
            sediment -= deposit;
            deposit_at(grid, pos, deposit, params.erosion_radius);
        } else {
            let erode = ((capacity - sediment) * params.erode_speed).min(-delta_height);
            erode_at(grid, pos, erode, params.erosion_radius);
            sediment += erode;
        }

        // delta_height is negative when descending, so subtracting it gains speed
        // going downhill (kinetic energy gained from the drop in elevation).
        speed = (speed * speed - delta_height * params.gravity).max(0.0).sqrt();
        water *= 1.0 - params.evaporate_speed;
        pos = new_pos;

        if water < f32::EPSILON {
            break;
        }
    }

    if sediment > 0.0 {
        deposit_at(grid, pos, sediment, params.erosion_radius);
    }
}

pub fn erode(grid: &mut HeightGrid, seed: u32, world_origin: I64Vec2, params: &ErosionParams) {
    let spawns = spawn_points(seed, world_origin, grid.width, grid.height, params.droplet_count);
    for spawn in spawns {
        simulate_droplet(grid, spawn, params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_grid(width: u32, height: u32, value: f32) -> HeightGrid {
        HeightGrid {
            width,
            height,
            data: vec![value; (width * height) as usize],
        }
    }

    // One-directional ramp: height decreases as x increases, constant along z.
    fn ramp_grid(width: u32, height: u32) -> HeightGrid {
        let mut data = Vec::with_capacity((width * height) as usize);
        for _ in 0..height {
            for x in 0..width {
                data.push((width - x) as f32);
            }
        }
        HeightGrid { width, height, data }
    }

    #[test]
    fn test_bilinear_flat_grid_has_zero_gradient() {
        let grid = flat_grid(10, 10, 5.0);
        let (height, gradient) = grid.bilinear_height_and_gradient(Vec2::new(3.5, 4.5));
        assert_eq!(5.0, height);
        assert_eq!(Vec2::ZERO, gradient);
    }

    #[test]
    fn test_droplet_flows_downhill_on_ramp() {
        let grid = ramp_grid(40, 40);
        let params = ErosionParams {
            // Disable erosion/deposition itself so this test isolates direction of
            // travel, not how the heightmap changes.
            erode_speed: 0.0,
            deposit_speed: 0.0,
            ..ErosionParams::default()
        };

        let spawn = Vec2::new(5.0, 20.0);
        let mut pos = spawn;
        let mut dir = Vec2::ZERO;
        let mut last_height = grid.bilinear_height_and_gradient(pos).0;
        for _ in 0..params.max_lifetime {
            let (height, gradient) = grid.bilinear_height_and_gradient(pos);
            dir = dir * params.inertia - gradient * (1.0 - params.inertia);
            if dir.length_squared() < f32::EPSILON {
                break;
            }
            dir = dir.normalize();
            let new_pos = pos + dir;
            if !grid.contains(new_pos) {
                break;
            }
            assert!(height <= last_height + f32::EPSILON, "droplet moved uphill");
            last_height = height;
            pos = new_pos;
        }
        assert!(pos.x > spawn.x, "droplet should have moved toward lower x (downhill)");
    }

    #[test]
    fn test_spawn_points_deterministic() {
        let a = spawn_points(42, I64Vec2::new(100, -50), 64, 64, 200);
        let b = spawn_points(42, I64Vec2::new(100, -50), 64, 64, 200);
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn test_erosion_conserves_mass() {
        let noise = NoiseGenerator::new(7);
        let mut grid = HeightGrid::from_noise(&noise, I64Vec2::new(0, 0), 96, 96, 256);
        let before: f32 = grid.data().iter().sum();

        erode(&mut grid, 7, I64Vec2::new(0, 0), &ErosionParams::default());

        let after: f32 = grid.data().iter().sum();
        let relative_diff = (after - before).abs() / before.abs().max(1.0);
        assert!(relative_diff < 0.01, "erosion should conserve total height mass, diff was {relative_diff}");
    }
}
