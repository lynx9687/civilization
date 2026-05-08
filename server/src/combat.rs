use bevy::prelude::*;
use shared::hex::HexPosition;
use shared::unit_definition::UnitRegistry;
use shared::units::{AttackTarget, Health, MoveTo, Owner, Unit};
use std::collections::{HashMap, HashSet};

/// One row per live unit, gathered by the wrapper system before calling the algorithm.
#[derive(Clone, Debug)]
pub struct UnitSnapshot {
    pub entity: Entity,
    // owner, max_hp, and attack_range are captured for future algorithm expansions
    // (e.g. faction-aware combat, morale, extended-range melee) but not yet read.
    #[allow(dead_code)]
    pub owner: Entity,
    pub hp: i32,
    #[allow(dead_code)]
    pub max_hp: u32,
    pub attack_damage: u32,
    #[allow(dead_code)]
    pub attack_range: u32,
    pub start_pos: HexPosition,
    pub action: ResolveAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveAction {
    /// No movement this turn.
    Stationary,
    /// Move to this destination. Triggers melee combat if an enemy ends up there too.
    MoveTo(HexPosition),
}

#[derive(Default, Debug)]
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}

/// Pure combat resolver. Expanded task by task.
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

/// Pre-movement phase: each ranged unit with an AttackTarget hits the live unit
/// at the target tile (if any) for `attack_damage`, with no counter and no movement.
/// Submit-time validation already guaranteed an enemy was at the tile at submit;
/// a missing target here means concentrated-fire killed it earlier in this loop,
/// or the marker survived a target despawn — either way the attack is wasted.
pub fn resolve_ranged_attacks(
    attackers: Query<(Entity, &Unit, &AttackTarget)>,
    // Health excluded here to avoid conflicting borrows with hp_q below;
    // liveness is checked via hp_q after we find the candidate by position.
    unit_positions: Query<(Entity, &HexPosition), With<Unit>>,
    registry: Res<UnitRegistry>,
    mut hp_q: Query<&mut Health>,
    mut commands: Commands,
) {
    let mut acts: Vec<_> = attackers.iter().collect();
    acts.sort_by_key(|(e, _, _)| *e);

    for (attacker_entity, unit, attack_target) in acts {
        let Some(def) = registry.get(&unit.type_id) else {
            commands.entity(attacker_entity).remove::<AttackTarget>();
            continue;
        };
        if def.attack_range <= 1 {
            // Melee Attack should have been gated out by available_verbs;
            // safety net: just consume the marker.
            commands.entity(attacker_entity).remove::<AttackTarget>();
            continue;
        }

        // Find the live unit at the target tile.
        let target_entity = unit_positions
            .iter()
            .find(|(e, p)| {
                **p == attack_target.pos && hp_q.get(*e).map(|h| h.current > 0).unwrap_or(false)
            })
            .map(|(e, _)| e);

        if let Some(te) = target_entity
            && let Ok(mut h) = hp_q.get_mut(te)
        {
            let before = h.current;
            h.current = h.current.saturating_sub(def.attack_damage);
            println!(
                "Ranged: {attacker_entity} hit {te} at {:?} for {} dmg (hp {} -> {})",
                attack_target.pos, def.attack_damage, before, h.current
            );
        } else {
            println!(
                "Ranged: {attacker_entity} attack at {:?} wasted (no live target)",
                attack_target.pos
            );
        }
        commands.entity(attacker_entity).remove::<AttackTarget>();
    }
}

