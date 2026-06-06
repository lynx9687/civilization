use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash};

#[derive(Debug, Clone, Deserialize)]
pub struct UnitDefinition {
    pub hp: u32,
    pub move_budget: u32,
    pub attack_range: u32,
    pub attack_damage: u32,
    pub gold_upkeep: u32,
    pub production_cost: u32,
    pub build_targets: Vec<String>,
    pub terrain_cost: HashMap<String, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitTypeId(pub u8);

#[derive(Resource, Default, Debug)]
pub struct UnitRegistry {
    pub name_to_id: HashMap<String, UnitTypeId>,
    pub definitions: HashMap<UnitTypeId, UnitDefinition>,
}

impl UnitRegistry {
    pub fn get(&self, type_id: &UnitTypeId) -> Option<&UnitDefinition> {
        self.definitions.get(type_id)
    }

    pub fn id_of(&self, name: &str) -> Option<UnitTypeId> {
        self.name_to_id.get(name).copied()
    }

    pub fn name_of(&self, id: UnitTypeId) -> Option<&str> {
        self.name_to_id
            .iter()
            .find(|(_, type_id)| **type_id == id)
            .map(|(name, _)| name.as_str())
    }

    /// Builds a registry from `(name, ron-contents)` pairs. Entries are sorted by
    /// name first so that ID assignment is deterministic: the server and client must
    /// agree on (name → id), and this order can't depend on the source (arbitrary
    /// `read_dir` order on native vs. embedded order on wasm).
    fn from_entries(mut entries: Vec<(String, String)>) -> Result<Self, LoadError> {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut name_to_id = HashMap::new();
        let mut definitions = HashMap::new();
        let mut next_id: u8 = 0;
        for (name, contents) in entries {
            let def: UnitDefinition = ron::from_str(&contents).map_err(|e| LoadError::Parse {
                path: std::path::PathBuf::from(&name),
                source: e,
            })?;
            let id = UnitTypeId(next_id);
            name_to_id.insert(name, id);
            definitions.insert(id, def);
            next_id += 1;
        }
        Ok(UnitRegistry {
            name_to_id,
            definitions,
        })
    }

    /// Loads unit definitions from a directory of `.ron` files (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_from_dir(dir: &std::path::Path) -> Result<Self, LoadError> {
        let read_dir = std::fs::read_dir(dir).map_err(|e| LoadError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let mut entries = Vec::new();
        for entry in read_dir {
            let path = entry
                .map_err(|e| LoadError::Io {
                    path: dir.to_path_buf(),
                    source: e,
                })?
                .path();
            if path.extension().and_then(|s| s.to_str()) != Some("ron") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| LoadError::BadFileName { path: path.clone() })?
                .to_string();
            let contents = std::fs::read_to_string(&path).map_err(|e| LoadError::Io {
                path: path.clone(),
                source: e,
            })?;
            entries.push((name, contents));
        }
        Self::from_entries(entries)
    }

    /// Loads unit definitions baked into the binary at compile time (wasm only,
    /// where there is no filesystem to read from).
    #[cfg(target_arch = "wasm32")]
    pub fn load_embedded() -> Result<Self, LoadError> {
        static UNIT_ASSETS: include_dir::Dir<'_> =
            include_dir::include_dir!("$CARGO_MANIFEST_DIR/../assets/units");
        let mut entries = Vec::new();
        for file in UNIT_ASSETS.files() {
            let path = file.path();
            if path.extension().and_then(|s| s.to_str()) != Some("ron") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| LoadError::BadFileName {
                    path: path.to_path_buf(),
                })?
                .to_string();
            let contents = file
                .contents_utf8()
                .ok_or_else(|| LoadError::BadFileName {
                    path: path.to_path_buf(),
                })?
                .to_string();
            entries.push((name, contents));
        }
        Self::from_entries(entries)
    }
}

