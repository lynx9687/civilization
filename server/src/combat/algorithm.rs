use bevy::prelude::Entity;
use shared::hex::HexPosition;
use std::collections::{HashMap, HashSet};

use super::types::{CityCapture, CitySnapshot, CombatDeltas, ResolveAction, UnitSnapshot};

/// Pure combat resolver. Borrows snapshots so the wrapper can keep the Vec
/// around for logging without cloning.
///
/// Units are placed optimistically at their desired tile, then conflicts are
/// drained per loop iteration:
/// (1) find a contested tile (2+ live units);
/// (2) if two of them share an owner the move *failed*, so roll the arriving
///     unit(s) (start_pos != tile) back to start with no damage — friendlies
///     never hit each other; otherwise apply one round of all-vs-all
///     simultaneous damage;
/// (3) settle the tile (≤1 survivor) or roll surviving combatants back to start.
///
/// City assaults resolve after unit conflicts, then the whole thing re-runs to a
/// fixpoint because a failed assault rolls its attacker back, which can re-create
/// a same-owner conflict with a follower that moved into the vacated tile.
///
/// Termination: every position write moves a unit *toward* its start_pos, and a
/// unit at its start is never advanced again, so each unit rolls back at most
/// once. With unique start positions, rollbacks and combat settlements are each
/// bounded by the unit count; the iteration cap is a generous sanity guard.
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
    let initial_city_hps: HashMap<Entity, i32> = cities
        .iter()
        .map(|city| (city.entity, city.hp.max(0)))
        .collect();
    let mut city_hps = initial_city_hps.clone();
    let mut city_owners: HashMap<Entity, Entity> = cities
        .iter()
        .map(|city| (city.entity, city.owner))
        .collect();
    let mut positions: HashMap<Entity, HexPosition> = HashMap::new();
    let mut hps: HashMap<Entity, i32> = HashMap::new();
    let mut deaths: HashSet<Entity> = HashSet::new();
    let mut city_captures = Vec::new();

    for u in units {
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

    // Single fixpoint: drain unit conflicts one at a time; when none remain,
    // resolve city assaults, and loop again only if an assault rolled an attacker
    // back (which can re-create a same-owner conflict with a follower that moved
    // into the vacated tile). `iter_count` is total iterations.
    // Generous O(N) sanity cap; see the termination note in the doc-comment.
    let iter_bound = 4 * units.len() as u32 + 16;
    let mut iter_count = 0_u32;
    loop {
        iter_count += 1;
        assert!(
            iter_count < iter_bound,
            "rollback chain failed to terminate"
        );

        if let Some((tile, combatants)) = next_conflict(&positions, &deaths) {
            // Same-owner co-location is a failed move, not a battle: roll back the
            // unit(s) that arrived here (start_pos != tile) and let the loop cascade
            // backward along the chain. The resident (start_pos == tile) keeps the
            // tile. Yielding before any damage is what stops friendlies from hitting
            // each other, and re-resolves a mixed friendly+enemy tile as a clean
            // melee once the friendly intruder steps off.
            let same_owner_movers: Vec<Entity> = combatants
                .iter()
                .copied()
                .filter(|&e| {
                    by_entity[&e].start_pos != tile
                        && combatants
                            .iter()
                            .any(|&o| o != e && by_entity[&o].owner == by_entity[&e].owner)
                })
                .collect();
            if !same_owner_movers.is_empty() {
                debug_assert!(
                    combatants
                        .iter()
                        .filter(|&&e| by_entity[&e].start_pos == tile)
                        .count()
                        <= 1,
                    "same-owner conflict must have at most one resident (in-degree / unique-start invariant)"
                );
                rollback_to_start(&same_owner_movers, &by_entity, &mut positions);
                continue;
            }

            let survivors = apply_combat_at_tile(&combatants, &by_entity, &mut hps, &mut deaths);
            if survivors.len() > 1 {
                rollback_to_start(&survivors, &by_entity, &mut positions);
            }
            // 0 or 1 alive: tile is settled; sole survivor (if any) stays at the conflict tile.
            continue;
        }

        // No unit conflicts remain. A failed city assault rolls its attacker back
        // onto its start tile, which can re-create a same-owner conflict with a
        // follower — so loop again to drain it; otherwise we are done.
        let city_rolled_back = resolve_city_melee(
            units,
            cities,
            &mut positions,
            &deaths,
            &mut city_hps,
            &mut city_owners,
            city_capture_hp,
            &mut city_captures,
        );
        if !city_rolled_back {
            break;
        }
    }

    let hp_changes: HashMap<Entity, i32> = hps
        .iter()
        .filter_map(|(e, &h)| {
            let initial = initial_hps.get(e).copied().unwrap_or(0);
            let delta = h - initial;
            if delta != 0 { Some((*e, delta)) } else { None }
        })
        .collect();
    let city_hp_changes: HashMap<Entity, i32> = city_hps
        .iter()
        .filter_map(|(e, &h)| {
            let initial = initial_city_hps.get(e).copied().unwrap_or(0);
            let delta = h - initial;
            if delta != 0 { Some((*e, delta)) } else { None }
        })
        .collect();

    CombatDeltas {
        hp_changes,
        city_hp_changes,
        city_captures,
        final_positions: positions,
        deaths,
    }
}

