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
}
