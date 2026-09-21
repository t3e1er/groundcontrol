//! Flat arena-allocated 3D Barnes-Hut Octree for high-performance n-body layout.
//!
//! Evaluates repulsive $O(N \log N)$ forces across $1\text{M}+$ particles in parallel
//! using Rayon, avoiding dynamic heap allocation on the hot simulation loop.

use rayon::prelude::*;

/// Single node in the flat arena octree.
#[derive(Debug, Clone)]
pub struct OctreeCell {
    /// Geometric center of the cell cube [x, y, z].
    pub center: [f32; 3],
    /// Half-width of the cube dimension.
    pub half_size: f32,
    /// Total mass (number of particles or weighted mass) within this cell.
    pub mass: f32,
    /// Center of mass [x, y, z] weighted by enclosed particles.
    pub center_of_mass: [f32; 3],
    /// 8 child cell indices into the arena `Vec<OctreeCell>` (0 = unoccupied).
    pub children: [u32; 8],
    /// Particle index if this is a leaf node, or `u32::MAX` if internal.
    pub particle_idx: u32,
}

impl OctreeCell {
    fn new_empty(center: [f32; 3], half_size: f32) -> Self {
        Self {
            center,
            half_size,
            mass: 0.0,
            center_of_mass: [0.0, 0.0, 0.0],
            children: [0; 8],
            particle_idx: u32::MAX,
        }
    }
}

/// Contiguous arena-allocated octree.
pub struct Octree {
    cells: Vec<OctreeCell>,
    theta_sq: f32,
}

impl Octree {
    /// Build a new Barnes-Hut octree from particle positions and masses.
    pub fn build(positions: &[[f32; 3]], masses: &[f32], theta: f32) -> Self {
        let n = positions.len();
        if n == 0 {
            return Self { cells: Vec::new(), theta_sq: theta * theta };
        }

        // 1. Calculate bounding cube enclosing all particles
        let mut min = positions[0];
        let mut max = positions[0];

        for &p in positions.iter().skip(1) {
            for k in 0..3 {
                if p[k] < min[k] {
                    min[k] = p[k];
                }
                if p[k] > max[k] {
                    max[k] = p[k];
                }
            }
        }

        let mut center = [0.0; 3];
        let mut max_span = 0.0f32;
        for k in 0..3 {
            center[k] = 0.5 * (min[k] + max[k]);
            let span = max[k] - min[k];
            if span > max_span {
                max_span = span;
            }
        }
        let half_size = (max_span * 0.5 + 1.0).max(10.0);

        // Pre-allocate arena (estimated upper bound ~2 * N cells)
        let mut tree = Self { cells: Vec::with_capacity(n * 2 + 16), theta_sq: theta * theta };

        // Root cell at index 0
        tree.cells.push(OctreeCell::new_empty(center, half_size));

        // 2. Insert particles into arena
        for i in 0..n {
            let m = if masses.is_empty() { 1.0 } else { masses[i] };
            tree.insert(0, i as u32, positions[i], m);
        }

        // 3. Compute masses and centers of mass bottom-up
        tree.update_masses(0);

        tree
    }

    fn octant_index(center: &[f32; 3], pos: &[f32; 3]) -> usize {
        let mut idx = 0;
        if pos[0] >= center[0] {
            idx |= 1;
        }
        if pos[1] >= center[1] {
            idx |= 2;
        }
        if pos[2] >= center[2] {
            idx |= 4;
        }
        idx
    }

    fn octant_center(center: &[f32; 3], half_size: f32, octant: usize) -> [f32; 3] {
        let q = half_size * 0.5;
        [
            center[0] + if (octant & 1) != 0 { q } else { -q },
            center[1] + if (octant & 2) != 0 { q } else { -q },
            center[2] + if (octant & 4) != 0 { q } else { -q },
        ]
    }

    fn insert(&mut self, cell_idx: usize, p_idx: u32, pos: [f32; 3], mass: f32) {
        let cell = &self.cells[cell_idx];
        let center = cell.center;
        let half_size = cell.half_size;
        let existing_p = cell.particle_idx;

        if cell.mass == 0.0 && existing_p == u32::MAX {
            // Empty leaf cell: claim it directly
            let cell_mut = &mut self.cells[cell_idx];
            cell_mut.particle_idx = p_idx;
            cell_mut.center_of_mass = pos;
            cell_mut.mass = mass;
            return;
        }

        if existing_p != u32::MAX {
            // Cell already holds a particle: convert to internal node and push both particles down
            let old_p = existing_p;
            let old_pos = cell.center_of_mass;
            let old_mass = cell.mass;

            self.cells[cell_idx].particle_idx = u32::MAX;
            self.cells[cell_idx].mass = 0.0; // will be recomputed

            // Insert old particle into child
            let oct1 = Self::octant_index(&center, &old_pos);
            let child1 = self.get_or_create_child(cell_idx, oct1, center, half_size);
            self.insert(child1, old_p, old_pos, old_mass);

            // Insert new particle into child
            let oct2 = Self::octant_index(&center, &pos);
            let child2 = self.get_or_create_child(cell_idx, oct2, center, half_size);
            self.insert(child2, p_idx, pos, mass);
        } else {
            // Internal node: route to child
            let oct = Self::octant_index(&center, &pos);
            let child = self.get_or_create_child(cell_idx, oct, center, half_size);
            self.insert(child, p_idx, pos, mass);
        }
    }

