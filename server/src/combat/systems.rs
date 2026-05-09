use bevy::prelude::*;
use shared::hex::HexPosition;
use shared::unit_definition::UnitRegistry;
use shared::units::{AttackTarget, Health, MoveTo, Owner, Unit};
use std::collections::HashMap;

use super::algorithm::resolve_movement_pure;
use super::types::{ResolveAction, UnitSnapshot};

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
    // 1. Build snapshot — immutable pass.
    let snapshot: Vec<UnitSnapshot> = queries
        .p0()
        .iter()
        .filter(|(_, _, _, h, _, _)| h.current > 0)
        .filter_map(|(e, owner, pos, h, unit, move_to)| {
            let def = registry.get(&unit.type_id)?;
            let action = match move_to {
                Some(m) => ResolveAction::MoveTo(m.pos),
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

    // Index for log-time lookup of a unit's pre-resolution state.
    let snap_by_entity: HashMap<Entity, &UnitSnapshot> =
        snapshot.iter().map(|s| (s.entity, s)).collect();

    // 2. Run pure algorithm.
    let deltas = resolve_movement_pure(&snapshot);

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
        if let Some(snap) = snap_by_entity.get(e) {
            let start = snap.start_pos;
            if *pos != start {
                println!("Moved: {e} {start:?} -> {pos:?}");
            } else if let ResolveAction::MoveTo(want) = snap.action
                && want != start
            {
                println!("Rolled back: {e} stayed at {start:?} (intended {want:?})");
            }
        }
        if let Ok(mut p) = queries.p2().get_mut(*e) {
            *p = *pos;
        }
    }

    // 5. Consume MoveTo markers from every unit that had one.
    // Death is logged once, by cleanup_dead_units in the next system.
    for snap in &snapshot {
        if matches!(snap.action, ResolveAction::MoveTo(_)) {
            commands.entity(snap.entity).remove::<MoveTo>();
        }
    }
}

/// Despawn every Unit whose current HP is 0. Replicon replicates the despawn.
pub fn cleanup_dead_units(
    candidates: Query<(Entity, &Health), With<Unit>>,
    mut commands: Commands,
) {
    for (entity, hp) in &candidates {
        if hp.current == 0 {
            println!("Despawn: {entity}");
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::components::*;
    use shared::unit_definition::*;
    use std::collections::HashMap;

    #[test]
    fn resolve_movement_simple_move_to_empty_tile() {
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
