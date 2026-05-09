# Combat System — Design Spec

## Goal

Replace the stubbed `resolve_attacks` / `resolve_moves` (which currently teleport without collision detection) with a combat resolution pipeline that satisfies two new constraints:

1. **No two units may share a tile.** When units of different players would land on the same tile, they fight. Survivors take the tile; non-survivors die or rollback.
2. **Combat plays out across game turns** as a single damage exchange per turn. The unit with the lowest HP eventually loses the attrition war.

The design is deliberately small: melee combat is just "Move into the enemy". The Attack verb exists only on ranged units. Fortify and settler-capture are tracked as follow-up specs to keep this spec focused.

## Locked Decisions

| Decision | Choice |
|----------|--------|
| Combat timing | Single damage exchange per turn; combat plays out across game turns |
| Multi-unit collisions | All-vs-all simultaneous: each unit takes the sum of all other attackers' `attack_damage` in one round |
| Rollback | All survivors of a contested-tile fight rollback to their turn-start position; the home unit's "rollback" is a no-op (it was already there) |
| Rollback chain | Recursive: rollbacks may create new conflicts that trigger more combat in the same resolution; converges because turn-start positions are unique |
| Friendly conflicts | Prevented upfront — `handle_unit_action` rejects Move targeting any tile where another friendly unit currently is OR has already queued a Move |
| Melee combat | Move-into-enemy is the only melee path. Combat triggers iff two opposing units end up on the same tile after movement is computed. |
| Attack verb scope | Only ranged units (`attack_range > 1`) get the Attack verb. Melee units only have Move. |
| Ranged Attack | Pre-movement phase: damage applied to whoever is at the target tile right now. No counter. Attacker does not move. |
| Damage formula | Flat `attack_damage`; no HP scaling. No defender modifier in this spec. |
| Settlers | Die normally (placeholder until settler-capture spec) |
| Fortify | Out of scope. `resolve_fortify` stays a stub. Fortified state and bonus are introduced by a follow-up spec. |

## Architectural Overview

Combat resolution is split into a small set of chained server systems inside the existing `turn_is_resolving` window. Each system has one responsibility. The non-trivial logic — the iterating conflict-and-rollback algorithm — lives in a pure function `resolve_movement_pure(snapshot) -> Deltas` so it can be unit-tested without a Bevy harness.

### Resolution pipeline

The new chain inside `server/src/main.rs` (additions and edits relative to the existing chain):

```
grow_city_population
grant_city_gold
resolve_ranged_attacks        [NEW]      Only ranged Attack actions (attack_range > 1).
                                         Pre-movement: damage applied to whoever is
                                         at the target tile. No counter.
resolve_movement              [NEW]      All Move actions. Combat triggers when two
                                         opposing units end up on the same tile.
                                         Iterating conflict -> combat -> rollback chain.
cleanup_dead_units            [NEW]      Despawn units with HP = 0
resolve_fortify               unchanged stub (Fortify spec is its own follow-up)
resolve_skip                  unchanged stub (passive-heal future spec)
resolve_builds
advance_city_production
advance_turn
```

Removals: `resolve_moves` and the existing stub of `resolve_attacks` in `server/src/turn.rs`. Their work is absorbed by `resolve_movement` and `resolve_ranged_attacks`.

### Why this order

- `resolve_ranged_attacks` runs before `resolve_movement` so ranged attacks land at pre-movement positions — "I shot you before you ran". A target that intends to move still takes the hit.
- `resolve_movement` is where all melee combat happens, as a side-effect of moving onto an enemy-occupied tile.
- `cleanup_dead_units` runs after combat so HP=0 units linger long enough to be observed by the resolvers, then despawn before the rest of the turn-end work.

### Verb availability

`available_verbs` in `shared/src/unit_definition.rs` changes its Attack gate from `attack_damage > 0` to `attack_range > 1`. Melee units (warrior, cavalry, knight) no longer see an Attack button — they engage by Moving into enemies. Ranged units (archer) keep the Attack verb for distance shots.

## Data Shapes

### Snapshot and delta types (`server/src/combat.rs`, new file)

```rust
/// One row per live unit, gathered by the wrapper system before calling the algorithm.
pub struct UnitSnapshot {
    pub entity: Entity,
    pub owner: Entity,           // player entity
    pub hp: i32,                 // signed so subtraction can't underflow
    pub max_hp: u32,
    pub attack_damage: u32,
    pub attack_range: u32,
    pub start_pos: HexPosition,  // turn-start position
    pub action: ResolveAction,
}

pub enum ResolveAction {
    /// No movement this turn (Stationary, Fortify, Skip, Build, or just inactive).
    Stationary,
    /// Move to this destination. Triggers melee combat if an enemy ends up there too.
    MoveTo(HexPosition),
}

/// What the algorithm decides; applied to ECS by the wrapper.
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,         // signed delta to apply to current HP
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}
```