    fn get_or_create_child(
        &mut self,
        cell_idx: usize,
        octant: usize,
        parent_center: [f32; 3],
        parent_half_size: f32,
    ) -> usize {
        let existing_child = self.cells[cell_idx].children[octant];
        if existing_child != 0 {
            return existing_child as usize;
        }

        let new_idx = self.cells.len();
        let child_center = Self::octant_center(&parent_center, parent_half_size, octant);
        let child_half_size = parent_half_size * 0.5;

        self.cells.push(OctreeCell::new_empty(child_center, child_half_size));
        self.cells[cell_idx].children[octant] = new_idx as u32;
        new_idx
    }

    fn update_masses(&mut self, cell_idx: usize) -> (f32, [f32; 3]) {
        if self.cells.is_empty() {
            return (0.0, [0.0, 0.0, 0.0]);
        }

        let is_leaf = self.cells[cell_idx].particle_idx != u32::MAX;
        if is_leaf {
            return (self.cells[cell_idx].mass, self.cells[cell_idx].center_of_mass);
        }

        let children = self.cells[cell_idx].children;
        let mut total_mass = 0.0f32;
        let mut com_num = [0.0f32; 3];

        for &child_idx in &children {
            if child_idx != 0 {
                let (c_mass, c_com) = self.update_masses(child_idx as usize);
                total_mass += c_mass;
                com_num[0] += c_com[0] * c_mass;
                com_num[1] += c_com[1] * c_mass;
                com_num[2] += c_com[2] * c_mass;
            }
        }

        let com = if total_mass > 0.0 {
            [com_num[0] / total_mass, com_num[1] / total_mass, com_num[2] / total_mass]
        } else {
            self.cells[cell_idx].center
        };

        self.cells[cell_idx].mass = total_mass;
        self.cells[cell_idx].center_of_mass = com;
        (total_mass, com)
    }

    /// Compute total repulsive force on a particle at `pos` from all other particles.
    pub fn compute_repulsive_force(
        &self,
        particle_idx: u32,
        pos: [f32; 3],
        repulsion_scale: f32,
        eps_sq: f32,
    ) -> [f32; 3] {
        if self.cells.is_empty() {
            return [0.0, 0.0, 0.0];
        }

        let mut force = [0.0f32; 3];
        let mut stack = Vec::with_capacity(64);
        stack.push(0usize);

        while let Some(cell_idx) = stack.pop() {
            let cell = &self.cells[cell_idx];
            if cell.mass <= 0.0 {
                continue;
            }

            if cell.particle_idx == particle_idx {
                // Ignore self
                continue;
            }

            let dx = cell.center_of_mass[0] - pos[0];
            let dy = cell.center_of_mass[1] - pos[1];
            let dz = cell.center_of_mass[2] - pos[2];
            let dist_sq = dx * dx + dy * dy + dz * dz + eps_sq;

            let size = cell.half_size * 2.0;
            let is_leaf = cell.particle_idx != u32::MAX;

            // Barnes-Hut opening angle criterion: (s / d)^2 < theta^2
            if is_leaf || (size * size < self.theta_sq * dist_sq) {
                let dist = dist_sq.sqrt();
                let f_mag = -repulsion_scale * cell.mass / (dist_sq * dist);
                force[0] += dx * f_mag;
                force[1] += dy * f_mag;
                force[2] += dz * f_mag;
            } else {
                for &child_idx in &cell.children {
                    if child_idx != 0 {
                        stack.push(child_idx as usize);
                    }
                }
            }
        }

        force
    }

    /// Calculate repulsive forces for all particles in parallel using Rayon.
    pub fn compute_all_repulsions(
        &self,
        positions: &[[f32; 3]],
        repulsion_scale: f32,
        eps_sq: f32,
    ) -> Vec<[f32; 3]> {
        positions
            .par_iter()
            .enumerate()
            .map(|(i, &pos)| self.compute_repulsive_force(i as u32, pos, repulsion_scale, eps_sq))
            .collect()
    }
}