/// Returns true if it rolled any attacker back to its start (a failed assault),
/// signalling the caller to re-drain unit conflicts — the rolled-back attacker
/// may now share a tile with a follower that moved into the tile it vacated.
#[allow(clippy::too_many_arguments)]
fn resolve_city_melee(
    units: &[UnitSnapshot],
    cities: &[CitySnapshot],
    positions: &mut HashMap<Entity, HexPosition>,
    deaths: &HashSet<Entity>,
    city_hps: &mut HashMap<Entity, i32>,
    city_owners: &mut HashMap<Entity, Entity>,
    city_capture_hp: u32,
    city_captures: &mut Vec<CityCapture>,
) -> bool {
    let mut rolled_back = false;
    let mut attackers: Vec<_> = units
        .iter()
        .filter(|unit| unit.hp > 0)
        .filter(|unit| !deaths.contains(&unit.entity))
        .filter(|unit| unit.attack_range == 1)
        .filter_map(|unit| {
            let ResolveAction::MoveTo(target) = unit.action else {
                return None;
            };
            if positions.get(&unit.entity).copied() != Some(target) {
                return None;
            }
            Some((unit.entity, unit, target))
        })
        .collect();
    attackers.sort_by_key(|(entity, _, _)| *entity);

    for (unit_entity, unit, target) in attackers {
        let Some(city) = cities.iter().find(|city| city.pos == target) else {
            continue;
        };
        if city_owners.get(&city.entity).copied() == Some(unit.owner) {
            continue;
        }

        let city_hp = city_hps.entry(city.entity).or_insert(0);
        let before = *city_hp;
        *city_hp = city_hp.saturating_sub(unit.attack_damage as i32);

        if before <= 0 || *city_hp <= 0 {
            city_owners.insert(city.entity, unit.owner);
            *city_hp = (city_capture_hp.min(city.max_hp)) as i32;
            city_captures.push(CityCapture {
                city: city.entity,
                by_unit: unit_entity,
                new_owner: unit.owner,
            });
        } else {
            positions.insert(unit.entity, unit.start_pos);
            rolled_back = true;
        }
    }
    rolled_back
}

