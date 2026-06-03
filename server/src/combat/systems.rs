use bevy::prelude::*;
use shared::cities::{City, CityOwner};
use shared::hex::HexPosition;
use shared::tiles::{OwnedTiles, TileOwner};
use shared::unit_definition::UnitRegistry;
use shared::units::{AttackTarget, ColorIndex, Health, MoveTo, Owner, Unit};
use std::collections::HashMap;

use crate::cities::{CITY_CAPTURE_HP, CityAttackedThisTurn};

use super::algorithm::resolve_movement_pure;
use super::types::{CitySnapshot, ResolveAction, UnitSnapshot};

/// Pre-movement phase: each ranged unit with an AttackTarget hits the live unit
/// at the target tile (if any) for `attack_damage`, with no counter and no movement.
/// Submit-time validation already guaranteed an enemy was at the tile at submit;
/// a missing target here means concentrated-fire killed it earlier in this loop,
/// or the marker survived a target despawn — either way the attack is wasted.
pub fn resolve_ranged_attacks(
    attackers: Query<(Entity, &Unit, &Owner, &AttackTarget)>,
    // Health excluded here to avoid conflicting borrows with hp_q below;
    // liveness is checked via hp_q after we find the candidate by position.
    unit_positions: Query<(Entity, &HexPosition), With<Unit>>,
    city_positions: Query<(Entity, &HexPosition, &CityOwner), With<City>>,
    registry: Res<UnitRegistry>,
    mut hp_q: Query<&mut Health>,
    mut commands: Commands,
) {
    let mut acts: Vec<_> = attackers.iter().collect();
    acts.sort_by_key(|(e, _, _, _)| *e);

    for (attacker_entity, unit, owner, attack_target) in acts {
        let Some(def) = registry.get(&unit.type_id) else {
            commands.entity(attacker_entity).remove::<AttackTarget>();
            continue;
        };

        // Find the live unit at the target tile.
        let target_entity = unit_positions
            .iter()
            .find(|(e, p)| {
                **p == attack_target.pos && hp_q.get(*e).map(|h| h.current > 0).unwrap_or(false)
            })
            .map(|(e, _)| e);

        if let Some(target_entity) = target_entity
            && let Ok(mut h) = hp_q.get_mut(target_entity)
        {
            let before = h.current;
            h.current = h.current.saturating_sub(def.attack_damage);
            println!(
                "Ranged: {attacker_entity} hit unit {target_entity} at {:?} for {} dmg (hp {} -> {})",
                attack_target.pos, def.attack_damage, before, h.current
            );
        } else if let Some((city_entity, _, _)) = city_positions
            .iter()
            .find(|(_, p, city_owner)| **p == attack_target.pos && city_owner.entity != owner.0)
        {
            if let Ok(mut h) = hp_q.get_mut(city_entity) {
                let before = h.current;
                h.current = h.current.saturating_sub(def.attack_damage);
                commands.entity(city_entity).insert(CityAttackedThisTurn);
                println!(
                    "Ranged: {attacker_entity} hit city {city_entity} at {:?} for {} dmg (hp {} -> {})",
                    attack_target.pos, def.attack_damage, before, h.current
                );
            }
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
/// snapshots cities, runs the algorithm, applies HP / ownership / position
/// deltas, and consumes resolved MoveTo markers.
///
/// Uses a `ParamSet` to avoid B0001: the read query (for snapshot building)
/// and the write queries (for applying deltas) share `Health` and `HexPosition`,
/// so Bevy requires them to be declared in a `ParamSet` to prove they are
/// used exclusively, not concurrently.
#[allow(clippy::type_complexity)] // ParamSet with several ECS queries; extracting a type alias gains little
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
        Query<&mut HexPosition, With<Unit>>,
        // p3: city snapshot query
        Query<(Entity, &CityOwner, &HexPosition, &Health), With<City>>,
        // p4: city color write-back
        Query<&mut ColorIndex, (With<City>, Without<Unit>)>,
        // p5: unit color lookup for captured city color
        Query<&ColorIndex, (With<Unit>, Without<City>)>,
        // p6: city-owned tile relationship lookup
        Query<&OwnedTiles, With<City>>,
    )>,
    tile_owners: Query<&TileOwner>,
    registry: Res<UnitRegistry>,
    mut commands: Commands,
) {
    // 1. Build immutable snapshots of live units and cities.
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
    let city_snapshot: Vec<CitySnapshot> = queries
        .p3()
        .iter()
        .map(|(entity, owner, pos, health)| CitySnapshot {
            entity,
            owner: owner.entity,
            hp: health.current as i32,
            max_hp: health.max,
            pos: *pos,
        })
        .collect();

    // Index unit snapshots for cheap lookup of pre-resolution state while logging.
    let snap_by_entity: HashMap<Entity, &UnitSnapshot> =
        snapshot.iter().map(|s| (s.entity, s)).collect();

    // 2. Run the pure resolver.
    let deltas = resolve_movement_pure(&snapshot, &city_snapshot, CITY_CAPTURE_HP);

    // 3. Apply unit/city HP changes and city ownership updates.
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
    for (e, delta) in &deltas.city_hp_changes {
        if let Ok(mut h) = queries.p1().get_mut(*e) {
            let new_hp = (h.current as i32 + delta).max(0);
            println!(
                "City combat: {e} hp {} -> {} (delta {})",
                h.current, new_hp, delta
            );
            h.current = new_hp as u32;
            commands.entity(*e).insert(CityAttackedThisTurn);
        }
    }

    for capture in &deltas.city_captures {
        println!(
            "City captured: {} by player {}",
            capture.city, capture.new_owner
        );
        commands.entity(capture.city).insert(CityOwner {
            entity: capture.new_owner,
        });

        if let Ok(unit_color) = queries.p5().get(capture.by_unit) {
            let new_color = unit_color.0;
            if let Ok(mut city_color) = queries.p4().get_mut(capture.city) {
                city_color.0 = new_color;
            }
        }

        if let Ok(owned_tiles) = queries.p6().get(capture.city) {
            for tile_entity in owned_tiles.collection() {
                let Ok(tile_owner) = tile_owners.get(*tile_entity) else {
                    continue;
                };
                commands.entity(*tile_entity).insert(TileOwner {
                    city_entity: tile_owner.city_entity,
                    player_entity: Some(capture.new_owner),
                });
            }
        }
    }

    // 4. Apply final positions to surviving units.
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

    // 5. Consume MoveTo markers from every live unit that had one.
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
    fn resolve_ranged_attacks_city_reaches_zero_without_capture() {
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

        let (attacker, city, defender) = {
            let world = app.world_mut();
            let attacker_owner = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let defender = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            let attacker = world
                .spawn((
                    Unit { type_id: archer_id },
                    HexPosition::new(0, 0),
                    Owner(attacker_owner),
                    Health::full(8),
                    AttackTarget {
                        pos: HexPosition::new(2, 0),
                    },
                ))
                .id();
            let city = world
                .spawn((
                    City,
                    HexPosition::new(2, 0),
                    CityOwner { entity: defender },
                    ColorIndex(1),
                    Health {
                        current: 3,
                        max: 20,
                    },
                ))
                .id();
            (attacker, city, defender)
        };

        app.update();

        assert_eq!(app.world().get::<Health>(city).unwrap().current, 0);
        assert_eq!(
            app.world().get::<CityOwner>(city).unwrap().entity,
            defender,
            "ranged attacks must not capture cities"
        );
        assert!(app.world().get::<AttackTarget>(attacker).is_none());
    }

    #[test]
    fn resolve_city_melee_attacks_captures_and_updates_owned_tiles() {
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

        let (city, attacker_owner, tile) = {
            let world = app.world_mut();
            let attacker_owner = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let defender_owner = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            world.spawn((
                Unit {
                    type_id: warrior_id,
                },
                HexPosition::new(0, 0),
                Owner(attacker_owner),
                ColorIndex(0),
                Health::full(10),
                MoveTo {
                    pos: HexPosition::new(1, 0),
                },
            ));
            let city = world
                .spawn((
                    City,
                    HexPosition::new(1, 0),
                    CityOwner {
                        entity: defender_owner,
                    },
                    ColorIndex(1),
                    Health {
                        current: 3,
                        max: 20,
                    },
                ))
                .id();
            let tile = world
                .spawn(TileOwner {
                    city_entity: city,
                    player_entity: Some(defender_owner),
                })
                .id();
            (city, attacker_owner, tile)
        };

        app.update();

        assert_eq!(
            app.world().get::<CityOwner>(city).unwrap().entity,
            attacker_owner
        );
        assert_eq!(app.world().get::<ColorIndex>(city).unwrap().0, 0);
        assert_eq!(app.world().get::<Health>(city).unwrap().current, 10);
        let tile_owner = app.world().get::<TileOwner>(tile).unwrap();
        assert_eq!(tile_owner.city_entity, city);
        assert_eq!(tile_owner.player_entity, Some(attacker_owner));
    }

    #[test]
    fn resolve_city_melee_attacks_captures_city_already_at_zero() {
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

        let (city, attacker_owner) = {
            let world = app.world_mut();
            let attacker_owner = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let defender_owner = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            world.spawn((
                Unit {
                    type_id: warrior_id,
                },
                HexPosition::new(0, 0),
                Owner(attacker_owner),
                ColorIndex(0),
                Health::full(10),
                MoveTo {
                    pos: HexPosition::new(1, 0),
                },
            ));
            let city = world
                .spawn((
                    City,
                    HexPosition::new(1, 0),
                    CityOwner {
                        entity: defender_owner,
                    },
                    ColorIndex(1),
                    Health {
                        current: 0,
                        max: 20,
                    },
                ))
                .id();
            (city, attacker_owner)
        };

        app.update();

        assert_eq!(
            app.world().get::<CityOwner>(city).unwrap().entity,
            attacker_owner
        );
        assert_eq!(app.world().get::<Health>(city).unwrap().current, 10);
    }

    #[test]
    fn resolve_city_melee_attacks_damages_without_capture_when_hp_remains() {
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

        let (city, defender_owner) = {
            let world = app.world_mut();
            let attacker_owner = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let defender_owner = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            world.spawn((
                Unit {
                    type_id: warrior_id,
                },
                HexPosition::new(0, 0),
                Owner(attacker_owner),
                ColorIndex(0),
                Health::full(10),
                MoveTo {
                    pos: HexPosition::new(1, 0),
                },
            ));
            let city = world
                .spawn((
                    City,
                    HexPosition::new(1, 0),
                    CityOwner {
                        entity: defender_owner,
                    },
                    ColorIndex(1),
                    Health {
                        current: 8,
                        max: 20,
                    },
                ))
                .id();
            (city, defender_owner)
        };

        app.update();

        assert_eq!(
            app.world().get::<CityOwner>(city).unwrap().entity,
            defender_owner
        );
        assert_eq!(app.world().get::<Health>(city).unwrap().current, 4);
    }

    #[test]
    fn resolve_city_melee_attacks_rolls_back_failed_city_entry() {
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

        let (attacker, city, defender_owner) = {
            let world = app.world_mut();
            let attacker_owner = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let defender_owner = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            let attacker = world
                .spawn((
                    Unit {
                        type_id: warrior_id,
                    },
                    HexPosition::new(0, 0),
                    Owner(attacker_owner),
                    ColorIndex(0),
                    Health::full(10),
                    MoveTo {
                        pos: HexPosition::new(1, 0),
                    },
                ))
                .id();
            let city = world
                .spawn((
                    City,
                    HexPosition::new(1, 0),
                    CityOwner {
                        entity: defender_owner,
                    },
                    ColorIndex(1),
                    Health {
                        current: 8,
                        max: 20,
                    },
                ))
                .id();
            (attacker, city, defender_owner)
        };

        app.update();

        assert_eq!(
            *app.world().get::<HexPosition>(attacker).unwrap(),
            HexPosition::new(0, 0),
            "failed city assault should leave attacker outside the city"
        );
        assert_eq!(
            app.world().get::<CityOwner>(city).unwrap().entity,
            defender_owner
        );
        assert_eq!(app.world().get::<Health>(city).unwrap().current, 4);
        assert!(app.world().get::<MoveTo>(attacker).is_none());
    }

    #[test]
    fn resolve_city_melee_attacks_ignores_ranged_unit_on_city() {
        let mut app = App::new();
        app.add_systems(Update, super::resolve_movement);

        let archer_id = UnitTypeId(0);
        let archer_def = UnitDefinition {
            hp: 8,
            move_budget: 2,
            attack_range: 2,
            attack_damage: 12,
            gold_upkeep: 1,
            production_cost: 25,
            build_targets: vec![],
            terrain_cost: HashMap::new(),
        };
        let mut registry = UnitRegistry::default();
        registry.name_to_id.insert("archer".into(), archer_id);
        registry.definitions.insert(archer_id, archer_def);
        app.insert_resource(registry);

        let (city, defender_owner) = {
            let world = app.world_mut();
            let attacker_owner = world
                .spawn(Player {
                    color_index: 0,
                    gold: 0,
                })
                .id();
            let defender_owner = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            world.spawn((
                Unit { type_id: archer_id },
                HexPosition::new(0, 0),
                Owner(attacker_owner),
                ColorIndex(0),
                Health::full(8),
                MoveTo {
                    pos: HexPosition::new(1, 0),
                },
            ));
            let city = world
                .spawn((
                    City,
                    HexPosition::new(1, 0),
                    CityOwner {
                        entity: defender_owner,
                    },
                    ColorIndex(1),
                    Health {
                        current: 5,
                        max: 20,
                    },
                ))
                .id();
            (city, defender_owner)
        };

        app.update();

        assert_eq!(
            app.world().get::<CityOwner>(city).unwrap().entity,
            defender_owner
        );
        assert_eq!(app.world().get::<Health>(city).unwrap().current, 5);
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