/// ECS wrapper around `resolve_movement_pure`. Snapshots live units,
/// runs the algorithm, applies HP / position deltas, and consumes MoveTo markers.
///
/// Uses a `ParamSet` to avoid B0001: the read query (for snapshot building)
/// and the write queries (for applying deltas) share `Health` and `HexPosition`,
/// so Bevy requires them to be declared in a `ParamSet` to prove they are
/// used exclusively, not concurrently.
#[allow(clippy::type_complexity)] // ParamSet with a 6-tuple query; extracting a type alias gains little
pub fn resolve_movement(
    mut queries: ParamSet<(
        // p0: read-only snapshot query
        Query<(
            Entity,
            &Owner,
            &HexPosition,
            &Health,
            &Unit,
            Option<&MoveTo>,
        )>,
        // p1: mutable HP write-back
        Query<&mut Health>,
        // p2: mutable position write-back
        Query<&mut HexPosition>,
    )>,
    registry: Res<UnitRegistry>,
    mut commands: Commands,
) {
    // 1. Build snapshot and record which entities had MoveTo — immutable pass only.
    let mut had_move_to: Vec<Entity> = Vec::new();
    let mut start_positions: HashMap<Entity, HexPosition> = HashMap::new();
    let mut intent: HashMap<Entity, HexPosition> = HashMap::new();
    let snapshot: Vec<UnitSnapshot> = queries
        .p0()
        .iter()
        .filter(|(_, _, _, h, _, _)| h.current > 0)
        .filter_map(|(e, owner, pos, h, unit, move_to)| {
            let def = registry.get(&unit.type_id)?;
            start_positions.insert(e, *pos);
            let action = match move_to {
                Some(m) => {
                    had_move_to.push(e);
                    intent.insert(e, m.pos);
                    ResolveAction::MoveTo(m.pos)
                }
                None => ResolveAction::Stationary,
            };
            Some(UnitSnapshot {
                entity: e,
                owner: owner.0,
                hp: h.current as i32,
                max_hp: h.max,
                attack_damage: def.attack_damage,
                attack_range: def.attack_range,
                start_pos: *pos,
                action,
            })
        })
        .collect();

    // 2. Run pure algorithm.
    let deltas = resolve_movement_pure(snapshot);

    // 3. Apply HP changes via p1.
    for (e, delta) in &deltas.hp_changes {
        if let Ok(mut h) = queries.p1().get_mut(*e) {
            let new_hp = (h.current as i32 + delta).max(0);
            println!(
                "Combat: {e} took {} dmg (hp {} -> {})",
                -delta, h.current, new_hp
            );
            h.current = new_hp as u32;
        }
    }

    // 4. Apply final positions to non-dead units via p2.
    for (e, pos) in &deltas.final_positions {
        if deltas.deaths.contains(e) {
            continue;
        }
        let start = start_positions.get(e).copied().unwrap_or(*pos);
        if *pos != start {
            println!("Moved: {e} {start:?} -> {pos:?}");
        } else if let Some(want) = intent.get(e).copied()
            && want != start
        {
            println!("Rolled back: {e} stayed at {start:?} (intended {want:?})");
        }
        if let Ok(mut p) = queries.p2().get_mut(*e) {
            *p = *pos;
        }
    }

    // Log deaths separately so they're easy to spot in the noise.
    for e in &deltas.deaths {
        println!("Died: {e}");
    }

    // 5. Consume MoveTo markers from every unit that had one.
    for e in had_move_to {
        commands.entity(e).remove::<MoveTo>();
    }
}