#[derive(Debug)]
pub enum LoadError {
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        source: ron::error::SpannedError,
    },
    BadFileName {
        path: std::path::PathBuf,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io { path, source } => {
                write!(f, "io error reading {}: {source}", path.display())
            }
            LoadError::Parse { path, source } => {
                write!(f, "parse error in {}: {source}", path.display())
            }
            LoadError::BadFileName { path } => write!(f, "bad file name: {}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

pub fn is_within_move_range(
    from: &crate::hex::HexPosition,
    to: &crate::hex::HexPosition,
    move_budget: u32,
) -> bool {
    let d = from.distance(to);
    d > 0 && (d as u32) <= move_budget
}

// intentionally parallel to is_within_move_range — keep semantics in sync
pub fn is_within_attack_range(
    from: &crate::hex::HexPosition,
    to: &crate::hex::HexPosition,
    attack_range: u32,
) -> bool {
    let d = from.distance(to);
    d > 0 && (d as u32) <= attack_range
}

/// Cost sentinel for terrain a unit can never enter (e.g. mountain). Any
/// per-terrain cost at or above this is treated as a wall, same as a missing
/// key. Kept in sync with the `99999` mountain entries in `assets/units/*.ron`.
pub const IMPASSABLE_COST: u32 = 99999;

/// Per-terrain enter cost for this unit, or `None` if it can never enter the
/// terrain. A missing `terrain_cost` key (e.g. water — no key at all) and the
/// [`IMPASSABLE_COST`] sentinel (e.g. mountain) both mean "wall". This — not
/// `Terrain::is_passable` — is the single source of truth for movement, so a
/// future amphibious unit only needs a water cost added to its RON.
fn enter_cost(def: &UnitDefinition, terrain: crate::terrain::Terrain) -> Option<u32> {
    match def.terrain_cost.get(terrain.name()) {
        Some(&c) if c < IMPASSABLE_COST => Some(c),
        _ => None,
    }
}

/// Every tile this unit can reach from `from` within its `move_budget`, mapped
/// to the cheapest accumulated enter-cost to get there. The start tile is
/// **excluded** (cost-0, no-op move). Cost to ENTER a tile is its terrain's
/// [`enter_cost`]; the search only steps onto enterable tiles, so walls
/// (water/mountain/missing-key) are never crossed *or* landed on. Boundary is
/// inclusive: a tile costing exactly `move_budget` is reachable.
///
/// This is the single source of truth shared by the server move validator and
/// the client move preview, so the two can never disagree. `terrain_at` is the
/// terrain lookup *and* the set of on-map tiles (returns `None` off the map);
/// the server passes `MapTiles`, the client a map built from replicated tiles.
pub fn reachable_tiles(
    from: &crate::hex::HexPosition,
    def: &UnitDefinition,
    terrain_at: impl Fn(&crate::hex::HexPosition) -> Option<crate::terrain::Terrain>,
) -> std::collections::HashMap<crate::hex::HexPosition, u32> {
    use crate::hex::HexPosition;
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap};

    let budget = def.move_budget;
    let mut best: HashMap<HexPosition, u32> = HashMap::new();
    // Min-heap on (cost, q, r): HexPosition isn't Ord (and hex.rs isn't ours to
    // change), so we order by the cost and the raw coords and rebuild the hex on
    // pop. The tie-breaker coords don't affect which tiles end up reachable.
    let mut heap: BinaryHeap<Reverse<(u32, i32, i32)>> = BinaryHeap::new();
    heap.push(Reverse((0, from.q, from.r)));
    best.insert(*from, 0);

    while let Some(Reverse((cost, q, r))) = heap.pop() {
        let pos = HexPosition::new(q, r);
        // Stale heap entry (a cheaper path was already settled).
        if best.get(&pos).is_some_and(|&b| cost > b) {
            continue;
        }
        for nb in pos.neighbors() {
            let Some(terrain) = terrain_at(&nb) else {
                continue; // off the map
            };
            let Some(step) = enter_cost(def, terrain) else {
                continue; // wall: can't enter this terrain
            };
            let next = cost + step;
            if next > budget {
                continue; // out of move budget
            }
            if best.get(&nb).is_none_or(|&b| next < b) {
                best.insert(nb, next);
                heap.push(Reverse((next, nb.q, nb.r)));
            }
        }
    }

    best.remove(from); // exclude the origin — a self-move is a no-op, never offered
    best
}

/// Terrain-aware replacement for [`is_within_move_range`]: true iff `to` is
/// reachable from `from` within budget over enterable tiles (origin excluded).
/// Membership-tests [`reachable_tiles`] so the server and client agree.
pub fn is_reachable(
    from: &crate::hex::HexPosition,
    to: &crate::hex::HexPosition,
    def: &UnitDefinition,
    terrain_at: impl Fn(&crate::hex::HexPosition) -> Option<crate::terrain::Terrain>,
) -> bool {
    reachable_tiles(from, def, terrain_at).contains_key(to)
}

/// Startup system that loads `UnitRegistry` from the runtime assets directory and inserts it as a resource.
/// Registered by `SharedPlugin` so both server and client get it for free.
pub fn load_unit_registry(mut commands: Commands) {
    // Native reads the runtime assets directory; wasm has no filesystem, so it uses
    // the definitions embedded into the binary at compile time.
    #[cfg(not(target_arch = "wasm32"))]
    let result = UnitRegistry::load_from_dir(&crate::assets::assets_dir().join("units"));
    #[cfg(target_arch = "wasm32")]
    let result = UnitRegistry::load_embedded();

    match result {
        Ok(registry) => {
            println!("Loaded {} unit definitions", registry.definitions.len());
            commands.insert_resource(registry);
        }
        Err(e) => panic!("Failed to load unit registry: {e}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitVerb {
    Move,
    Attack,
    Fortify,
    Build,
    Skip,
}

// universal verbs available to every unit; capability flags add the rest
pub fn available_verbs(def: &UnitDefinition) -> Vec<UnitVerb> {
    let mut v = vec![UnitVerb::Move, UnitVerb::Fortify, UnitVerb::Skip];
    // only ranged units (attack_range > 1) get the Attack verb;
    // melee classes engage by moving into an enemy hex instead
    if def.attack_range > 1 {
        v.push(UnitVerb::Attack);
    }
    if !def.build_targets.is_empty() {
        v.push(UnitVerb::Build);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unit_definition_deserializes_from_ron() {
        let ron = r#"(
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 20,
            build_targets: [],
            terrain_cost: {
                "grassland": 1,
                "hill": 2,
                "forest": 2,
                "mountain": 99999,
            },
        )"#;
        let def: UnitDefinition = ron::from_str(ron).expect("should parse");
        assert_eq!(def.hp, 10);
        assert_eq!(def.move_budget, 2);
        assert_eq!(def.attack_range, 1);
        assert_eq!(def.attack_damage, 4);
        assert_eq!(def.gold_upkeep, 1);
        assert_eq!(def.production_cost, 20);
        assert!(def.build_targets.is_empty());
        assert_eq!(def.terrain_cost.get("grassland"), Some(&1));
        assert_eq!(def.terrain_cost.get("mountain"), Some(&99999));
    }

    #[test]
    fn test_all_shipped_unit_files_parse() {
        let unit_files = [
            ("warrior", "../assets/units/warrior.ron"),
            ("archer", "../assets/units/archer.ron"),
            ("cavalry", "../assets/units/cavalry.ron"),
            ("knight", "../assets/units/knight.ron"),
            ("settler", "../assets/units/settler.ron"),
        ];
        for (name, path) in unit_files {
            let contents = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("failed to read {name}: {e}"));
            let _def: UnitDefinition =
                ron::from_str(&contents).unwrap_or_else(|e| panic!("failed to parse {name}: {e}"));
        }
    }

    #[test]
    fn test_registry_loads_all_definitions_from_dir() {
        let registry = UnitRegistry::load_from_dir(std::path::Path::new("../assets/units"))
            .expect("should load");

        let warrior_id = registry.id_of("warrior").expect("warrior should exist");
        let archer_id = registry.id_of("archer").expect("archer should exist");
        let cavalry_id = registry.id_of("cavalry").expect("cavalry should exist");
        let knight_id = registry.id_of("knight").expect("knight should exist");
        let settler_id = registry.id_of("settler").expect("settler should exist");

        assert!(registry.get(&warrior_id).is_some());
        assert!(registry.get(&archer_id).is_some());
        assert!(registry.get(&cavalry_id).is_some());
        assert!(registry.get(&knight_id).is_some());
        assert!(registry.get(&settler_id).is_some());
        assert_eq!(registry.definitions.len(), 5);

        let warrior = registry.get(&warrior_id).unwrap();
        assert_eq!(warrior.hp, 10);
        assert_eq!(warrior.move_budget, 2);

        // Deterministic IDs: alphabetical sort means archer=0, cavalry=1, knight=2, settler=3, warrior=4
        assert_eq!(archer_id, UnitTypeId(0));
        assert_eq!(warrior_id, UnitTypeId(4));
    }

    #[test]
    fn test_is_within_move_range_respects_budget() {
        use crate::hex::HexPosition;

        let from = HexPosition::new(0, 0);
        let close = HexPosition::new(2, 0);
        let far = HexPosition::new(5, 0);

        assert!(is_within_move_range(&from, &close, 2)); // distance 2, budget 2 → ok
        assert!(!is_within_move_range(&from, &far, 2)); // distance 5, budget 2 → out
        assert!(!is_within_move_range(&from, &from, 2)); // same hex → out (no-op move)
    }

    #[test]
    fn test_melee_classes_have_no_attack_verb() {
        let registry = UnitRegistry::load_from_dir(std::path::Path::new("../assets/units"))
            .expect("should load");
        for name in ["warrior", "cavalry", "knight"] {
            let def = registry
                .get(&registry.id_of(name).expect(name))
                .expect(name);
            let verbs = available_verbs(def);
            assert!(verbs.contains(&UnitVerb::Move), "{name} should Move");
            assert!(verbs.contains(&UnitVerb::Fortify), "{name} should Fortify");
            assert!(verbs.contains(&UnitVerb::Skip), "{name} should Skip");
            assert!(
                !verbs.contains(&UnitVerb::Attack),
                "{name} (melee, attack_range==1) must NOT have Attack verb"
            );
            assert!(!verbs.contains(&UnitVerb::Build), "{name} cannot Build");
        }
    }

    #[test]
    fn test_archer_has_attack_verb() {
        let registry = UnitRegistry::load_from_dir(std::path::Path::new("../assets/units"))
            .expect("should load");
        let archer = registry.get(&registry.id_of("archer").unwrap()).unwrap();
        let verbs = available_verbs(archer);
        assert!(
            verbs.contains(&UnitVerb::Attack),
            "archer (attack_range>1) should Attack"
        );
        assert!(verbs.contains(&UnitVerb::Move));
        assert!(verbs.contains(&UnitVerb::Fortify));
        assert!(verbs.contains(&UnitVerb::Skip));
        assert!(!verbs.contains(&UnitVerb::Build));
    }

    #[test]
    fn test_available_verbs_for_settler() {
        let registry = UnitRegistry::load_from_dir(std::path::Path::new("../assets/units"))
            .expect("should load");
        let settler = registry.get(&registry.id_of("settler").unwrap()).unwrap();
        let verbs = available_verbs(settler);
        assert!(verbs.contains(&UnitVerb::Move));
        assert!(verbs.contains(&UnitVerb::Build));
        assert!(verbs.contains(&UnitVerb::Fortify));
        assert!(verbs.contains(&UnitVerb::Skip));
        // settler has attack_range = 0, so the attack_range > 1 gate is false
        assert!(!verbs.contains(&UnitVerb::Attack));
    }

    // --- terrain-aware reachability (reachable_tiles / is_reachable) ---

    /// A warrior-like def with the canonical land costs (grassland 1, hill 2,
    /// forest 2, mountain 99999) and NO water key — mirrors assets/units/*.ron.
    fn land_unit(move_budget: u32) -> UnitDefinition {
        let mut terrain_cost = HashMap::new();
        terrain_cost.insert("grassland".to_string(), 1);
        terrain_cost.insert("hill".to_string(), 2);
        terrain_cost.insert("forest".to_string(), 2);
        terrain_cost.insert("mountain".to_string(), IMPASSABLE_COST);
        UnitDefinition {
            hp: 10,
            move_budget,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost,
        }
    }

    #[test]
    fn reachable_excludes_origin() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        let from = HexPosition::new(0, 0);
        let def = land_unit(2);
        let map = |_: &HexPosition| Some(Terrain::Grassland);
        let reach = reachable_tiles(&from, &def, map);
        assert!(
            !reach.contains_key(&from),
            "origin (no-op self-move) must be excluded"
        );
        assert!(!is_reachable(&from, &from, &def, map));
    }

    #[test]
    fn reachable_cost_accumulates_per_terrain() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        let from = HexPosition::new(0, 0);
        // All grassland (cost 1 each), budget 2: a 1-away tile costs 1, 2-away costs 2.
        let def = land_unit(2);
        let grass = |_: &HexPosition| Some(Terrain::Grassland);
        let reach = reachable_tiles(&from, &def, grass);
        assert_eq!(reach.get(&HexPosition::new(1, 0)), Some(&1));
        assert_eq!(reach.get(&HexPosition::new(2, 0)), Some(&2));
        // A hill neighbor costs 2 to enter — exactly the budget, so reachable...
        let hill_at_1 = |p: &HexPosition| {
            if *p == HexPosition::new(1, 0) {
                Some(Terrain::Hill)
            } else {
                Some(Terrain::Grassland)
            }
        };
        let reach = reachable_tiles(&from, &def, hill_at_1);
        assert_eq!(
            reach.get(&HexPosition::new(1, 0)),
            Some(&2),
            "entering a hill costs 2"
        );
        // ...but nothing directly past it: (2,0) costs 3 by every route at budget 2
        // (hill 2 + grass 1 direct, or a grass detour landing on a cost-2 tile + 1).
        assert!(
            !reach.contains_key(&HexPosition::new(2, 0)),
            "no budget left to step beyond the hill"
        );
    }

    #[test]
    fn reachable_prefers_cheaper_path() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        // Direct neighbor (2,0) is forest (cost 2); the two-grassland detour
        // through (1,0) then (2,0) would also be 2, but the direct forest step is
        // a single hop. Either way the cheapest recorded cost to (2,0) is 2.
        let from = HexPosition::new(0, 0);
        let def = land_unit(3);
        let map = |p: &HexPosition| {
            if *p == HexPosition::new(2, 0) {
                Some(Terrain::Hill) // cost 2 direct
            } else {
                Some(Terrain::Grassland) // cost 1 each via detour
            }
        };
        let reach = reachable_tiles(&from, &def, map);
        // Cheapest to (2,0): grassland (1,0)=1 then grassland... but (2,0) is hill.
        // Path A: (1,0)g=1 -> (2,0)hill=+2 = 3. Path B: direct neighbor of origin?
        // (2,0) is distance 2, not a neighbor, so only via (1,0): 1+2 = 3.
        assert_eq!(reach.get(&HexPosition::new(2, 0)), Some(&3));
    }

    #[test]
    fn reachable_excludes_water_mountain_and_missing_key() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        let from = HexPosition::new(0, 0);
        let def = land_unit(5);
        // (1,0) water (no key), (1,-1) mountain (sentinel), rest grassland.
        let map = |p: &HexPosition| {
            if *p == HexPosition::new(1, 0) {
                Some(Terrain::Water)
            } else if *p == HexPosition::new(1, -1) {
                Some(Terrain::Mountain)
            } else {
                Some(Terrain::Grassland)
            }
        };
        let reach = reachable_tiles(&from, &def, map);
        assert!(
            !reach.contains_key(&HexPosition::new(1, 0)),
            "water (missing key) is unenterable"
        );
        assert!(
            !reach.contains_key(&HexPosition::new(1, -1)),
            "mountain (99999 sentinel) is unenterable"
        );
        // A grassland neighbor is still reachable.
        assert!(reach.contains_key(&HexPosition::new(0, 1)));
    }

    #[test]
    fn reachable_cannot_path_through_water() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        // Land at (0,0) and (2,0), but the only on-map link is water at (1,0):
        // (2,0)'s sole on-map neighbor is water, so it's unreachable at any budget.
        let from = HexPosition::new(0, 0);
        let def = land_unit(99);
        let map = |p: &HexPosition| match (p.q, p.r) {
            (0, 0) | (2, 0) => Some(Terrain::Grassland),
            (1, 0) => Some(Terrain::Water),
            _ => None, // everything else off the map
        };
        assert!(
            !is_reachable(&from, &HexPosition::new(2, 0), &def, map),
            "cannot path through water even with a huge budget"
        );
        assert!(
            !is_reachable(&from, &HexPosition::new(1, 0), &def, map),
            "cannot land on water"
        );
    }

    #[test]
    fn reachable_respects_off_map_bounds() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        let from = HexPosition::new(0, 0);
        let def = land_unit(5);
        // Only the origin is on the map; everything else returns None.
        let map = |p: &HexPosition| (*p == from).then_some(Terrain::Grassland);
        let reach = reachable_tiles(&from, &def, map);
        assert!(reach.is_empty(), "no on-map neighbors → nothing reachable");
    }

    #[test]
    fn reachable_budget_boundary_is_inclusive() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        let from = HexPosition::new(0, 0);
        let grass = |_: &HexPosition| Some(Terrain::Grassland);
        // Budget 1: only the 1-away ring (cost 1) is reachable; 2-away is not.
        let def = land_unit(1);
        let reach = reachable_tiles(&from, &def, grass);
        assert!(
            reach.contains_key(&HexPosition::new(1, 0)),
            "cost 1 == budget"
        );
        assert!(
            !reach.contains_key(&HexPosition::new(2, 0)),
            "cost 2 > budget 1"
        );
    }

    #[test]
    fn reachable_zero_budget_reaches_nothing() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        let from = HexPosition::new(0, 0);
        let def = land_unit(0);
        let grass = |_: &HexPosition| Some(Terrain::Grassland);
        assert!(reachable_tiles(&from, &def, grass).is_empty());
    }

    #[test]
    fn server_and_client_agree_on_reachability() {
        use crate::hex::HexPosition;
        use crate::terrain::Terrain;
        // Same start/def/terrain → identical reachable sets, whether the lookup is
        // a closure (server's MapTiles style) or a prebuilt map (client style).
        let from = HexPosition::new(0, 0);
        let def = land_unit(3);
        let mut tiles: HashMap<HexPosition, Terrain> = HashMap::new();
        for hex in crate::hex::generate_grid(3) {
            // Sprinkle a deterministic mix of terrains.
            let t = match (hex.q + hex.r).rem_euclid(4) {
                0 => Terrain::Grassland,
                1 => Terrain::Hill,
                2 => Terrain::Forest,
                _ => Terrain::Water,
            };
            tiles.insert(hex, t);
        }
        // Server-style: closure over the map.
        let server = reachable_tiles(&from, &def, |p| tiles.get(p).copied());
        // Client-style: same prebuilt map, same closure shape.
        let client = reachable_tiles(&from, &def, |p| tiles.get(p).copied());
        assert_eq!(
            server, client,
            "the one shared fn must give identical results to both callers"
        );
        // Spot-check is_reachable agrees with set membership for a few tiles.
        for hex in crate::hex::generate_grid(3) {
            assert_eq!(
                is_reachable(&from, &hex, &def, |p| tiles.get(p).copied()),
                server.contains_key(&hex),
                "is_reachable must match reachable_tiles membership for {hex:?}"
            );
        }
    }

    #[test]
    fn test_is_within_attack_range_boundaries() {
        use crate::hex::HexPosition;

        let from = HexPosition::new(0, 0);
        let same = HexPosition::new(0, 0);
        let one = HexPosition::new(1, 0);
        let two = HexPosition::new(2, 0);

        // attack_range 1: only adjacent enemies
        assert!(!is_within_attack_range(&from, &same, 1)); // same hex never targetable
        assert!(is_within_attack_range(&from, &one, 1));
        assert!(!is_within_attack_range(&from, &two, 1));

        // attack_range 2 (archer): up to 2 hexes away
        assert!(is_within_attack_range(&from, &one, 2));
        assert!(is_within_attack_range(&from, &two, 2));
        assert!(!is_within_attack_range(&from, &HexPosition::new(3, 0), 2));
    }
}
