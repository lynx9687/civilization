# Combat System — Design Spec

## Goal

Replace the stubbed `resolve_attacks` / `resolve_fortify` / `resolve_moves` (which currently teleport without collision detection) with a full combat resolution pipeline. Two new constraints govern the design:

1. **No two units may share a tile.** When units of different players would land on the same tile, they fight. Survivors take the tile; non-survivors die or rollback.
2. **Combat plays out across game turns** as a single damage exchange per turn. The unit with the lowest HP eventually loses the attrition war.

This spec covers unit-vs-unit combat (melee + ranged), movement conflicts, the rollback chain when stalemates push movers home, and the persistent `Fortified` state. Settler-capture and city combat are out of scope and tracked as follow-up specs.

## Locked Decisions

| Decision | Choice |
|----------|--------|
| Combat timing | Single damage exchange per turn; combat plays out across game turns |
| Multi-unit collisions | All-vs-all simultaneous: each unit takes the sum of all other attackers' damage in one round |
| Rollback | All survivors of a contested-tile fight rollback to their turn-start position; the home unit's "rollback" is a no-op |
| Rollback chain | Recursive: rollbacks may create new conflicts that trigger more combat in the same resolution; converges because turn-start positions are unique |
| Friendly conflicts | Prevented upfront — `handle_unit_action` rejects Move targeting a tile occupied by another friendly unit |
| Attack vs Move-into-enemy | Distinct. Attack lands damage in a pre-movement phase (target cannot dodge by moving). Move-into-enemy is contingent — combat only triggers if defender is still on the destination at movement phase. |
| Melee Attack outcome | Attacker stays put always; advancement is exclusively via Move-into-enemy on a sole-survivor outcome. |
| Ranged classification | A unit is ranged iff `attack_range > 1`. Ranged Attack: no counter at any distance. Move-into-enemy by anyone (including a ranged unit): melee, with counter. |
| Damage formula | Flat `attack_damage`; no HP scaling |
| Fortified bonus | 50% damage reduction (floor); applies to a unit iff it has the `Fortified` marker AND its action this turn is None or Fortify; lost on Move/Attack/Skip/Build |
| Settlers | Die normally for now (placeholder until settler-capture spec) |

## Architectural Overview

Combat resolution is split into a small set of chained server systems within the existing `turn_is_resolving` window. Each system has one responsibility. The non-trivial logic — the iterating conflict-and-rollback algorithm — lives in a pure function `resolve_movement(snapshot) -> Deltas` so it can be unit-tested without a Bevy harness.

### Resolution pipeline

The new chain inside `server/src/main.rs` (additions and edits relative to the existing chain):

```
grow_city_population
grant_city_gold
apply_fortifying              [NEW]      Fortifying marker -> Fortified marker
resolve_attacks               [NEW]      All Attack actions (ranged + melee).
                                         Pre-movement: damage applied to target;
                                         counter applies iff attacker is melee.
                                         Attacker does not advance.
                                         Strips Fortified from each attacker.
resolve_movement              [NEW]      Move actions only. Combat is contingent on
                                         the defender still being on the destination.
                                         Iterating conflict -> combat -> rollback chain.
                                         Strips Fortified from each unit that had MoveTo.
cleanup_dead_units            [NEW]      Despawn units with HP = 0
resolve_skip                  unchanged stub (passive-heal future spec).
                                         Strips Fortified from each unit that had Skipping.
resolve_builds                Strips Fortified from each unit that had BuildProject.
advance_city_production
advance_turn
```

Removals: `resolve_moves`, `resolve_attacks` (the stubs in `server/src/turn.rs`), and `resolve_fortify`. Their work is absorbed by the systems above. Each resolver strips Fortified from the units it processed — embedding the strip per-resolver is simpler than a separate central system.

### Why this order

- `apply_fortifying` first so this turn's submitted Fortify is visible during combat.
- `resolve_attacks` before `resolve_movement` so explicit Attacks land at pre-movement positions — targets cannot dodge by moving.
- Ranged Attack hitting a tile that the target then vacates is intuitive: "I shot you before you ran."
- `cleanup_dead_units` runs after combat so HP=0 units linger long enough to be observed by the resolvers, then despawn before the rest of the turn-end work.

## Data Shapes

### New component (`shared/src/units.rs`)

