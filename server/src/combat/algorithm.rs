use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

use super::types::{CityCapture, CitySnapshot, CombatDeltas, ResolveAction, UnitSnapshot};

/// Pure combat resolver. Borrows snapshots so the wrapper can keep the Vec
/// around for logging without cloning.
///
/// The algorithm has three phases per loop iteration:
/// (1) find a tile with 2+ live units (a conflict),
/// (2) apply one round of all-vs-all simultaneous damage there,
/// (3) either settle the tile (≤1 survivor) or rollback survivors to start_pos.
///
/// Iteration stops when no conflict remains. Termination is guaranteed by
/// `start_pos` uniqueness (enforced by handle_unit_action's friendly-stack
/// checks); the 256-iteration cap is a sanity guard.
pub fn resolve_movement_pure(
    units: &[UnitSnapshot],
    cities: &[CitySnapshot],
    city_capture_hp: u32,
) -> CombatDeltas {
    // Snapshot-derived indices and initial working state.
    let initial_hps: HashMap<Entity, i32> = units
        .iter()
        .filter(|u| u.hp > 0)
        .map(|u| (u.entity, u.hp))
        .collect();
    let by_entity: HashMap<Entity, &UnitSnapshot> = units.iter().map(|u| (u.entity, u)).collect();
    let mut unit_positions: HashMap<Entity, HexPosition> = HashMap::new();
    let mut unit_hps: HashMap<Entity, i32> = HashMap::new();
    let mut unit_deaths: HashSet<Entity> = HashSet::new();
    let initial_city_hps: HashMap<Entity, i32> = cities
        .iter()
        .map(|city| (city.entity, city.hp.max(0)))
        .collect();
    let mut city_working = cities.to_vec();
    let mut city_captures = Vec::new();

    for u in units {
        if u.hp <= 0 {
            continue;
        }
        let desired = match u.action {
            ResolveAction::Stationary => u.start_pos,
            ResolveAction::MoveTo(t) => t,
        };
        unit_positions.insert(u.entity, desired);
        unit_hps.insert(u.entity, u.hp);
    }

    let mut iter_count = 0_u32;
    loop {
        iter_count += 1;
        assert!(iter_count < 256, "rollback chain failed to terminate");

        let Some((_tile, combatants)) = find_first_conflict(&unit_positions, &unit_deaths) else {
            break;
        };
        let survivors =
            apply_combat_at_tile(&combatants, &by_entity, &mut unit_hps, &mut unit_deaths);
        if survivors.len() > 1 {
            rollback_to_start(&survivors, &by_entity, &mut unit_positions);
        }
        // 0 or 1 alive: tile is already settled; sole survivor (if any) stays at the conflict tile.
    }

    resolve_city_melee(
        units,
        &mut city_working,
        &mut unit_positions,
        &unit_deaths,
        city_capture_hp,
        &mut city_captures,
    );

    let hp_changes: HashMap<Entity, i32> = unit_hps
        .iter()
        .filter_map(|(e, &h)| {
            let initial = initial_hps.get(e).copied().unwrap_or(0);
            let delta = h - initial;
            if delta != 0 { Some((*e, delta)) } else { None }
        })
        .collect();
    let city_hp_changes: HashMap<Entity, i32> = city_working
        .iter()
        .filter_map(|city| {
            let initial = initial_city_hps.get(&city.entity).copied().unwrap_or(0);
            let delta = city.hp - initial;
            if delta != 0 {
                Some((city.entity, delta))
            } else {
                None
            }
        })
        .collect();

    CombatDeltas {
        hp_changes,
        city_hp_changes,
        city_captures,
        final_positions: unit_positions,
        deaths: unit_deaths,
    }
}

fn resolve_city_melee(
    units: &[UnitSnapshot],
    cities: &mut [CitySnapshot],
    unit_positions: &mut HashMap<Entity, HexPosition>,
    unit_deaths: &HashSet<Entity>,
    city_capture_hp: u32,
    city_captures: &mut Vec<CityCapture>,
) {
    let mut attackers: Vec<_> = units
        .iter()
        .filter(|unit| unit.hp > 0)
        .filter(|unit| !unit_deaths.contains(&unit.entity))
        .filter(|unit| unit.attack_range == 1)
        .filter_map(|unit| {
            let ResolveAction::MoveTo(target) = unit.action else {
                return None;
            };
            if unit_positions.get(&unit.entity).copied() != Some(target) {
                return None;
            }
            Some((unit.entity, unit, target))
        })
        .collect();
    attackers.sort_by_key(|(entity, _, _)| *entity);

    for (unit_entity, unit, target) in attackers {
        let Some(city) = cities.iter_mut().find(|city| city.pos == target) else {
            continue;
        };
        if city.owner == unit.owner {
            continue;
        }

        let before = city.hp;
        city.hp = city.hp.saturating_sub(unit.attack_damage as i32);

        if before <= 0 || city.hp <= 0 {
            city.owner = unit.owner;
            city.hp = (city_capture_hp.min(city.max_hp)) as i32;
            city_captures.push(CityCapture {
                city: city.entity,
                by_unit: unit_entity,
                new_owner: unit.owner,
            });
        } else {
            unit_positions.insert(unit.entity, unit.start_pos);
        }
    }
}

/// Find the first tile that has 2+ live units (excluding already-dead entities).
/// Returns the tile and its combatants, or None when the world is conflict-free.
fn find_first_conflict(
    positions: &HashMap<Entity, HexPosition>,
    deaths: &HashSet<Entity>,
) -> Option<(HexPosition, Vec<Entity>)> {
    let mut by_tile: HashMap<HexPosition, Vec<Entity>> = HashMap::new();
    for (&e, &p) in positions {
        if deaths.contains(&e) {
            continue;
        }
        by_tile.entry(p).or_default().push(e);
    }
    by_tile.into_iter().find(|(_, list)| list.len() >= 2)
}