The pure algorithm `resolve_movement_pure(units: Vec<UnitSnapshot>) -> CombatDeltas` is a free function with no Bevy types. Tests construct snapshots by hand and assert on deltas.

No new shared component is added in this spec. `Health` (already replicated) carries combat damage; despawn handles death.

## The Two Algorithms

### `resolve_ranged_attacks` — ranged-only, deterministic order

```rust
// Pseudocode. Real code uses Bevy queries.
let mut attackers: Vec<_> = collect_attackers_with_attack_target();
attackers.sort_by_key(|a| a.entity);   // determinism

for attacker in attackers {
    let def = registry.get(attacker.unit.type_id);
    debug_assert!(def.attack_range > 1, "melee units don't have AttackTarget");

    let target = live_unit_at(attacker.attack_target.pos);
    let Some(target) = target else {
        consume_attack_target(attacker.entity);
        continue;  // Concentrated-fire: target died from an earlier attack in this loop.
    };

    target.hp -= def.attack_damage;          // no counter for ranged
    consume_attack_target(attacker.entity);
}
```

Submit-time validation already rejected Attack actions with no enemy at the target, so we never see a wholly-empty target tile here unless something was killed mid-loop.

### `resolve_movement_pure` — the iterating conflict-and-rollback algorithm

```rust
fn resolve_movement_pure(units: Vec<UnitSnapshot>) -> CombatDeltas {
    let mut positions: HashMap<Entity, HexPosition> = HashMap::new();
    let mut hps:       HashMap<Entity, i32>         = HashMap::new();
    let mut deaths:    HashSet<Entity>              = HashSet::new();

    // Initialize: every live unit starts at its desired position.
    for u in &units {
        if u.hp <= 0 { continue; }
        let desired = match u.action {
            ResolveAction::Stationary  => u.start_pos,
            ResolveAction::MoveTo(t)   => t,
        };
        positions.insert(u.entity, desired);
        hps.insert(u.entity, u.hp);
    }

    let mut iter_count = 0;
    loop {
        iter_count += 1;
        assert!(iter_count < 256, "rollback chain failed to terminate");

        let Some(tile) = first_tile_with_multiple_live_units(&positions, &deaths) else {
            break;
        };
        let combatants = live_units_at(tile, &positions, &deaths, &units);

        // All-vs-all simultaneous melee damage exchange.
        // Each unit takes the sum of every other combatant's attack_damage.
        for u in &combatants {
            let raw: u32 = combatants.iter()
                .filter(|v| v.entity != u.entity)
                .map(|v| v.attack_damage)
                .sum();
            *hps.get_mut(&u.entity).unwrap() -= raw as i32;
        }

        // Apply this round's deaths.
        for u in &combatants {
            if hps[&u.entity] <= 0 {
                deaths.insert(u.entity);
            }
        }

        let survivors: Vec<_> = combatants.iter()
            .filter(|u| !deaths.contains(&u.entity))
            .collect();

        if survivors.len() > 1 {
            // 2+ alive — all rollback to start_pos.
            // Home unit (start_pos == tile) stays automatically (no-op rollback).
            for s in &survivors {
                positions.insert(s.entity, s.start_pos);
            }
        }
        // 0 or 1 alive — tile settled. Sole survivor (if any) is already at
        // `tile` in `positions`. Dead units stay tagged in `deaths`; they're
        // ignored by future conflict detection.
    }

    CombatDeltas {
        hp_changes: hp_deltas_against_initial(&hps, &units),
        final_positions: positions,
        deaths,
    }
}
```

#### Termination

`start_pos` is unique per unit (no two units shared a tile at turn start; friendly stacking is rejected upfront). Once a unit rolls back, its position is `start_pos`. New conflicts can only form at start positions where another unit moved in. Each chain link resolves one conflict; chain length is bounded by the number of involved units. The 256 hard cap is a sanity guard — should never trip in normal play.

### Worked example — chain combat

Setup: A@T1, B@T2, C@T3, all warriors (`hp=10`, `attack_damage=4`).
Actions: A queues Move to T2; C queues Move to T1; B is stationary.

