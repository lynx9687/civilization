# Unique Starting Unit Positions — Design Spec

## Goal

Guarantee that every starting unit a player receives at join-time spawns on a distinct hex tile — both within a single player's units and across players. Today (`server/src/players.rs:78–101`) the server picks `q` and `r` independently from `gen_range(-2..=2)` for each unit with no per-tile dedup, so two units can collide.

The combat algorithm being landed on `combat-system-spec` assumes turn-start positions are unique; that property is what makes its rollback chain converge. When two units share a tile at game start, the rollback step keeps sending them back to the same tile, the iteration cap (256) trips, and the server panics with `rollback chain failed to terminate` (issue #6).

The fix lives at the spawn site, not in combat. The combat algorithm stays unchanged.

## Scope

**In scope**

- `handle_new_clients` in `server/src/players.rs` — the only place starting units are created today.
- A small reusable helper for picking N distinct positions out of the spawn region.

**Out of scope**

- Combat code (the invariant becomes true by construction at the spawn call site).
- Mid-game unit creation (city production already places into the city tile; not a starting-position problem).
- Player separation — all players still cluster around the origin, same as today. Per-player anchors and squad layouts are a larger design (see *Alternatives considered*).
- Map size and spawn-region radius — keep current values; flag the tightness.

## Approach

**Locked decision: per-tile dedup at spawn time, implemented as enumerate → filter → shuffle → take.** For each new player processed in `handle_new_clients`, build the set of tiles already occupied by existing units (and by units chosen earlier in the same frame), enumerate the candidate spawn region, filter out occupied tiles, shuffle the remainder with `rand`, and assign the first `N` tiles to the new player's `N` starting units.

### Why this shape over the alternatives

- **Versus a retry loop** ("pick random; if taken, pick again"). Same random feel, but no retry bound to tune and no pathological behavior near saturation. Failure becomes a cheap `candidates.len() < count` check rather than hitting a counter, and the outcome is uniform over the available tiles.
- **Versus a deterministic squad layout** (fixed offsets from a per-player anchor). Squad layout solves more (it also separates players), but it requires a *new* prerequisite — choosing player anchors on the map — with its own design questions (anchor-selection algorithm, fairness, configurability). Issue #6 is positional uniqueness; this approach fixes that minimally and doesn't preclude squads layering on top later.

### Frame-local in-flight tracking

`handle_new_clients` is `Update`-scheduled and may match multiple `Added<AuthorizedClient>` entities in a single tick (two players authorize on the same frame). Each iteration uses `Commands::spawn`, which doesn't materialize until `Commands` flushes — so a same-frame second iteration's `Query<&HexPosition, With<Unit>>` would not see the first iteration's just-spawned units.

The fix tracks an `occupied: HashSet<HexPosition>` *outside* the per-client loop, seeded from the existing-units query, and updates it in-line as each unit's position is decided. This makes uniqueness across same-frame joins independent of `Commands` flush ordering.

### Spawn region

Keep the existing axial parallelogram `q ∈ [-2, 2], r ∈ [-2, 2]` (25 tiles) for minimum diff. Capacity check: `max_clients = 8`; starting units will become 3 once `combat-system-spec` lands → up to 24 units. 24-of-25 is barely-fitting. Lift the magic numbers into a `STARTING_AREA_HALF_EXTENT: i32 = 2` constant with a comment noting the tightness and that the region is a parallelogram, not a hex disc; expanding it is then a one-line change when needed. Defer the expansion itself — that's a balance/feel decision, not part of this fix.

### Helper signature

```rust
fn pick_starting_positions(
    occupied: &HashSet<HexPosition>,
    count: usize,
    rng: &mut impl Rng,
) -> Vec<HexPosition>
```

Behavior:
1. Enumerate the spawn region (`q, r ∈ [-STARTING_AREA_HALF_EXTENT, STARTING_AREA_HALF_EXTENT]`, two nested ranges).
2. Filter out positions present in `occupied`.
3. Shuffle the remaining `Vec` in place (e.g. `SliceRandom::shuffle`).
4. Return the first `count` positions.
5. Panic if `candidates.len() < count` with a clear message naming the requested count and available count.

`&mut impl Rng` lets production pass `&mut rand::thread_rng()` and tests pass a seeded `StdRng` for determinism.

### Call-site changes in `handle_new_clients`

- New parameter: `existing_units: Query<&HexPosition, With<Unit>>`.
- Before the per-client loop: `let mut occupied: HashSet<HexPosition> = existing_units.iter().copied().collect(); let mut rng = rand::thread_rng();`
- Inside the per-client loop, *before* the unit-spawning inner loop: `let positions = pick_starting_positions(&occupied, starting_units.len(), &mut rng);`
- Replace the per-unit `gen_range` calls with `positions[i]`. After spawning each unit, `occupied.insert(positions[i]);`.

The existing `rand::thread_rng().gen_range(-2..=2)` calls go away.

## Error handling

The only new failure mode is "spawn region full" (`candidates.len() < count`). That's a server-config error (too many players for the region, or too many starting units), not a runtime fault. Panic with a descriptive message — same severity as the existing `panic!("missing unit definition for {unit_type}")` immediately above it. Whichever fires, the server is unrecoverable; we want the cause loud in the log.

## Testing

Unit tests in `server/src/players.rs`, against the helper:

1. **Distinct positions when `occupied` is empty.** Call with `count = 3`, assert the returned `Vec` has 3 elements and all distinct (round-trip through a `HashSet`).
2. **Excludes occupied positions.** Pre-populate `occupied` with 5 chosen tiles inside the spawn region, call with `count = 3`, assert the returned positions are disjoint from `occupied` and still distinct.
3. **Panics when the region is saturated.** Pre-populate `occupied` with 23 tiles inside the region (leaving 2), call with `count = 3`, expect panic. (`#[should_panic]`.)

These cover the substance. A system-level test of `handle_new_clients` would require Bevy world setup (the existing `turn.rs` tests show this is feasible) but adds little above the helper tests — defer unless we hit a glue bug.

## Alternatives considered

- **Retry-loop dedup.** The other option mentioned in the issue. Equivalent random feel; rejected for retry-bound tuning and saturation behavior, as above.
- **Deterministic squad layout with per-player anchors.** Solves a strictly larger problem (also separates players). Rejected as out-of-scope for this fix; revisit when player separation becomes a balance concern.
- **Expand the spawn region pre-emptively** (e.g. axial radius 3 → ~37 tiles). Worth doing eventually; out of scope here because it changes feel, not just correctness. Mark the constant; revisit if/when starting-unit count or `max_clients` rises.
