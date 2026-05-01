use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

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

#[derive(Resource, Default, Debug)]
pub struct UnitRegistry {
    pub definitions: HashMap<String, UnitDefinition>,
}

impl UnitRegistry {
    pub fn get(&self, name: &str) -> Option<&UnitDefinition> {
        self.definitions.get(name)
    }

    pub fn load_from_dir(dir: &std::path::Path) -> Result<Self, LoadError> {
        let mut definitions = HashMap::new();
        let entries = std::fs::read_dir(dir).map_err(|e| LoadError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| LoadError::Io {
                path: dir.to_path_buf(),
                source: e,
            })?;
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
            definitions.insert(name, def);
        }
        Ok(UnitRegistry { definitions })
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
        assert!(registry.get("warrior").is_some());
        assert!(registry.get("archer").is_some());
        assert!(registry.get("cavalry").is_some());
        assert!(registry.get("knight").is_some());
        assert!(registry.get("settler").is_some());
        assert_eq!(registry.definitions.len(), 5);

        let warrior = registry.get("warrior").unwrap();
        assert_eq!(warrior.hp, 10);
        assert_eq!(warrior.move_budget, 2);
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
}