| Iter | Conflict | Combatants | Damage | HP after iter | Survivors | Action |
|------|----------|------------|--------|---------------|-----------|--------|
| 1 | T2 | A, B | each takes 4 | A=6, B=6, C=10 | 2 | A → T1 (rollback), B → T2 (no-op) |
| 2 | T1 | A (6), C (10) | A takes 4, C takes 4 | A=2, B=6, C=6 | 2 | A → T1 (no-op), C → T3 (rollback) |
| 3 | none | — | — | — | — | terminate |

Final: A@T1 (2/10), B@T2 (6/10), C@T3 (6/10). A engaged in two combats and took two rounds of damage; B and C each took one.

## ECS Wrapper Systems

### `resolve_ranged_attacks`

Sketch (final query shape settled during implementation):

```rust
pub fn resolve_ranged_attacks(
    attackers: Query<(Entity, &Unit, &AttackTarget)>,
    targets: Query<(Entity, &HexPosition, &mut Health), With<Unit>>,
    registry: Res<UnitRegistry>,
    mut commands: Commands,
) { ... }
```

Sort attackers by entity id, find the live unit at each `AttackTarget.pos`, subtract the attacker's `attack_damage` from its `Health`, remove the `AttackTarget` marker. No counter, no movement. A `debug_assert` that `attack_range > 1` documents the invariant that melee units never reach this code path (because they don't have the Attack verb).

### `resolve_movement`

The wrapper for the pure algorithm. Sketch:

```rust
pub fn resolve_movement(/* queries omitted */) {
    // 1. Build snapshot from live units. start_pos = current HexPosition (the
    //    turn-start position; no movement has been applied yet this turn).
    //    Action = MoveTo(target) if MoveTo present, else Stationary.
    let snapshot = build_snapshot(/* queries */);

    // 2. Run the pure algorithm.
    let deltas = resolve_movement_pure(snapshot);

    // 3. Apply HP changes (saturating subtract).
    // 4. Apply final positions to non-dead units.
    // 5. Strip MoveTo markers (consumed).
}
```

### `cleanup_dead_units`

```rust
pub fn cleanup_dead_units(
    candidates: Query<(Entity, &Health), With<Unit>>,
    mut commands: Commands,
) {
    for (entity, hp) in &candidates {
        if hp.current == 0 {
            commands.entity(entity).despawn();
        }
    }
}
```

Replicon replicates the despawn to clients automatically.

### Server validation tweak

`handle_unit_action` in `server/src/turn.rs` gains a friendly-stacking rejection for Move. The check looks at both currently-occupied tiles AND already-queued moves:

```rust
UnitAction::Move { target } => {
    // existing range and bounds checks ...

    // Reject if any friendly is already at the target tile.
    if friendly_units.iter().any(|(pos, owner)| pos == target && owner.0 == *player_entity) {
        return;
    }

    // Reject if any friendly already queued a Move to the target tile.
    if friendly_movers.iter().any(|(move_to, owner)| move_to.pos == *target && owner.0 == *player_entity) {
        return;
    }

    queue_marker(&mut commands, entity, MoveTo { pos: *target });
}
```

The two checks together guarantee no two friendly units ever end up on the same tile. Combined with the `start_pos` uniqueness invariant, the algorithm never sees friendly conflicts.

### Verb availability change

In `shared/src/unit_definition.rs`:

```rust
pub fn available_verbs(def: &UnitDefinition) -> Vec<UnitVerb> {
    let mut v = vec![UnitVerb::Move, UnitVerb::Fortify, UnitVerb::Skip];
    if def.attack_range > 1 {                         // changed from attack_damage > 0
        v.push(UnitVerb::Attack);
    }
    if !def.build_targets.is_empty() {
        v.push(UnitVerb::Build);
    }
    v
}
```

Side effect: warrior, cavalry, and knight no longer see Attack. Archer still does. Settler (no `attack_range > 1`) doesn't show Attack — same as before.

## Edge Cases

| # | Case | Resolution |
|---|------|-----------|
| 1 | Ranged target moves away mid-turn | Attack hits at pre-movement position; target takes damage even though it then moves. |
| 2 | Unit dies from ranged Attack before its Move | Snapshot building filters HP > 0; dead unit's MoveTo is silently ignored. |
| 3 | Two units swap tiles | After applying intents, no conflicts; both succeed. Traversal collisions are not modeled (documented limitation). |
| 4 | Ranged unit Moves into enemy | Move-into-enemy is melee for everyone; the ranged unit takes counter damage just like any other unit. |
| 5 | Move-into-enemy that "misses" | Mover queued Move to T_B; defender queued Move elsewhere; defender vacates; mover walks into the empty tile, no combat. The contingent nature of melee is intentional. |
| 6 | Three friendlies rolling back | Friendly stacking impossible by construction (start_pos uniqueness + the two upfront move rejections). |
| 7 | Ranged target tile becomes empty mid-loop | Concentrated-fire: an earlier attacker killed the target; later attackers find no live unit; their attack is wasted. Marker still consumed. |
| 8 | All combatants die same iteration | Tile becomes empty; survivors set is empty; nothing to rollback. |
| 9 | Iteration cap (256) hit in `resolve_movement_pure` | `panic!` — server bug; should never trip in normal play. |
| 10 | Mutual ranged Attacks | A and B both ranged-Attack each other. No counter on either side. Each takes `other.attack_damage` once. Symmetric, single damage. |
| 11 | Settler attacked | Settler has `attack_damage = 0`. Takes damage normally; if in melee combat its contribution to all-vs-all damage is 0. Dies at HP = 0. Placeholder until settler-capture spec. |