/// One round of all-vs-all simultaneous damage among `combatants` on the same tile.
/// Each combatant takes the sum of every other combatant's `attack_damage`.
/// Mutates `hps` (damage applied) and `deaths` (anyone reaching HP ≤ 0 this round).
/// Returns the survivors of this round.
fn apply_combat_at_tile(
    combatants: &[Entity],
    by_entity: &HashMap<Entity, &UnitSnapshot>,
    hps: &mut HashMap<Entity, i32>,
    deaths: &mut HashSet<Entity>,
) -> Vec<Entity> {
    // Compute damages first so the exchange is simultaneous (no kill-before-strike).
    let damages: HashMap<Entity, i32> = combatants
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
    for (&e, &dmg) in &damages {
        *hps.get_mut(&e).unwrap() -= dmg;
    }
    for &e in combatants {
        if hps[&e] <= 0 {
            deaths.insert(e);
        }
    }
    combatants
        .iter()
        .copied()
        .filter(|e| !deaths.contains(e))
        .collect()
}

/// Send each survivor back to its turn-start position. The home unit's "rollback"
/// is a no-op since its start_pos equals the conflict tile.
fn rollback_to_start(
    survivors: &[Entity],
    by_entity: &HashMap<Entity, &UnitSnapshot>,
    positions: &mut HashMap<Entity, HexPosition>,
) {
    for s in survivors {
        positions.insert(*s, by_entity[s].start_pos);
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
        let deltas = resolve_movement_pure(&[], &[], 10);
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

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

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

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

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

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

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

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

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

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

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
    fn move_into_enemy_misses_when_defender_vacated() {
        let (_world, entities) = fake_entities(2);
        let p = Entity::PLACEHOLDER;
        // A moves T1→T2 hoping to hit B; B moves T2→T3, vacating T2.
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
                action: ResolveAction::MoveTo(t3),
            },
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        // No conflict ever forms; both units end up at their destinations.
        assert_eq!(deltas.final_positions.get(&entities[0]), Some(&t2));
        assert_eq!(deltas.final_positions.get(&entities[1]), Some(&t3));
        assert!(deltas.hp_changes.is_empty(), "no combat happened");
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

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

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

    #[test]
    fn melee_city_attack_captures_and_keeps_unit_on_city() {
        let (_world, entities) = fake_entities(2);
        let attacker_owner = entities[0];
        let defender_owner = entities[1];
        let city = Entity::PLACEHOLDER;
        let city_pos = HexPosition::new(1, 0);
        let snapshot = vec![UnitSnapshot {
            entity: entities[0],
            owner: attacker_owner,
            hp: 10,
            max_hp: 10,
            attack_damage: 4,
            attack_range: 1,
            start_pos: HexPosition::new(0, 0),
            action: ResolveAction::MoveTo(city_pos),
        }];
        let cities = vec![CitySnapshot {
            entity: city,
            owner: defender_owner,
            hp: 3,
            max_hp: 20,
            pos: city_pos,
        }];

        let deltas = resolve_movement_pure(&snapshot, &cities, 10);

        assert_eq!(deltas.final_positions.get(&entities[0]), Some(&city_pos));
        assert_eq!(deltas.city_hp_changes.get(&city), Some(&7));
        assert_eq!(
            deltas.city_captures,
            vec![CityCapture {
                city,
                by_unit: entities[0],
                new_owner: attacker_owner,
            }]
        );
    }

    #[test]
    fn melee_city_attack_rolls_back_when_city_survives() {
        let (_world, entities) = fake_entities(2);
        let attacker_owner = entities[0];
        let defender_owner = entities[1];
        let city = Entity::PLACEHOLDER;
        let start = HexPosition::new(0, 0);
        let city_pos = HexPosition::new(1, 0);
        let snapshot = vec![UnitSnapshot {
            entity: entities[0],
            owner: attacker_owner,
            hp: 10,
            max_hp: 10,
            attack_damage: 4,
            attack_range: 1,
            start_pos: start,
            action: ResolveAction::MoveTo(city_pos),
        }];
        let cities = vec![CitySnapshot {
            entity: city,
            owner: defender_owner,
            hp: 8,
            max_hp: 20,
            pos: city_pos,
        }];

        let deltas = resolve_movement_pure(&snapshot, &cities, 10);

        assert_eq!(deltas.final_positions.get(&entities[0]), Some(&start));
        assert_eq!(deltas.city_hp_changes.get(&city), Some(&-4));
        assert!(deltas.city_captures.is_empty());
    }

    #[test]
    fn ranged_unit_moving_to_city_does_not_melee_capture() {
        let (_world, entities) = fake_entities(2);
        let attacker_owner = entities[0];
        let defender_owner = entities[1];
        let city = Entity::PLACEHOLDER;
        let city_pos = HexPosition::new(1, 0);
        let snapshot = vec![UnitSnapshot {
            entity: entities[0],
            owner: attacker_owner,
            hp: 10,
            max_hp: 10,
            attack_damage: 20,
            attack_range: 2,
            start_pos: HexPosition::new(0, 0),
            action: ResolveAction::MoveTo(city_pos),
        }];
        let cities = vec![CitySnapshot {
            entity: city,
            owner: defender_owner,
            hp: 1,
            max_hp: 20,
            pos: city_pos,
        }];

        let deltas = resolve_movement_pure(&snapshot, &cities, 10);

        assert_eq!(deltas.final_positions.get(&entities[0]), Some(&city_pos));
        assert!(deltas.city_hp_changes.is_empty());
        assert!(deltas.city_captures.is_empty());
    }
}
