// Types and function are stubs; future tasks will wire them into ECS systems.

use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

/// One row per live unit, gathered by the wrapper system before calling the algorithm.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct UnitSnapshot {
    pub entity: Entity,
    pub owner: Entity,
    pub hp: i32,
    pub max_hp: u32,
    pub attack_damage: u32,
    pub attack_range: u32,
    pub start_pos: HexPosition,
    pub action: ResolveAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveAction {
    /// No movement this turn. Used in tests; production systems added in a later task.
    #[allow(dead_code)]
    Stationary,
    /// Move to this destination. Triggers melee combat if an enemy ends up there too.
    #[allow(dead_code)]
    MoveTo(HexPosition),
}

#[derive(Default, Debug)]
#[allow(dead_code)]
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}

/// Pure combat resolver. Expanded task by task.
#[allow(dead_code)]
pub fn resolve_movement_pure(units: Vec<UnitSnapshot>) -> CombatDeltas {
    let mut positions: HashMap<Entity, HexPosition> = HashMap::new();
    let mut hps: HashMap<Entity, i32> = HashMap::new();
    let mut deaths: HashSet<Entity> = HashSet::new();

    let initial_hps: HashMap<Entity, i32> = units
        .iter()
        .filter(|u| u.hp > 0)
        .map(|u| (u.entity, u.hp))
        .collect();

    // Index snapshot by entity for fast lookup later.
    let by_entity: HashMap<Entity, &UnitSnapshot> = units.iter().map(|u| (u.entity, u)).collect();

    for u in &units {
        if u.hp <= 0 {
            continue;
        }
        let desired = match u.action {
            ResolveAction::Stationary => u.start_pos,
            ResolveAction::MoveTo(t) => t,
        };
        positions.insert(u.entity, desired);
        hps.insert(u.entity, u.hp);
    }

    // Iterate: detect conflicts, fight, rollback, repeat until stable.
    let mut iter_count = 0_u32;
    loop {
        iter_count += 1;
        assert!(iter_count < 256, "rollback chain failed to terminate");

        // Find first tile with 2+ live units.
        let mut by_tile: HashMap<HexPosition, Vec<Entity>> = HashMap::new();
        for (&e, &p) in &positions {
            if deaths.contains(&e) {
                continue;
            }
            by_tile.entry(p).or_default().push(e);
        }
        let conflict_tile = by_tile
            .iter()
            .find(|(_, list)| list.len() >= 2)
            .map(|(p, _)| *p);
        let Some(tile) = conflict_tile else {
            break;
        };
        let combatants: Vec<Entity> = by_tile.get(&tile).cloned().unwrap_or_default();

        // All-vs-all simultaneous damage.
        // Each combatant takes the sum of every other combatant's attack_damage.
        let damages_to_take: HashMap<Entity, i32> = combatants
            .iter()
            .map(|&e| {
                let raw: u32 = combatants
                    .iter()
                    .filter(|&&v| v != e)
                    .map(|&v| by_entity[&v].attack_damage)
                    .sum();
                (e, raw as i32)
            })
            .collect();
        for (&e, &dmg) in &damages_to_take {
            *hps.get_mut(&e).unwrap() -= dmg;
        }

        // Apply this round's deaths.
        for &e in &combatants {
            if hps[&e] <= 0 {
                deaths.insert(e);
            }
        }

        let survivors: Vec<Entity> = combatants
            .iter()
            .copied()
            .filter(|e| !deaths.contains(e))
            .collect();

        if survivors.len() > 1 {
            // 2+ alive — all rollback to start_pos. Home unit's rollback is a no-op.
            for s in &survivors {
                positions.insert(*s, by_entity[s].start_pos);
            }
        }
        // 0 or 1 alive: tile settled. Sole survivor (if any) is already at `tile`.
    }

    // Build hp_changes (delta) from initial.
    let hp_changes: HashMap<Entity, i32> = hps
        .iter()
        .filter_map(|(e, &h)| {
            let initial = initial_hps.get(e).copied().unwrap_or(0);
            let delta = h - initial;
            if delta != 0 { Some((*e, delta)) } else { None }
        })
        .collect();

    CombatDeltas {
        hp_changes,
        final_positions: positions,
        deaths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    /// Helper: spawn N empty entities in a fresh World and return their ids.
    /// Lets tests construct distinct Entity values without setting up full ECS state.
    fn fake_entities(n: usize) -> (World, Vec<Entity>) {
        let mut world = World::new();
        let entities: Vec<_> = (0..n).map(|_| world.spawn_empty().id()).collect();
        (world, entities)
    }

    #[test]
    fn empty_input_returns_empty_deltas() {
        let deltas = resolve_movement_pure(vec![]);
        assert!(deltas.hp_changes.is_empty());
        assert!(deltas.final_positions.is_empty());
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn two_way_conflict_both_alive_rolls_back() {
        let (_world, entities) = fake_entities(2);
        let p1 = Entity::PLACEHOLDER;
        let p2 = Entity::PLACEHOLDER;
        // A at (0,0) moves to (1,0); B at (1,0) is stationary.
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p1,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: HexPosition::new(0, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p2,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: HexPosition::new(1, 0),
                action: ResolveAction::Stationary,
            },
        ];

        let deltas = resolve_movement_pure(snapshot);

        // Both alive → both rolled back to start. A back to (0,0), B stays at (1,0).
        assert_eq!(
            deltas.final_positions.get(&entities[0]),
            Some(&HexPosition::new(0, 0))
        );
        assert_eq!(
            deltas.final_positions.get(&entities[1]),
            Some(&HexPosition::new(1, 0))
        );
        // Each took 4 damage.
        assert_eq!(deltas.hp_changes.get(&entities[0]), Some(&-4));
        assert_eq!(deltas.hp_changes.get(&entities[1]), Some(&-4));
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn two_way_conflict_one_survivor_takes_tile() {
        let (_world, entities) = fake_entities(2);
        let p = Entity::PLACEHOLDER;
        // A is much stronger and B is on its last legs.
        // A: 10/10 HP, 8 atk. B: 4/4 HP, 2 atk.
        // After exchange: A takes 2 → 8 HP (alive). B takes 8 → 0 HP (dead).
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p,
                hp: 10,
                max_hp: 10,
                attack_damage: 8,
                attack_range: 1,
                start_pos: HexPosition::new(0, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p,
                hp: 4,
                max_hp: 4,
                attack_damage: 2,
                attack_range: 1,
                start_pos: HexPosition::new(1, 0),
                action: ResolveAction::Stationary,
            },
        ];

        let deltas = resolve_movement_pure(snapshot);

        assert!(deltas.deaths.contains(&entities[1]), "B should be dead");
        assert!(!deltas.deaths.contains(&entities[0]), "A should be alive");
        // A (sole survivor) takes the tile.
        assert_eq!(
            deltas.final_positions.get(&entities[0]),
            Some(&HexPosition::new(1, 0))
        );
        assert_eq!(deltas.hp_changes.get(&entities[0]), Some(&-2));
        assert_eq!(deltas.hp_changes.get(&entities[1]), Some(&-8));
    }

    #[test]
    fn two_way_conflict_both_die_tile_empty() {
        let (_world, entities) = fake_entities(2);
        let p = Entity::PLACEHOLDER;
        // Each does 10 damage, both have 8 HP.
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p,
                hp: 8,
                max_hp: 8,
                attack_damage: 10,
                attack_range: 1,
                start_pos: HexPosition::new(0, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p,
                hp: 8,
                max_hp: 8,
                attack_damage: 10,
                attack_range: 1,
                start_pos: HexPosition::new(1, 0),
                action: ResolveAction::Stationary,
            },
        ];

        let deltas = resolve_movement_pure(snapshot);

        assert!(deltas.deaths.contains(&entities[0]));
        assert!(deltas.deaths.contains(&entities[1]));
    }

    #[test]
    fn three_way_conflict_each_takes_sum_of_others() {
        let (_world, entities) = fake_entities(3);
        let p = Entity::PLACEHOLDER;
        // All three converge on (1, 0). attack_damage = 4 each.
        // Each takes 4 + 4 = 8 damage; HPs are 10, 12, 14 → 2, 4, 6 — all alive.
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: HexPosition::new(0, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p,
                hp: 12,
                max_hp: 12,
                attack_damage: 4,
                attack_range: 1,
                start_pos: HexPosition::new(1, 0),
                action: ResolveAction::Stationary,
            },
            UnitSnapshot {
                entity: entities[2],
                owner: p,
                hp: 14,
                max_hp: 14,
                attack_damage: 4,
                attack_range: 1,
                start_pos: HexPosition::new(2, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
        ];

        let deltas = resolve_movement_pure(snapshot);

        // Each took 8 damage from the other two.
        assert_eq!(deltas.hp_changes.get(&entities[0]), Some(&-8));
        assert_eq!(deltas.hp_changes.get(&entities[1]), Some(&-8));
        assert_eq!(deltas.hp_changes.get(&entities[2]), Some(&-8));
        // 3 alive → all rollback to start.
        assert_eq!(
            deltas.final_positions.get(&entities[0]),
            Some(&HexPosition::new(0, 0))
        );
        assert_eq!(
            deltas.final_positions.get(&entities[1]),
            Some(&HexPosition::new(1, 0))
        );
        assert_eq!(
            deltas.final_positions.get(&entities[2]),
            Some(&HexPosition::new(2, 0))
        );
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn chain_combat_worked_example() {
        // Setup: A@T1, B@T2, C@T3, all warriors (hp=10, attack_damage=4).
        // Actions: A moves to T2; C moves to T1; B stationary.
        // Iter 1: T2 conflict (A, B) → both -4. Survivors rollback. A→T1, B→T2 (no-op).
        // Iter 2: T1 conflict (A, C) → both -4 (A now at 6, C at 10).
        //         A→2 HP, C→6 HP. Survivors rollback. A→T1 (no-op), C→T3.
        // Final: A@T1 (2/10), B@T2 (6/10), C@T3 (6/10).
        let (_world, entities) = fake_entities(3);
        let p = Entity::PLACEHOLDER;
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);

        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: t1,
                action: ResolveAction::MoveTo(t2),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: t2,
                action: ResolveAction::Stationary,
            },
            UnitSnapshot {
                entity: entities[2],
                owner: p,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: t3,
                action: ResolveAction::MoveTo(t1),
            },
        ];

        let deltas = resolve_movement_pure(snapshot);

        assert_eq!(deltas.final_positions.get(&entities[0]), Some(&t1));
        assert_eq!(deltas.final_positions.get(&entities[1]), Some(&t2));
        assert_eq!(deltas.final_positions.get(&entities[2]), Some(&t3));

        assert_eq!(
            deltas.hp_changes.get(&entities[0]),
            Some(&-8),
            "A took two rounds"
        );
        assert_eq!(
            deltas.hp_changes.get(&entities[1]),
            Some(&-4),
            "B took one round"
        );
        assert_eq!(
            deltas.hp_changes.get(&entities[2]),
            Some(&-4),
            "C took one round"
        );
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn single_mover_lands_at_destination() {
        let (_world, entities) = fake_entities(1);
        let player = Entity::PLACEHOLDER;
        let snapshot = vec![UnitSnapshot {
            entity: entities[0],
            owner: player,
            hp: 10,
            max_hp: 10,
            attack_damage: 4,
            attack_range: 1,
            start_pos: HexPosition::new(0, 0),
            action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
        }];

        let deltas = resolve_movement_pure(snapshot);

        assert_eq!(
            deltas.final_positions.get(&entities[0]),
            Some(&HexPosition::new(1, 0))
        );
        assert!(
            deltas.hp_changes.is_empty(),
            "no damage in a non-conflict move"
        );
        assert!(deltas.deaths.is_empty());
    }
}
