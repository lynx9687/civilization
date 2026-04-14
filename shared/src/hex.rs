use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HexPosition {
    pub q: i32,
    pub r: i32,
}

const AXIAL_DIRECTIONS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];

impl HexPosition {
    pub fn new(q: i32, r: i32) -> Self {
        HexPosition { q, r }
    }

    pub fn neighbors(&self) -> Vec<HexPosition> {
        AXIAL_DIRECTIONS
            .iter()
            .map(|(dq, dr)| HexPosition::new(self.q + dq, self.r + dr))
            .collect()
    }

    pub fn is_neighbor(&self, other: &HexPosition) -> bool {
        self.distance(other) == 1
    }

    pub fn distance(&self, other: &HexPosition) -> i32 {
        let dq = (self.q - other.q).abs();
        let dr = (self.r - other.r).abs();
        let ds = ((self.q - other.q) + (self.r - other.r)).abs();
        (dq + dr + ds) / 2
    }

    pub fn in_bounds(&self, radius: i32) -> bool {
        self.q.abs().max(self.r.abs()).max((self.q + self.r).abs()) <= radius
    }
}

pub fn generate_grid(radius: i32) -> Vec<HexPosition> {
    let mut hexes = Vec::new();
    for q in -radius..=radius {
        let r_min = (-radius).max(-q - radius);
        let r_max = radius.min(-q + radius);
        for r in r_min..=r_max {
            hexes.push(HexPosition::new(q, r));
        }
    }
    hexes
}

pub fn hex_to_pixel(hex: &HexPosition, size: f32) -> Vec2 {
    let x = size * 1.5 * hex.q as f32;
    let y = size * (3f32.sqrt() / 2.0 * hex.q as f32 + 3f32.sqrt() * hex.r as f32);
    Vec2::new(x, y)
}

pub fn pixel_to_hex(pixel: Vec2, size: f32) -> HexPosition {
    let fq = (2.0 / 3.0 * pixel.x) / size;
    let fr = (-1.0 / 3.0 * pixel.x + 3f32.sqrt() / 3.0 * pixel.y) / size;
    axial_round(fq, fr)
}

fn axial_round(fq: f32, fr: f32) -> HexPosition {
    let fs = -fq - fr;

    let mut q = fq.round() as i32;
    let mut r = fr.round() as i32;
    let s = fs.round() as i32;

    let dq = (fq - q as f32).abs();
    let dr = (fr - r as f32).abs();
    let ds = (fs - s as f32).abs();

    if dq > dr && dq > ds {
        q = -r - s;
    } else if dr > ds {
        r = -q - s;
    }

    HexPosition { q, r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neighbors_of_origin() {
        let origin = HexPosition::new(0, 0);
        let neighbors = origin.neighbors();
        assert_eq!(neighbors.len(), 6);
        let expected = vec![
            HexPosition::new(1, 0),
            HexPosition::new(1, -1),
            HexPosition::new(0, -1),
            HexPosition::new(-1, 0),
            HexPosition::new(-1, 1),
            HexPosition::new(0, 1),
        ];
        assert_eq!(neighbors, expected);
    }

    #[test]
    fn test_neighbors_of_nonorigin() {
        let hex = HexPosition::new(2, -1);
        let neighbors = hex.neighbors();
        assert_eq!(neighbors.len(), 6);
        let expected = vec![
            HexPosition::new(3, -1),
            HexPosition::new(3, -2),
            HexPosition::new(2, -2),
            HexPosition::new(1, -1),
            HexPosition::new(1, 0),
            HexPosition::new(2, 0),
        ];
        assert_eq!(neighbors, expected);
    }

    #[test]
    fn test_is_neighbor() {
        let origin = HexPosition::new(0, 0);
        // All 6 directions should be neighbors
        for &(dq, dr) in &AXIAL_DIRECTIONS {
            let neighbor = HexPosition::new(dq, dr);
            assert!(
                origin.is_neighbor(&neighbor),
                "{:?} should be neighbor of origin",
                neighbor
            );
        }
        // Non-neighbors
        assert!(!origin.is_neighbor(&HexPosition::new(2, 0)));
        assert!(!origin.is_neighbor(&HexPosition::new(0, 0)));
        assert!(!origin.is_neighbor(&HexPosition::new(2, -2)));
    }

    #[test]
    fn test_distance() {
        let origin = HexPosition::new(0, 0);
        assert_eq!(origin.distance(&origin), 0);
        assert_eq!(origin.distance(&HexPosition::new(1, 0)), 1);
        assert_eq!(origin.distance(&HexPosition::new(2, -1)), 2);
        assert_eq!(
            HexPosition::new(-1, 2).distance(&HexPosition::new(2, -2)),
            4
        );
    }

    #[test]
    fn test_in_bounds() {
        assert!(HexPosition::new(0, 0).in_bounds(5));
        assert!(HexPosition::new(5, 0).in_bounds(5));
        assert!(!HexPosition::new(6, 0).in_bounds(5));
        assert!(!HexPosition::new(1, 2).in_bounds(2));
    }

    #[test]
    fn test_generate_grid_radius_0() {
        let grid = generate_grid(0);
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0], HexPosition::new(0, 0));
    }

    #[test]
    fn test_generate_grid_radius_1() {
        let grid = generate_grid(1);
        assert_eq!(grid.len(), 7);
    }

    #[test]
    fn test_generate_grid_radius_5() {
        let grid = generate_grid(5);
        assert_eq!(grid.len(), 91);
    }

    #[test]
    fn test_generate_grid_all_in_bounds() {
        let radius = 3;
        let grid = generate_grid(radius);
        for hex in &grid {
            assert!(
                hex.in_bounds(radius),
                "{:?} should be in bounds of radius {}",
                hex,
                radius
            );
        }
    }

    #[test]
    fn test_hex_to_pixel_origin() {
        let origin = HexPosition::new(0, 0);
        let pixel = hex_to_pixel(&origin, 32.0);
        assert!(
            (pixel.x).abs() < 1e-5,
            "x should be near 0, got {}",
            pixel.x
        );
        assert!(
            (pixel.y).abs() < 1e-5,
            "y should be near 0, got {}",
            pixel.y
        );
    }

    #[test]
    fn test_hex_to_pixel_q1() {
        let hex = HexPosition::new(1, 0);
        let size = 32.0;
        let pixel = hex_to_pixel(&hex, size);
        let expected_x = size * 1.5;
        assert!(
            (pixel.x - expected_x).abs() < 1e-4,
            "x should be {}, got {}",
            expected_x,
            pixel.x
        );
    }

    #[test]
    fn test_pixel_to_hex_roundtrip() {
        let size = 32.0;
        let grid = generate_grid(3);
        for hex in &grid {
            let pixel = hex_to_pixel(hex, size);
            let back = pixel_to_hex(pixel, size);
            assert_eq!(
                *hex, back,
                "roundtrip failed for {:?}: pixel={:?}, got back {:?}",
                hex, pixel, back
            );
        }
    }
}
