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

    pub fn load_from_dir(dir: &std::path::Path) -> Result<Self, LoadError> {
        let mut name_to_id = HashMap::new();
        let mut definitions = HashMap::new();
        let mut next_id: u8 = 0;
        let read_dir = std::fs::read_dir(dir).map_err(|e| LoadError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        // Collect and sort by path for deterministic ID assignment.
        // Server and client must agree on (name → id), so the order can't
        // depend on filesystem-arbitrary read_dir order.
        let mut entries: Vec<_> =
            read_dir
                .collect::<Result<_, _>>()
                .map_err(|e| LoadError::Io {
                    path: dir.to_path_buf(),
                    source: e,
                })?;
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
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
            let def: UnitDefinition = ron::from_str(&contents).map_err(|e| LoadError::Parse {
                path: path.clone(),
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

/// Startup system that loads `UnitRegistry` from `assets/units/` and inserts it as a resource.
/// Registered by `SharedPlugin` so both server and client get it for free.
pub fn load_unit_registry(mut commands: Commands) {
    let path = std::path::Path::new("assets/units");
    match UnitRegistry::load_from_dir(path) {
        Ok(registry) => {
            println!(
                "Loaded {} unit definitions from {}",
                registry.definitions.len(),
                path.display()
            );
            commands.insert_resource(registry);
        }
        Err(e) => {
            panic!("Failed to load unit registry from {}: {e}", path.display());
        }
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
    if def.attack_damage > 0 {
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
    fn test_available_verbs_for_warrior_class() {
        let registry = UnitRegistry::load_from_dir(std::path::Path::new("../assets/units"))
            .expect("should load");
        for name in ["warrior", "archer", "cavalry", "knight"] {
            let def = registry
                .get(&registry.id_of(name).expect(name))
                .expect(name);
            let verbs = available_verbs(def);
            assert!(verbs.contains(&UnitVerb::Move), "{name} should Move");
            assert!(verbs.contains(&UnitVerb::Attack), "{name} should Attack");
            assert!(verbs.contains(&UnitVerb::Fortify), "{name} should Fortify");
            assert!(verbs.contains(&UnitVerb::Skip), "{name} should Skip");
            assert!(!verbs.contains(&UnitVerb::Build), "{name} cannot Build");
        }
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
        // settler has attack_damage = 0 → Attack must be absent
        assert!(!verbs.contains(&UnitVerb::Attack));
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