## Testing

### Pure-function unit tests (no Bevy harness)

In `server/src/combat.rs`, against `resolve_movement_pure`:
- Single mover, no conflict — takes destination.
- 2-way conflict, both alive — each rolls back to start_pos.
- 2-way conflict, one dies — survivor takes the tile.
- 2-way conflict, both die — tile empty.
- 3-way all-vs-all damage sums.
- 3-way one survivor — sole survivor takes the tile.
- Chain combat (the worked example): A@T1→T2, B@T2 stays, C@T3→T1 → final positions match the table.
- Move-into-enemy that "misses": defender vacated, mover takes empty tile freely.

### Server-side ECS integration tests (extend `server/src/turn.rs` patterns)

- `resolve_ranged_attacks`: archer hits at distance 2, no counter, AttackTarget consumed.
- `resolve_movement`: unit moves to empty tile.
- `resolve_movement`: swap succeeds, no combat.
- `resolve_movement`: Move-into-enemy stalemate, mover rolls back.
- `resolve_movement`: Move-into-enemy kill, mover takes the tile.
- `cleanup_dead_units`: HP=0 unit despawned.
- `handle_unit_action` rejects Move into friendly-occupied tile.
- `handle_unit_action` rejects Move into a tile already targeted by another friendly's queued Move.
- `available_verbs` for melee units no longer includes Attack; archer still has it.
- Existing regression test `test_rejected_action_preserves_prior_marker` still passes.

### What we don't test

- Multi-turn HP persistence is implicitly covered by integration tests (Health is a normal component).
- Network replication relies on replicon plumbing; manual two-client verify when the implementation lands.

## Modules Touched

| File | Change |
|------|--------|
| `shared/src/unit_definition.rs` | Change `available_verbs` Attack gate from `attack_damage > 0` to `attack_range > 1`. Update tests accordingly. |
| `server/src/combat.rs` | New file: `UnitSnapshot`, `ResolveAction`, `CombatDeltas`, `resolve_movement_pure`, plus the wrapper systems `resolve_ranged_attacks`, `resolve_movement`, `cleanup_dead_units`. |
| `server/src/turn.rs` | Delete the `resolve_moves` and `resolve_attacks` stubs. Tighten `handle_unit_action` to reject Moves into friendly-occupied tiles AND tiles already targeted by another friendly's queued Move. `resolve_fortify`, `resolve_skip`, `handle_finish_turn`, `update_turn_phase`, `advance_turn` unchanged. |
| `server/src/main.rs` | Add `mod combat;`. Update the system chain in the resolution window per the pipeline above. |

## Out of Scope (separate specs)

- **Fortify mechanic** — the persistent `Fortified` state with a defense bonus and the rules around when it's lost. The Fortify verb still queues its `Fortifying` marker (per the unit-action-menu spec); `resolve_fortify` stays a stub until that spec lands.
- **Settler capture mechanic** — settler currently dies normally; capture, friendly-stacking with one military unit, and change-of-owner are deferred.
- **City vs unit combat** — capturing cities, city HP, population loss during attacks.
- **Passive heal on Skip** — heal-by-skip in friendly territory; mentioned in the unit-action-menu spec.
- **Strategic resources** (horses, iron) gating cavalry/knight production.
- **Combat visuals** — floating damage numbers, death animations, attack cues.

## Future Extensions

- **Fortify** — the immediate next combat spec. Adds defense bonus and the Fortified marker; integrates as a defender modifier in the damage calc.
- **Defense modifiers from terrain** (forest/hill bonus) — extends the damage calc with a terrain factor.
- **HP-scaled damage** — if attrition feels too predictable, switch from flat to HP-scaled. Single localised change in the damage formula.
- **Promotion/XP system** — surviving combat could grant XP; modifier on `attack_damage`.
- **Catapult / siege weapons** — area damage or city-only damage modifiers.