/// Find the next contested tile (2+ live units, excluding already-dead
/// entities), chosen deterministically as the smallest tile by (q, r). HashMap
/// iteration order is unspecified, so picking the min tile makes turn resolution
/// reproducible run-to-run (stable tests and replays). The combat outcome itself
/// is order-independent — all-vs-all damage is summed before it is applied, and
/// rollbacks/yields are keyed by entity — so the combatants list order does not
/// matter and is returned as-is. Returns the tile and its combatants, or None
/// when no tile has 2+ live units.
fn next_conflict(
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
    by_tile
        .into_iter()
        .filter(|(_, list)| list.len() >= 2)
        .min_by_key(|(tile, _)| (tile.q, tile.r))
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
        let (_world, entities) = fake_entities(4);
        // Distinct owners → enemy combat, not a friendly failed move.
        let p1 = entities[2];
        let p2 = entities[3];
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
        let (_world, entities) = fake_entities(4);
        // Distinct owners → enemy combat, not a friendly failed move.
        let p1 = entities[2];
        let p2 = entities[3];
        // A is much stronger and B is on its last legs.
        // A: 10/10 HP, 8 atk. B: 4/4 HP, 2 atk.
        // After exchange: A takes 2 → 8 HP (alive). B takes 8 → 0 HP (dead).
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p1,
                hp: 10,
                max_hp: 10,
                attack_damage: 8,
                attack_range: 1,
                start_pos: HexPosition::new(0, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p2,
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
        let (_world, entities) = fake_entities(4);
        // Distinct owners → enemy combat, not a friendly failed move.
        let p1 = entities[2];
        let p2 = entities[3];
        // Each does 10 damage, both have 8 HP.
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p1,
                hp: 8,
                max_hp: 8,
                attack_damage: 10,
                attack_range: 1,
                start_pos: HexPosition::new(0, 0),
                action: ResolveAction::MoveTo(HexPosition::new(1, 0)),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p2,
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
        let (_world, entities) = fake_entities(6);
        // Three distinct owners → full all-vs-all enemy combat.
        let p1 = entities[3];
        let p2 = entities[4];
        let p3 = entities[5];
        // All three converge on (1, 0). attack_damage = 4 each.
        // Each takes 4 + 4 = 8 damage; HPs are 10, 12, 14 → 2, 4, 6 — all alive.
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
                hp: 12,
                max_hp: 12,
                attack_damage: 4,
                attack_range: 1,
                start_pos: HexPosition::new(1, 0),
                action: ResolveAction::Stationary,
            },
            UnitSnapshot {
                entity: entities[2],
                owner: p3,
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
        let (_world, entities) = fake_entities(6);
        // Three distinct owners → each conflict is enemy combat (same cascade
        // shape a friendly chain takes, but with damage).
        let p1 = entities[3];
        let p2 = entities[4];
        let p3 = entities[5];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);

        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p1,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: t1,
                action: ResolveAction::MoveTo(t2),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p2,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: t2,
                action: ResolveAction::Stationary,
            },
            UnitSnapshot {
                entity: entities[2],
                owner: p3,
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
        let (_world, entities) = fake_entities(4);
        // Distinct owners: A hopes to catch enemy B, but B vacates first.
        let p1 = entities[2];
        let p2 = entities[3];
        // A moves T1→T2 hoping to hit B; B moves T2→T3, vacating T2.
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);
        let snapshot = vec![
            UnitSnapshot {
                entity: entities[0],
                owner: p1,
                hp: 10,
                max_hp: 10,
                attack_damage: 4,
                attack_range: 1,
                start_pos: t1,
                action: ResolveAction::MoveTo(t2),
            },
            UnitSnapshot {
                entity: entities[1],
                owner: p2,
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

    /// Standard warrior snapshot (hp 10, atk 4, range 1) for follow-move tests.
    fn warrior(
        entity: Entity,
        owner: Entity,
        start: HexPosition,
        action: ResolveAction,
    ) -> UnitSnapshot {
        UnitSnapshot {
            entity,
            owner,
            hp: 10,
            max_hp: 10,
            attack_damage: 4,
            attack_range: 1,
            start_pos: start,
            action,
        }
    }

    #[test]
    fn happy_chain_follows_into_vacated_tile() {
        let (_world, e) = fake_entities(3);
        let p = e[2];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);
        // A:T1→T2 follows B:T2→T3; T3 is empty.
        let snapshot = vec![
            warrior(e[0], p, t1, ResolveAction::MoveTo(t2)),
            warrior(e[1], p, t2, ResolveAction::MoveTo(t3)),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        assert_eq!(deltas.final_positions.get(&e[0]), Some(&t2));
        assert_eq!(deltas.final_positions.get(&e[1]), Some(&t3));
        assert!(deltas.hp_changes.is_empty(), "friendly follow → no combat");
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn three_cycle_rotation_resolves_without_damage() {
        let (_world, e) = fake_entities(4);
        let p = e[3];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);
        // A→B→C→A rotation; every destination is vacated simultaneously.
        let snapshot = vec![
            warrior(e[0], p, t1, ResolveAction::MoveTo(t2)),
            warrior(e[1], p, t2, ResolveAction::MoveTo(t3)),
            warrior(e[2], p, t3, ResolveAction::MoveTo(t1)),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        assert_eq!(deltas.final_positions.get(&e[0]), Some(&t2));
        assert_eq!(deltas.final_positions.get(&e[1]), Some(&t3));
        assert_eq!(deltas.final_positions.get(&e[2]), Some(&t1));
        assert!(deltas.hp_changes.is_empty(), "rotation → no combat");
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn two_unit_swap_resolves_without_damage() {
        let (_world, e) = fake_entities(3);
        let p = e[2];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let snapshot = vec![
            warrior(e[0], p, t1, ResolveAction::MoveTo(t2)),
            warrior(e[1], p, t2, ResolveAction::MoveTo(t1)),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        assert_eq!(deltas.final_positions.get(&e[0]), Some(&t2));
        assert_eq!(deltas.final_positions.get(&e[1]), Some(&t1));
        assert!(deltas.hp_changes.is_empty(), "swap → no combat");
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn stationary_friendly_occupant_rolls_back_mover_without_damage() {
        // The core new case: B doesn't actually vacate, so A's follow fails — and
        // friendlies must NOT damage each other (the old resolver dealt 4 to each).
        let (_world, e) = fake_entities(3);
        let p = e[2];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let snapshot = vec![
            warrior(e[0], p, t1, ResolveAction::MoveTo(t2)),
            warrior(e[1], p, t2, ResolveAction::Stationary),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        assert_eq!(deltas.final_positions.get(&e[0]), Some(&t1), "mover yields");
        assert_eq!(
            deltas.final_positions.get(&e[1]),
            Some(&t2),
            "resident stays"
        );
        assert!(deltas.hp_changes.is_empty(), "friendlies must not fight");
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn friendly_chain_blocked_by_stationary_tail_cascades_back() {
        // A→B→C but C is stationary; the failure cascades backward: B yields, then A.
        let (_world, e) = fake_entities(4);
        let p = e[3];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);
        let snapshot = vec![
            warrior(e[0], p, t1, ResolveAction::MoveTo(t2)),
            warrior(e[1], p, t2, ResolveAction::MoveTo(t3)),
            warrior(e[2], p, t3, ResolveAction::Stationary),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        assert_eq!(deltas.final_positions.get(&e[0]), Some(&t1));
        assert_eq!(deltas.final_positions.get(&e[1]), Some(&t2));
        assert_eq!(deltas.final_positions.get(&e[2]), Some(&t3));
        assert!(
            deltas.hp_changes.is_empty(),
            "no friendly fire in a stalled chain"
        );
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn mixed_tile_friendly_yields_before_enemy_combat() {
        // T2 ends up with friendly resident R, friendly follower F, and enemy E.
        // F must yield first so R↔E resolve as a clean melee — no friendly fire.
        let (_world, e) = fake_entities(5);
        let p = e[3];
        let q = e[4];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let te = HexPosition::new(1, 1);
        let (r, f, enemy) = (e[0], e[1], e[2]);
        let snapshot = vec![
            warrior(r, p, t2, ResolveAction::Stationary),
            warrior(f, p, t1, ResolveAction::MoveTo(t2)),
            warrior(enemy, q, te, ResolveAction::MoveTo(t2)),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        // Follower bounced home, untouched.
        assert_eq!(deltas.final_positions.get(&f), Some(&t1));
        assert!(
            !deltas.hp_changes.contains_key(&f),
            "follower takes no friendly fire"
        );
        // Resident and enemy traded one round, both survived and rolled apart.
        assert_eq!(deltas.final_positions.get(&r), Some(&t2));
        assert_eq!(deltas.final_positions.get(&enemy), Some(&te));
        assert_eq!(deltas.hp_changes.get(&r), Some(&-4));
        assert_eq!(deltas.hp_changes.get(&enemy), Some(&-4));
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn combat_broken_cycle_unwinds_to_start() {
        // Friendly 3-cycle A→B→C→A, but enemy E also rushes the tile C targets (T1).
        // C draws its melee and rolls back, unzipping the whole cycle to start.
        let (_world, e) = fake_entities(6);
        let p = e[4];
        let q = e[5];
        let t1 = HexPosition::new(0, 0);
        let t2 = HexPosition::new(1, 0);
        let t3 = HexPosition::new(2, 0);
        let te = HexPosition::new(0, 1);
        let (a, b, c, enemy) = (e[0], e[1], e[2], e[3]);
        let snapshot = vec![
            warrior(a, p, t1, ResolveAction::MoveTo(t2)),
            warrior(b, p, t2, ResolveAction::MoveTo(t3)),
            warrior(c, p, t3, ResolveAction::MoveTo(t1)),
            warrior(enemy, q, te, ResolveAction::MoveTo(t1)),
        ];

        let deltas = resolve_movement_pure(&snapshot, &[], 10);

        // Whole cycle unwound to start; enemy bounced home.
        assert_eq!(deltas.final_positions.get(&a), Some(&t1));
        assert_eq!(deltas.final_positions.get(&b), Some(&t2));
        assert_eq!(deltas.final_positions.get(&c), Some(&t3));
        assert_eq!(deltas.final_positions.get(&enemy), Some(&te));
        // Only C and E fought; A and B merely yielded.
        assert_eq!(deltas.hp_changes.get(&c), Some(&-4));
        assert_eq!(deltas.hp_changes.get(&enemy), Some(&-4));
        assert!(!deltas.hp_changes.contains_key(&a));
        assert!(!deltas.hp_changes.contains_key(&b));
        assert!(deltas.deaths.is_empty());
    }

    #[test]
    fn failed_city_assault_rolls_back_follower_no_double_stack() {
        // Regression: H assaults an enemy city and fails; follower M moved into H's
        // vacated tile. City-melee rollback must re-enter conflict resolution so M
        // also yields — no two friendlies left stacked on one tile.
        let (_world, e) = fake_entities(5);
        let p = e[2];
        let q = e[3];
        let city = e[4];
        let s_m = HexPosition::new(0, 0);
        let s_h = HexPosition::new(1, 0);
        let city_pos = HexPosition::new(2, 0);
        let snapshot = vec![
            warrior(e[0], p, s_h, ResolveAction::MoveTo(city_pos)), // H assaults city
            warrior(e[1], p, s_m, ResolveAction::MoveTo(s_h)),      // M follows into H's tile
        ];
        let cities = vec![CitySnapshot {
            entity: city,
            owner: q,
            hp: 20,
            max_hp: 20,
            pos: city_pos,
        }];

        let deltas = resolve_movement_pure(&snapshot, &cities, 10);

        // Both rolled back to their own start — no shared tile.
        assert_eq!(deltas.final_positions.get(&e[0]), Some(&s_h));
        assert_eq!(deltas.final_positions.get(&e[1]), Some(&s_m));
        assert_ne!(
            deltas.final_positions.get(&e[0]),
            deltas.final_positions.get(&e[1]),
            "followers must not double-stack after a failed city assault"
        );
        // City damaged but held; no capture; no unit-vs-unit combat.
        assert_eq!(deltas.city_hp_changes.get(&city), Some(&-4));
        assert!(deltas.city_captures.is_empty());
        assert!(deltas.hp_changes.is_empty());
        assert!(deltas.deaths.is_empty());
    }
}