```rust
/// Persistent defensive stance. Set by apply_fortifying when a Fortifying marker
/// resolves; removed by the resolver of any non-Fortify action (Move, Attack,
/// Skip, Build). Replicated so the client can show a fortify badge.
#[derive(Component, Serialize, Deserialize, Debug)]
#[require(Replicated)]
pub struct Fortified;
```

Combat damage is just `Health` mutation (already exists, already replicated). Death is despawn (replicated as removal).

### Plugin registration (`shared/src/plugin.rs`)

```rust
.replicate::<Fortified>()
```

### Shared pure helpers (`shared/src/combat.rs`, new file)

```rust
/// Flat damage from a single attacker. Settler with attack_damage = 0 contributes nothing.
pub fn damage_dealt(attacker: &UnitDefinition) -> u32 {
    attacker.attack_damage
}

/// Defender's Fortified bonus halves incoming damage (floor).
pub fn damage_taken(raw: u32, defender_fortified: bool) -> u32 {
    if defender_fortified { raw / 2 } else { raw }
}

/// Counter is dealt by the defender to the attacker iff the engagement is melee.
/// Move-into-enemy is melee for everyone; explicit Attack is melee iff the
/// attacker has attack_range == 1.
pub fn engagement_is_melee(attacker_def: &UnitDefinition, used_move_into_enemy: bool) -> bool {
    used_move_into_enemy || attacker_def.attack_range == 1
}
```

These tiny pure functions live in `shared` so the client can later show damage previews from the same source of truth.

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
    /// Eligible for the 50% Fortified defender bonus iff:
    ///   Fortified marker present (resolve_attacks already stripped it from attackers)
    ///   AND none of the {Skipping, BuildProject} markers are queued.
    /// (MoveTo presence is captured separately via `action`.)
    pub fortified_defender: bool,
    pub start_pos: HexPosition,  // turn-start position
    pub action: ResolveAction,
}

pub enum ResolveAction {
    /// No movement intent: unit stays at start_pos this turn.
    Stationary,
    /// Unit wants to be at this destination (Move action).
    /// Melee Attack is NOT here — already resolved in the prior phase.
    MoveTo(HexPosition),
}

/// What the algorithm decides; applied to ECS by the wrapper.
pub struct CombatDeltas {
    pub hp_changes: HashMap<Entity, i32>,        // signed delta to apply to current HP
    pub final_positions: HashMap<Entity, HexPosition>,
    pub deaths: HashSet<Entity>,
}
```

The `resolve_movement` algorithm consumes `Vec<UnitSnapshot>` and returns `CombatDeltas`. The wrapper system applies the deltas to ECS components.

## The Two Algorithms

### `resolve_attacks` — pre-movement, deterministic order

```
sort attackers by entity id   // determinism
for attacker in attackers:
    consume Fortified marker on attacker  // Attack always strips Fortified
    let def = registry.get(attacker.unit.type_id)
    let target = the LIVE unit at AttackTarget.pos right now (if any)
    if target is None:
        consume AttackTarget marker, continue
        // Target may have died from an earlier attack in this same loop
        // (concentrated-fire case). Submit-time validation already ensured
        // a target was there at submit; nothing else lets the tile go empty.
    let target_is_defender = target.has_Fortified
                          && target has none of {MoveTo, AttackTarget, Skipping, BuildProject}
    target.hp -= damage_taken(def.attack_damage, target_is_defender)
    if def.attack_range == 1:           // melee: counter
        // Attacker took an action (Attack), so no Fortified bonus on counter.
        attacker.hp -= damage_taken(target.attack_damage, false)
    consume AttackTarget marker