/// Despawn every Unit whose current HP is 0. Replicon replicates the despawn.
pub fn cleanup_dead_units(
    candidates: Query<(Entity, &Health), With<Unit>>,
    mut commands: Commands,
) {
    for (entity, hp) in &candidates {
        if hp.current == 0 {
            println!("Despawning dead unit {entity}");
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let deltas = resolve_movement_pure(snapshot);

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

    #[test]
    fn resolve_movement_simple_move_to_empty_tile() {
        use shared::components::*;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_systems(Update, super::resolve_movement);

        let warrior_id = UnitTypeId(0);
        let warrior_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("warrior".into(), warrior_id);
        registry.definitions.insert(warrior_id, warrior_def);
        app.insert_resource(registry);

        let unit = {
            let world = app.world_mut();
            let p = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            world
                .spawn((
                    Unit {
                        type_id: warrior_id,
                    },
                    HexPosition::new(0, 0),
                    Owner(p),
                    Health::full(10),
                    MoveTo {
                        pos: HexPosition::new(1, 0),
                    },
                ))
                .id()
        };

        app.update();

        let pos = app.world().get::<HexPosition>(unit).unwrap();
        assert_eq!(*pos, HexPosition::new(1, 0));
        assert!(app.world().get::<MoveTo>(unit).is_none(), "MoveTo consumed");
        let hp = app.world().get::<Health>(unit).unwrap();
        assert_eq!(hp.current, 10, "no combat → no damage");
    }

    #[test]
    fn resolve_movement_two_way_stalemate_rolls_back() {
        use shared::components::*;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_systems(Update, super::resolve_movement);

        let warrior_id = UnitTypeId(0);
        let warrior_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 4,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("warrior".into(), warrior_id);
        registry.definitions.insert(warrior_id, warrior_def);
        app.insert_resource(registry);

        let (a, b) = {
            let world = app.world_mut();
            let p1 = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let p2 = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            let a = world
                .spawn((
                    Unit {
                        type_id: warrior_id,
                    },
                    HexPosition::new(0, 0),
                    Owner(p1),
                    Health::full(10),
                    MoveTo {
                        pos: HexPosition::new(1, 0),
                    },
                ))
                .id();
            let b = world
                .spawn((
                    Unit {
                        type_id: warrior_id,
                    },
                    HexPosition::new(1, 0),
                    Owner(p2),
                    Health::full(10),
                ))
                .id();
            (a, b)
        };

        app.update();

        // A rolled back to start.
        assert_eq!(
            *app.world().get::<HexPosition>(a).unwrap(),
            HexPosition::new(0, 0)
        );
        // B stayed.
        assert_eq!(
            *app.world().get::<HexPosition>(b).unwrap(),
            HexPosition::new(1, 0)
        );
        // Both took 4 damage.
        assert_eq!(app.world().get::<Health>(a).unwrap().current, 6);
        assert_eq!(app.world().get::<Health>(b).unwrap().current, 6);
        // MoveTo on A consumed.
        assert!(app.world().get::<MoveTo>(a).is_none());
    }

    #[test]
    fn resolve_movement_move_into_enemy_kill_takes_tile() {
        use shared::components::*;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_systems(Update, super::resolve_movement);

        // Two unit types: a strong attacker (10 atk) and a weak defender (2 atk, 1 HP).
        let strong = UnitTypeId(0);
        let weak = UnitTypeId(1);
        let strong_def = UnitDefinition {
            hp: 10,
            move_budget: 2,
            attack_range: 1,
            attack_damage: 10,
            gold_upkeep: 1,
            production_cost: 10,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let weak_def = UnitDefinition {
            hp: 1,
            move_budget: 1,
            attack_range: 1,
            attack_damage: 2,
            gold_upkeep: 0,
            production_cost: 5,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("strong".into(), strong);
        registry.name_to_id.insert("weak".into(), weak);
        registry.definitions.insert(strong, strong_def);
        registry.definitions.insert(weak, weak_def);
        app.insert_resource(registry);

        let (attacker, victim) = {
            let world = app.world_mut();
            let p1 = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let p2 = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            let attacker = world
                .spawn((
                    Unit { type_id: strong },
                    HexPosition::new(0, 0),
                    Owner(p1),
                    Health::full(10),
                    MoveTo {
                        pos: HexPosition::new(1, 0),
                    },
                ))
                .id();
            let victim = world
                .spawn((
                    Unit { type_id: weak },
                    HexPosition::new(1, 0),
                    Owner(p2),
                    Health::full(1),
                ))
                .id();
            (attacker, victim)
        };

        app.update();

        // Attacker survived and took the tile.
        assert_eq!(
            *app.world().get::<HexPosition>(attacker).unwrap(),
            HexPosition::new(1, 0)
        );
        // Victim is dead (HP 0 — despawn happens in cleanup_dead_units, not here).
        let v_hp = app.world().get::<Health>(victim).unwrap().current;
        assert_eq!(v_hp, 0, "victim HP should be 0");
        // Attacker took 2 damage.
        assert_eq!(app.world().get::<Health>(attacker).unwrap().current, 8);
    }

    #[test]
    fn resolve_ranged_attacks_archer_hits_target_no_counter() {
        use shared::components::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_systems(Update, super::resolve_ranged_attacks);

        // Archer registry entry.
        let archer_id = UnitTypeId(0);
        let archer_def = UnitDefinition {
            hp: 8,
            move_budget: 2,
            attack_range: 2,
            attack_damage: 3,
            gold_upkeep: 1,
            production_cost: 25,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("archer".into(), archer_id);
        registry.definitions.insert(archer_id, archer_def);
        app.insert_resource(registry);

        let (attacker, target) = {
            let world = app.world_mut();
            let p1 = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let p2 = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            let attacker = world
                .spawn((
                    Unit { type_id: archer_id },
                    HexPosition::new(0, 0),
                    Owner(p1),
                    Health::full(8),
                    AttackTarget {
                        pos: HexPosition::new(2, 0),
                    },
                ))
                .id();
            let target = world
                .spawn((
                    Unit { type_id: archer_id },
                    HexPosition::new(2, 0),
                    Owner(p2),
                    Health::full(8),
                ))
                .id();
            (attacker, target)
        };

        app.update();

        // Target took 3 damage. No counter — attacker still at full HP.
        let target_hp = app.world().get::<Health>(target).unwrap();
        assert_eq!(target_hp.current, 5, "target should take attack_damage=3");
        let attacker_hp = app.world().get::<Health>(attacker).unwrap();
        assert_eq!(
            attacker_hp.current, 8,
            "attacker takes no counter from ranged"
        );
        // AttackTarget consumed.
        assert!(app.world().get::<AttackTarget>(attacker).is_none());
    }

    #[test]
    fn cleanup_dead_units_despawns_zero_hp() {
        use shared::unit_definition::UnitTypeId;
        use shared::units::*;

        let mut app = App::new();
        app.add_systems(Update, super::cleanup_dead_units);

        let (alive, dead) = {
            let world = app.world_mut();
            let alive = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(0, 0),
                    Health {
                        current: 5,
                        max: 10,
                    },
                ))
                .id();
            let dead = world
                .spawn((
                    Unit {
                        type_id: UnitTypeId(0),
                    },
                    HexPosition::new(1, 0),
                    Health {
                        current: 0,
                        max: 10,
                    },
                ))
                .id();
            (alive, dead)
        };

        app.update();

        assert!(app.world().entities().contains(alive), "alive unit kept");
        assert!(
            !app.world().entities().contains(dead),
            "HP=0 unit despawned"
        );
    }

    #[test]
    fn resolve_ranged_attacks_no_target_consumes_marker() {
        use shared::components::*;
        use shared::hex::HexPosition;
        use shared::unit_definition::*;
        use shared::units::*;
        use std::collections::HashMap;

        let mut app = App::new();
        app.add_systems(Update, super::resolve_ranged_attacks);

        let archer_id = UnitTypeId(0);
        let archer_def = UnitDefinition {
            hp: 8,
            move_budget: 2,
            attack_range: 2,
            attack_damage: 3,
            gold_upkeep: 1,
            production_cost: 25,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("archer".into(), archer_id);
        registry.definitions.insert(archer_id, archer_def);
        app.insert_resource(registry);

        let attacker = {
            let world = app.world_mut();
            let p = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            world
                .spawn((
                    Unit { type_id: archer_id },
                    HexPosition::new(0, 0),
                    Owner(p),
                    Health::full(8),
                    AttackTarget {
                        pos: HexPosition::new(2, 0),
                    }, // no one there
                ))
                .id()
        };

        app.update();

        assert!(
            app.world().get::<AttackTarget>(attacker).is_none(),
            "AttackTarget consumed even when target tile is empty"
        );
    }
}
