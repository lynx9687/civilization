use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::assets::assets_dir;
use crate::tiles::TileResources;

/// Terrain type carried by every map tile. Replicated so the client can pick a
/// tile material and run terrain-aware movement previews. The names returned by
/// [`Terrain::name`] are the canonical keys used everywhere terrain is referenced
/// by string — notably the `terrain_cost` maps in `assets/units/*.ron`.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Terrain {
    #[default]
    Grassland,
    Hill,
    Forest,
    Mountain,
    Water,
}

impl Terrain {
    /// Every terrain in declaration order — for generators and material setup.
    /// The discriminants are `0..ALL.len()`, so `terrain as usize` indexes this.
    pub const ALL: [Terrain; 5] = [
        Terrain::Grassland,
        Terrain::Hill,
        Terrain::Forest,
        Terrain::Mountain,
        Terrain::Water,
    ];

    /// Canonical lowercase name. Must match the `terrain_cost` keys in the unit
    /// RONs so per-unit movement costs resolve by terrain name.
    pub fn name(&self) -> &'static str {
        match self {
            Terrain::Grassland => "grassland",
            Terrain::Hill => "hill",
            Terrain::Forest => "forest",
            Terrain::Mountain => "mountain",
            Terrain::Water => "water",
        }
    }

    pub fn from_name(name: &str) -> Option<Terrain> {
        match name {
            "grassland" => Some(Terrain::Grassland),
            "hill" => Some(Terrain::Hill),
            "forest" => Some(Terrain::Forest),
            "mountain" => Some(Terrain::Mountain),
            "water" => Some(Terrain::Water),
            _ => None,
        }
    }

    /// Whether a land unit can ever stand on this terrain. Mountains and water
    /// are barriers used by the generator to keep a single connected landmass;
    /// per-unit movement cost (and finer impassability) lives in the unit RONs.
    pub fn is_passable(&self) -> bool {
        !matches!(self, Terrain::Mountain | Terrain::Water)
    }
}

/// Canonical per-terrain resource yields, loaded from `assets/terrain.ron`.
/// Editing the RON retunes the economy without recompiling, mirroring the unit
/// registry. A claimed tile contributes its terrain's yields to the owning city.
#[derive(Resource, Debug, Clone, Default)]
pub struct TerrainTable {
    yields: HashMap<Terrain, TileResources>,
}

impl TerrainTable {
    /// Yields for a terrain; zero if the table is missing an entry (it shouldn't).
    pub fn yields(&self, terrain: Terrain) -> TileResources {
        self.yields.get(&terrain).copied().unwrap_or(TileResources {
            food: 0,
            production: 0,
            gold: 0,
        })
    }

    pub fn len(&self) -> usize {
        self.yields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.yields.is_empty()
    }

    /// Parses a `name -> TileResources` RON map and resolves the names to
    /// [`Terrain`] variants.
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, TerrainLoadError> {
        let contents = std::fs::read_to_string(path).map_err(|e| TerrainLoadError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let by_name: HashMap<String, TileResources> =
            ron::from_str(&contents).map_err(|e| TerrainLoadError::Parse {
                path: path.to_path_buf(),
                source: e,
            })?;
        let mut yields = HashMap::new();
        for (name, res) in by_name {
            let terrain =
                Terrain::from_name(&name).ok_or(TerrainLoadError::UnknownTerrain(name))?;
            yields.insert(terrain, res);
        }
        Ok(TerrainTable { yields })
    }
}

#[derive(Debug)]
pub enum TerrainLoadError {
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        source: ron::error::SpannedError,
    },
    UnknownTerrain(String),
}

impl std::fmt::Display for TerrainLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerrainLoadError::Io { path, source } => {
                write!(f, "io error reading {}: {source}", path.display())
            }
            TerrainLoadError::Parse { path, source } => {
                write!(f, "parse error in {}: {source}", path.display())
            }
            TerrainLoadError::UnknownTerrain(name) => write!(f, "unknown terrain name: {name}"),
        }
    }
}

impl std::error::Error for TerrainLoadError {}

/// Startup system that loads `TerrainTable` from the runtime assets directory.
/// Registered by `SharedPlugin` so both server and client get it for free.
pub fn load_terrain_table(mut commands: Commands) {
    let path = assets_dir().join("terrain.ron");
    match TerrainTable::load_from_file(&path) {
        Ok(table) => {
            println!(
                "Loaded {} terrain definitions from {}",
                table.len(),
                path.display()
            );
            commands.insert_resource(table);
        }
        Err(e) => panic!("Failed to load terrain table from {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_roundtrips_for_every_variant() {
        for t in Terrain::ALL {
            assert_eq!(Terrain::from_name(t.name()), Some(t));
        }
        assert_eq!(Terrain::from_name("lava"), None);
    }

    #[test]
    fn all_indexes_match_discriminants() {
        for t in Terrain::ALL {
            assert_eq!(Terrain::ALL[t as usize], t);
        }
    }

    #[test]
    fn mountain_and_water_are_impassable() {
        assert!(!Terrain::Mountain.is_passable());
        assert!(!Terrain::Water.is_passable());
        assert!(Terrain::Grassland.is_passable());
        assert!(Terrain::Hill.is_passable());
        assert!(Terrain::Forest.is_passable());
    }

    #[test]
    fn shipped_terrain_table_parses_and_differentiates_yields() {
        let table = TerrainTable::load_from_file(std::path::Path::new("../assets/terrain.ron"))
            .expect("terrain.ron should load");
        // Every terrain must have an entry.
        assert_eq!(table.len(), Terrain::ALL.len());
        // Terrain must actually drive yields — distinct terrains should not all
        // collapse to the same numbers (this is the folded-in #12 yields check).
        let grassland = table.yields(Terrain::Grassland);
        assert_ne!(table.yields(Terrain::Hill), grassland);
        assert_ne!(table.yields(Terrain::Forest), grassland);
    }
}