```

A unit can be killed by an earlier attacker in the loop, in which case later attackers find no live unit at the target tile and waste their action. This is acceptable: attacks were submitted assuming a target was there; attrition between simultaneous attackers is expected.

Note on the defender check: `target has none of {MoveTo, AttackTarget, Skipping, BuildProject}` correctly handles all "lost on action" cases. `Fortifying` (the queued marker) is already gone by this phase — it was consumed by `apply_fortifying` and turned into the persistent `Fortified` marker. `AttackTarget` for an attacker we already processed in this loop has been stripped, so a unit that already attacked is correctly seen as "having acted" via its now-missing Fortified (we strip Fortified at the top of each iteration).

### `resolve_movement` — the iterating conflict-and-rollback algorithm

```
fn resolve_movement(units: Vec<UnitSnapshot>) -> CombatDeltas:
    // Working state, mutable.
    positions: map<Entity, HexPosition>
    hps:       map<Entity, i32>
    deaths:    set<Entity>

    // Initialize: every unit starts at its desired position.
    for u in units where u.hp > 0:
        positions[u.entity] = match u.action:
            Stationary  => u.start_pos
            MoveTo(t)   => t
        hps[u.entity] = u.hp

    loop:
        T = any tile with 2+ live units in `positions`
        if T is None: break

        combatants = live units at T

        // All-vs-all simultaneous melee damage exchange.
        // Every unit at T deals attack_damage to every other unit at T.
        for u in combatants:
            raw = sum of v.attack_damage for v in combatants where v != u
            is_defender = u.fortified_defender
                       && u.action == Stationary
                       && u.start_pos == T   // didn't move into T; was already here
            taken = damage_taken(raw, is_defender)
            hps[u.entity] -= taken

        // Apply this round's deaths.
        for u in combatants:
            if hps[u.entity] <= 0:
                deaths.insert(u.entity)

        survivors = combatants \ deaths

        if survivors.len() <= 1:
            // 0 or 1 alive — tile settled.
            // Sole survivor (if any) is already at T in `positions`. No change.
            // Dead units stay tagged in `deaths`; they'll be ignored by future
            // conflict detection (live units only).
        else:
            // 2+ alive — all rollback to start_pos.
            // Home unit (start_pos == T) effectively stays.
            for s in survivors:
                positions[s.entity] = s.start_pos

    return CombatDeltas {
        hp_changes: per-entity (final_hp - initial_hp),
        final_positions: positions,
        deaths,
    }
```

#### Termination

`start_pos` is unique per unit (no two units shared a tile at turn start). After at most one rollback per chain step, all rolling-back units are at their unique `start_pos`. New conflicts can only form at start positions where another unit already moved in. Each chain link resolves one conflict; chain length is bounded by the number of involved units. We add a hard cap of 256 iterations (`panic!` if exceeded — should be unreachable; trips only on a bug).

### Worked example — chain combat

Setup: A@T1, B@T2, C@T3, all warriors (`hp=10`, `attack_damage=4`).
Actions: A queues Move to T2; C queues Move to T1; B is stationary.

| Iter | Conflict | Combatants | Damage | HP after iter | Survivors | Action |
|------|----------|------------|--------|---------------|-----------|--------|
| 1 | T2 | A, B | each takes 4 | A=6, B=6, C=10 | 2 | A → T1, B → T2 (no-op) |
| 2 | T1 | A (6), C (10) | A takes 4, C takes 4 | A=2, B=6, C=6 | 2 | A → T1 (no-op), C → T3 |
| 3 | none | — | — | — | — | terminate |

Final: A@T1 (2/10), B@T2 (6/10), C@T3 (6/10). A engaged in two combats and took two rounds of damage; B and C each took one.

## ECS Wrapper Systems

### `apply_fortifying`

```rust
pub fn apply_fortifying(
    units: Query<Entity, With<Fortifying>>,
    mut commands: Commands,
) {
    for entity in &units {
        commands.entity(entity)
            .remove::<Fortifying>()
            .insert(Fortified);
    }
}
```

Idempotent. A unit that re-fortifies just keeps the marker.

### `resolve_attacks`

Sketch (final query shape settled during implementation):

```rust
pub fn resolve_attacks(
    attackers: Query<(Entity, &Unit, &AttackTarget)>,
    targets: Query<(Entity, &HexPosition, &Unit, Has<Fortified> /* + queued-action markers */), With<Unit>>,
    registry: Res<UnitRegistry>,
    mut hp_q: Query<&mut Health>,
    mut commands: Commands,
) { ... }
```

Sort attackers by entity id; for each, look up the live target at `AttackTarget.pos`, compute primary and counter damages with the helpers, mutate `Health`, and remove the `AttackTarget` marker. The "is the target a stationary defender" check inspects the target's queued markers: a target with no `MoveTo` and no `AttackTarget` qualifies as stationary; combined with the `Fortified` marker it gets the bonus.

### `resolve_movement`

The wrapper for the pure algorithm. Sketch:

```rust
pub fn resolve_movement(/* queries omitted */) {
    // 1. Build snapshot from live units — current HexPosition is start_pos.
    //    Action = MoveTo(target) if MoveTo present, else Stationary.
    let snapshot = build_snapshot(/* queries */);

    // 2. Run the pure algorithm.
    let deltas = resolve_movement_pure(snapshot);

    // 3. Apply HP changes (saturating subtract).
    // 4. Apply final positions to non-dead units.
    // 5. Strip MoveTo markers (consumed).
    //    AttackTarget was already consumed by resolve_attacks.
}
```

`build_snapshot` translates each unit's queued markers into `ResolveAction::MoveTo(t)` if `MoveTo` is present, else `ResolveAction::Stationary`. Only `MoveTo` indicates movement intent at this phase — `AttackTarget` was already consumed by `resolve_attacks`.

### Fortified-stripping inside each resolver

Fortified-marker removal is embedded in each resolver rather than centralized in a separate system:
- `resolve_attacks` strips Fortified at the top of each attacker's iteration.
- `resolve_movement` strips Fortified after applying the move, for every unit whose snapshot had `MoveTo`.
- `resolve_skip` strips Fortified for every unit with the `Skipping` marker.
- `resolve_builds` strips Fortified for every unit with the `BuildProject` marker.

This avoids a separate cross-cutting system and matches the locality principle: the resolver that consumed the queued action is the one that knows the unit acted.

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

The two checks together prevent any path to friendly stacking at resolution: a friendly never arrives at a tile occupied by another friendly's start position (first check), and two friendlies can't both queue Moves to the same empty tile (second check). With both, the algorithm's `start_pos` uniqueness guarantee extends to "no two friendly units at the same final position either."

## Edge Cases

| # | Case | Resolution |
|---|------|-----------|
| 1 | Ranged target moves away mid-turn | Attack hits at pre-movement position; target takes damage even though it then moves. |
| 2 | Unit dies from Attack before its Move | Snapshot building filters HP > 0; dead unit's MoveTo is silently ignored. |
| 3 | Move + Attack same turn | Single-action invariant from unit-action-menu spec prevents this. |
| 4 | Two units swap tiles | After applying intents, no conflicts; both succeed. Traversal collisions are not modeled. |
| 5 | Ranged unit Moves into enemy | Move-into-enemy is melee for everyone, including ranged; counter applies. |
| 6 | Fortified unit Attacks | `is_defender = false` because action is Attack; no bonus on counter taken. Marker stripped at end. |
| 7 | Fortified stationary unit caught by both Attack AND a mover | Step 2: defender takes 50% from Attack. Step 3: same defender (still Fortified, still stationary) takes 50% in melee combat. Bonus persists across phases. |
| 8 | Three friendlies rolling back | Friendly stacking impossible by construction (start_pos uniqueness + upfront move rejection). |
| 9 | Fortified unit submits Move | `is_defender = false` because action is Move; no bonus this turn. Marker stripped. |
| 10 | Ranged target tile becomes empty | Wasted attack, no-op. Marker consumed. |
| 11 | All combatants die same iteration | Tile becomes empty; survivors set is empty; nothing to rollback. |
| 12 | Iteration cap (256) hit in `resolve_movement` | `panic!` — server bug; should never trip. |
| 13 | Mutual Attacks (A attacks B; B attacks A; both melee) | Process by entity id. Each Attack does its own primary + counter. Net: A takes `B.attack_damage * 2`; B takes `A.attack_damage * 2`. Mutual commit hurts both. Documented quirk. |
| 14 | Attack + defender Moves into attacker | A Attacks B; B Moves to A's tile. Step 2: A's attack lands on B at B's pre-move tile, with counter. Step 3: B (if alive) moves to A's tile; conflict; melee combat. Each unit hits the other twice this turn. |
| 15 | Concentrated fire | Multiple units Attack the same target; first ones land, target dies, later attackers find no live target at the tile → wasted. Order is by entity id (deterministic). |
| 16 | Settler attacked | Settler has `attack_damage = 0`. Takes damage normally; counter is computed but lands as 0 damage. Dies at HP = 0. Placeholder until settler-capture spec. |

## Testing

### Pure-function unit tests (no Bevy harness)

`shared/src/combat.rs`:
- `damage_taken(raw, false)` returns raw.
- `damage_taken(raw, true)` returns `raw / 2`.
- `damage_taken(0, _)` returns 0.
- `engagement_is_melee` for ranged Attack, melee Attack, Move-into-enemy.

`server/src/combat.rs` against `resolve_movement_pure`:
- Single mover, no conflict — takes destination.
- 2-way conflict, both alive — each rolls back to start_pos.
- 2-way conflict, one dies — survivor takes tile.
- 2-way conflict, both die — tile empty.
- 3-way all-vs-all damage sums.
- 3-way one survivor — sole survivor takes tile.
- Chain combat (the worked example): A@T1→T2, B@T2 stays, C@T3→T1 → final positions match table.
- Fortified defender bonus: 2-way conflict where defender is Fortified+stationary; takes half damage.
- Fortified attacker no bonus: marker present but action = Move; takes full damage.

### Server-side ECS integration tests (extend the `server/src/turn.rs` test pattern)

- `apply_fortifying` swaps Fortifying → Fortified.
- `resolve_attacks`: ranged hits at distance 2, no counter.
- `resolve_attacks`: melee adjacent Attack, target damaged, attacker takes counter.
- `resolve_attacks`: Fortified+stationary defender takes 50%.
- `resolve_movement`: unit moves to empty tile.
- `resolve_movement`: swap succeeds, no combat.
- `resolve_movement`: Move-into-enemy stalemate, mover rolls back.
- `resolve_movement`: Move-into-enemy kill, mover takes tile.
- `resolve_movement`: defender escaped — mover takes tile freely.
- Fortified-stripping after action: unit that did Move loses Fortified (after `resolve_movement`); unit that did Attack loses Fortified (after `resolve_attacks`); unit that did Skip / Build loses Fortified (after their resolvers).
- `cleanup_dead_units`: HP=0 unit despawned.
- `handle_unit_action` rejects Move into friendly-occupied tile.
- Existing regression test `test_rejected_action_preserves_prior_marker` still passes.

### Client-side

A small Fortified visual indicator is a follow-up. This spec only ensures the marker is replicated.

### What we don't test

- Multi-turn HP persistence is implicitly covered by integration tests (Health is a normal component).
- Network replication relies on replicon plumbing; manual two-client verify when the implementation lands.

## Modules Touched

| File | Change |
|------|--------|
| `shared/src/units.rs` | Add `Fortified` component (replicated). |
| `shared/src/combat.rs` | New file: `damage_taken`, `damage_dealt`, `engagement_is_melee`. |
| `shared/src/lib.rs` | `pub mod combat;` |
| `shared/src/plugin.rs` | `.replicate::<Fortified>()` |
| `server/src/combat.rs` | New file: `UnitSnapshot`, `ResolveAction`, `CombatDeltas`, `resolve_movement_pure`, plus the wrapper systems `apply_fortifying`, `resolve_attacks`, `resolve_movement`, `cleanup_dead_units`. |
| `server/src/turn.rs` | Delete `resolve_moves`, `resolve_attacks`, `resolve_fortify` stubs. Add Fortified-stripping branches to `resolve_skip` and `resolve_builds` (still stubs otherwise). Tighten `handle_unit_action` to reject Move into friendly-occupied tiles AND into tiles already targeted by another friendly's queued Move. `handle_finish_turn`, `update_turn_phase`, `advance_turn` unchanged. |
| `server/src/main.rs` | Add `mod combat;`. Update the system chain in the resolution window per the pipeline above. |

## Out of Scope (separate specs)

- **Settler capture mechanic** — settler currently dies normally; capture, friendly-stacking with one military unit, and change-of-owner are deferred.
- **City vs unit combat** — capturing cities, city HP, population loss during attacks.
- **Passive heal on Skip** — heal-by-skip in friendly territory; mentioned in the unit-action-menu spec.
- **Damage previews on the client** — when added, will use `shared/src/combat.rs` helpers.
- **Strategic resources** (horses, iron) gating cavalry/knight production.
- **Combat visuals** — floating damage numbers, death animations, fortify badge.

## Future Extensions

- **Defense modifiers from terrain** (forest/hill bonus) — extend `damage_taken` to take a terrain factor.
- **Healing while fortified** (Civ-style "Fortify until healed") — orthogonal action; integrates with the passive-heal spec.
- **Promotion/XP system** — surviving combat could grant XP; modifier on `attack_damage`.
- **Catapult / siege weapons** — area damage or city-only damage modifiers; clean extension to `damage_taken`.
