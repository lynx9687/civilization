//! Procedural map generation.
//!
//! Replaces the old fixed-radius `spawn_grid`: the map is generated at **game
//! start**, when the player count is known, sized to the player count, with
//! players placed on well-separated tiles of a single connected landmass. It
//! reads the host's `MapSettings` (currently the server default — the lobby UI
//! that lets the host choose them lands in a follow-up).
//!
//! Terrain comes from two coherent noise fields — an *elevation* and a
//! *moisture* `Fbm<Perlin>` — sampled per tile and sliced into bands, rather than
//! independent per-tile dice rolls, so hills/forests/water form contiguous
//! regions. A radial edge falloff pulls elevation down toward the rim, biasing
//! water to the map edge so the core landmass stays connected. The `MapSettings`
//! knobs shift the band thresholds: `water` raises sea level, `hilliness` lifts
//! the hill/mountain cutoffs, `forest` lowers the moisture needed for forest.
//!
//! Determinism: the noise fields are seeded by a single `u32` drawn from the
//! caller's seeded RNG, and the output is built by iterating the sorted-disc
//! `positions` vector — no `HashMap`/`HashSet` iteration order leaks in, so the
//! same seed reproduces the same map across process runs.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use rand::{Rng, SeedableRng, rngs::StdRng};

use shared::components::{DefeatedPlayer, HexTile, Player, TurnPhase, TurnState, VictoriousPlayer};
use shared::hex::{HexPosition, generate_grid};
use shared::map_settings::{MapSettings, MapSize};
use shared::terrain::{Terrain, TerrainTable};
use shared::unit_definition::UnitRegistry;
use shared::units::{ColorIndex, Health, Owner, Unit};

/// Units placed per player at game start.
const STARTING_UNITS: [&str; 5] = ["warrior", "archer", "settler", "knight", "cavalry"];

/// The current game's map: every tile position and its terrain. Empty until
/// generation. It's the authoritative "is this hex on the map / what's there"
/// lookup (movement bounds + passability) and the run-condition flag, so
/// generation happens exactly once per game.
#[derive(Resource, Default, Debug)]
pub struct MapTiles(pub HashMap<HexPosition, Terrain>);

/// Base disc radius for a map size, before scaling by player count.
fn base_radius(size: MapSize) -> i32 {
    match size {
        MapSize::Small => 4,
        MapSize::Medium => 6,
        MapSize::Large => 8,
    }
}

/// Disc radius for `player_count` players. Grows with the count so every player
/// gets a separated cluster of [`STARTING_UNITS`], and is capped so games stay
/// small. Always at least 3.
fn map_radius(size: MapSize, player_count: usize) -> i32 {
    (base_radius(size) + player_count as i32).clamp(3, 16)
}

/// Octaves for both noise fields. A few octaves give broad bands with a little
/// fine detail without turning into per-tile salt-and-pepper.
const NOISE_OCTAVES: usize = 4;

/// Spatial frequency of the noise per hex step. Perlin/Fbm is exactly 0 at
/// integer lattice points, so we *must* sample at a fractional scale — at ~0.18
/// the dominant features span several tiles, giving coherent bands across a
/// radius 4..16 disc rather than noise-per-tile (or a flat zero map).
const NOISE_FREQUENCY: f64 = 0.18;

/// Generates the tiles for a game. Deterministic in `rng`: the same seeded RNG,
/// `settings`, and `player_count` always produce the same map — including across
/// process runs, since the noise seed comes from `rng` and the output is built in
/// the disc's fixed scan order.
///
/// Guarantees the passable-land tiles form **one connected component** — any
/// passable tile that would be cut off is turned to water, so no player can be
/// walled off ("all continents connected").
pub fn generate_map(
    settings: &MapSettings,
    player_count: usize,
    rng: &mut impl Rng,
) -> Vec<(HexPosition, Terrain)> {
    let radius = map_radius(settings.size, player_count.max(1));
    let positions = generate_grid(radius);

    // One u32 from the seeded RNG drives both noise fields; the `noise` crate
    // builds its permutation table deterministically from it, so the same seed
    // reproduces the same fields across runs. Moisture is offset so the two
    // fields are uncorrelated.
    // `gen` is a reserved keyword in edition 2024; call the raw identifier.
    let noise_seed = rng.r#gen::<u32>();
    let elevation = fbm(noise_seed);
    let moisture = fbm(noise_seed ^ 0x9E37_79B9);

    // Sample both fields per tile (iterating the disc's fixed scan order, never a
    // HashMap, so nothing seed-nondeterministic leaks in). `land_elev` carves the
    // coast: subtracting an edge^2 falloff sinks the rim, so the lowest-elevation
    // tiles — the ones water-banding will drown — are coastal, keeping land whole.
    let mut samples: Vec<TileSample> = Vec::with_capacity(positions.len());
    for &pos in &positions {
        let e = sample(&elevation, pos);
        let m = sample(&moisture, pos);
        samples.push(TileSample {
            pos,
            land_elev: e - edge_falloff(pos, radius),
            moisture: m,
        });
    }

    let mut terrain = band_by_rank(&samples, settings);

    // Keep only the largest connected blob of passable land; drown the rest so
    // the passable set is connected by construction. This is the LAST terrain
    // mutation — anything after it could reintroduce a cut-off passable tile.
    let land = largest_passable_component(&terrain);
    for (pos, t) in terrain.iter_mut() {
        if t.is_passable() && !land.contains(pos) {
            *t = Terrain::Water;
        }
    }

    positions.into_iter().map(|p| (p, terrain[&p])).collect()
}

/// One tile's coherent-noise inputs: coast-adjusted elevation and moisture.
struct TileSample {
    pos: HexPosition,
    land_elev: f32,
    moisture: f32,
}

/// Builds a fractal-Brownian-motion Perlin field with our standard octave count,
/// seeded deterministically by `seed`.
fn fbm(seed: u32) -> Fbm<Perlin> {
    Fbm::<Perlin>::new(seed).set_octaves(NOISE_OCTAVES)
}

/// Samples `field` at hex `pos`, returning a value normalized to roughly `0.0..1.0`.
/// Coordinates are scaled by [`NOISE_FREQUENCY`] (never integer — see the const)
/// so features span several tiles.
fn sample(field: &Fbm<Perlin>, pos: HexPosition) -> f32 {
    let raw = field.get([
        pos.q as f64 * NOISE_FREQUENCY,
        pos.r as f64 * NOISE_FREQUENCY,
    ]);
    // Fbm output is roughly in -1.0..1.0; remap to 0.0..1.0 and clamp the tails.
    ((raw as f32) * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Radial falloff (edge^2, biting only near the rim) subtracted from elevation so
/// the coast sinks and water bands to the map edge rather than the interior.
fn edge_falloff(pos: HexPosition, radius: i32) -> f32 {
    let origin = HexPosition::new(0, 0);
    let edge = if radius > 0 {
        pos.distance(&origin) as f32 / radius as f32
    } else {
        0.0
    };
    edge * edge
}

/// Classifies every tile into a terrain band by **rank within this map**, not by
/// absolute noise value. The noise fields are bell-shaped and sit at a
/// seed-dependent place on the number line, so absolute thresholds give wildly
/// varying (often empty) bands; ranking the actual per-map distribution makes the
/// knobs control real proportions and is distribution-invariant.
///
/// - `water` ⇒ the lowest fraction of `land_elev` becomes Water (coastal, since
///   the edge falloff put the rim at the bottom — keeps land connected).
/// - `hilliness` ⇒ the highest land by elevation becomes Mountain, then Hill.
/// - `forest` ⇒ the wettest remaining lowland becomes Forest.
///
/// Determinism: ranks are computed by sorting with [`f32::total_cmp`] (NaN-safe)
/// and breaking ties on `(q, r)`, then terrain is assigned by index — no float
/// threshold comparisons, no HashMap iteration order.
fn band_by_rank(samples: &[TileSample], settings: &MapSettings) -> HashMap<HexPosition, Terrain> {
    let n = samples.len();
    let mut terrain: HashMap<HexPosition, Terrain> = HashMap::with_capacity(n);
    if n == 0 {
        return terrain;
    }

    // Elevation order, low → high; ties broken by position for reproducibility.
    let mut by_elev: Vec<usize> = (0..n).collect();
    by_elev.sort_by(|&a, &b| {
        samples[a]
            .land_elev
            .total_cmp(&samples[b].land_elev)
            .then_with(|| {
                (samples[a].pos.q, samples[a].pos.r).cmp(&(samples[b].pos.q, samples[b].pos.r))
            })
    });

    // Water = lowest `water_frac` of tiles. Capped so enough land remains to seat
    // every player's units (radius grows with player count, so this rarely bites,
    // but the cap is what keeps the high-water knob from drowning the game).
    let water_frac = (0.12 + settings.water * 0.45).clamp(0.0, 0.6);
    let water_count = ((n as f32 * water_frac).round() as usize).min(n);

    // Among land (the high end of the elevation order), the top slices are relief.
    // Clamp so mountains+hills never exceed the land tiles (high hilliness on a
    // small map could otherwise overrun the lowland and underflow the slice).
    let land_count = n - water_count;
    let mut mountain_count = (land_count as f32 * settings.hilliness * 0.12).round() as usize;
    let mut hill_count = (land_count as f32 * settings.hilliness * 0.38).round() as usize;
    if mountain_count + hill_count > land_count {
        let relief = land_count;
        // Preserve the mountain:hill ratio (~0.12:0.38) when capping.
        mountain_count = relief / 4;
        hill_count = relief - mountain_count;
    }
    let lowland_end = n - mountain_count - hill_count;

    // Assign elevation-driven bands by rank.
    for (rank, &idx) in by_elev.iter().enumerate() {
        let pos = samples[idx].pos;
        let t = if rank < water_count {
            Terrain::Water
        } else if rank >= n - mountain_count {
            Terrain::Mountain
        } else if rank >= lowland_end {
            Terrain::Hill
        } else {
            // Lowland: decided by moisture rank below.
            Terrain::Grassland
        };
        terrain.insert(pos, t);
    }

    // Forest = the wettest `forest` fraction of the remaining lowland (grassland).
    let mut lowland: Vec<usize> = by_elev[water_count..lowland_end].to_vec();
    lowland.sort_by(|&a, &b| {
        samples[b]
            .moisture
            .total_cmp(&samples[a].moisture)
            .then_with(|| {
                (samples[a].pos.q, samples[a].pos.r).cmp(&(samples[b].pos.q, samples[b].pos.r))
            })
    });
    let forest_count = (lowland.len() as f32 * settings.forest).round() as usize;
    for &idx in lowland.iter().take(forest_count) {
        terrain.insert(samples[idx].pos, Terrain::Forest);
    }

    terrain
}

/// Largest connected set of passable tiles, walking the 6 hex neighbors. On a
/// size tie the lexicographically-smallest start wins, so the result is
/// independent of `HashMap` iteration order — required for seed reproducibility.
fn largest_passable_component(terrain: &HashMap<HexPosition, Terrain>) -> HashSet<HexPosition> {
    let passable = |p: &HexPosition| terrain.get(p).is_some_and(|t| t.is_passable());

    // Deterministic scan order (the same seed must reproduce the same map across
    // process runs, where HashMap hashing is randomized).
    let mut starts: Vec<HexPosition> = terrain.keys().copied().collect();
    starts.sort_by_key(|p| (p.q, p.r));

    let mut seen: HashSet<HexPosition> = HashSet::new();
    let mut best: HashSet<HexPosition> = HashSet::new();

    for start in starts {
        if !passable(&start) || seen.contains(&start) {
            continue;
        }
        let mut component = HashSet::new();
        let mut queue = VecDeque::from([start]);
        seen.insert(start);
        while let Some(pos) = queue.pop_front() {
            component.insert(pos);
            for nb in pos.neighbors() {
                if passable(&nb) && seen.insert(nb) {
                    queue.push_back(nb);
                }
            }
        }
        if component.len() > best.len() {
            best = component;
        }
    }
    best
}

/// Chooses `count` well-separated passable tiles via farthest-point sampling:
/// the first anchor is `passable_land[0]` (deterministic given a sorted input),
/// each subsequent anchor maximizes the minimum distance to those already chosen.
/// Never panics — returns up to `min(count, passable_land.len())` anchors.
pub fn pick_player_anchors(passable_land: &[HexPosition], count: usize) -> Vec<HexPosition> {
    if count == 0 || passable_land.is_empty() {
        return Vec::new();
    }
    let mut anchors = Vec::with_capacity(count.min(passable_land.len()));
    anchors.push(passable_land[0]);
    while anchors.len() < count {
        let Some(&next) = passable_land
            .iter()
            .filter(|p| !anchors.contains(p))
            .max_by_key(|p| anchors.iter().map(|a| a.distance(p)).min().unwrap_or(0))
        else {
            break;
        };
        anchors.push(next);
    }
    anchors
}

/// Places `n` units near `anchor` on distinct passable tiles, spiraling outward
/// over the connected landmass and skipping tiles already taken by other players.
/// The anchor itself is the first spot. Marks chosen tiles in `occupied`.
pub fn place_units_near(
    anchor: HexPosition,
    n: usize,
    land: &HashSet<HexPosition>,
    occupied: &mut HashSet<HexPosition>,
) -> Vec<HexPosition> {
    let mut spots = Vec::with_capacity(n);
    let mut visited: HashSet<HexPosition> = HashSet::from([anchor]);
    let mut queue: VecDeque<HexPosition> = VecDeque::from([anchor]);
    while spots.len() < n {
        let Some(pos) = queue.pop_front() else {
            break;
        };
        if land.contains(&pos) && occupied.insert(pos) {
            spots.push(pos);
        }
        for nb in pos.neighbors() {
            if land.contains(&nb) && visited.insert(nb) {
                queue.push_back(nb);
            }
        }
    }
    spots
}

/// Run condition: a game just entered `Accepting` and no map exists yet.
pub fn should_generate_map(turn_state: Query<&TurnState>, map_tiles: Res<MapTiles>) -> bool {
    turn_state
        .single()
        .map(|s| s.phase == TurnPhase::Accepting)
        .unwrap_or(false)
        && map_tiles.0.is_empty()
}

/// Generates the map and places every active player's starting units. Runs once
/// per game (gated by [`should_generate_map`]) — kept out of the `StartGame`
/// observer so that observer's tests don't need any of these resources.
#[allow(clippy::type_complexity)]
pub fn generate_map_on_start(
    mut commands: Commands,
    settings: Res<MapSettings>,
    mut map_tiles: ResMut<MapTiles>,
    terrain_table: Res<TerrainTable>,
    registry: Res<UnitRegistry>,
    players: Query<(Entity, &Player), (Without<DefeatedPlayer>, Without<VictoriousPlayer>)>,
) {
    let player_list: Vec<(Entity, u8)> = players.iter().map(|(e, p)| (e, p.color_index)).collect();
    if player_list.is_empty() {
        return;
    }

    let mut rng = match settings.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_entropy(),
    };

    let tiles = generate_map(&settings, player_list.len(), &mut rng);
    for (pos, terrain) in &tiles {
        commands.spawn((
            Replicated,
            HexTile,
            *pos,
            *terrain,
            terrain_table.yields(*terrain),
        ));
        map_tiles.0.insert(*pos, *terrain);
    }

    let land_vec: Vec<HexPosition> = tiles
        .iter()
        .filter(|(_, t)| t.is_passable())
        .map(|(p, _)| *p)
        .collect();
    let land_set: HashSet<HexPosition> = land_vec.iter().copied().collect();
    let anchors = pick_player_anchors(&land_vec, player_list.len());

    let mut occupied: HashSet<HexPosition> = HashSet::new();
    for (&(player_entity, color_index), anchor) in player_list.iter().zip(anchors.iter()) {
        let spots = place_units_near(*anchor, STARTING_UNITS.len(), &land_set, &mut occupied);
        for (unit_type, pos) in STARTING_UNITS.iter().zip(spots.iter()) {
            let type_id = registry
                .id_of(unit_type)
                .unwrap_or_else(|| panic!("missing unit definition for {unit_type}"));
            let definition = registry
                .get(&type_id)
                .unwrap_or_else(|| panic!("registry has id but no definition for {unit_type}"));
            commands.spawn((
                Unit { type_id },
                *pos,
                Owner(player_entity),
                ColorIndex(color_index),
                Health::full(definition.hp),
            ));
            println!(
                "Spawned {unit_type} at ({}, {}) for player {player_entity}",
                pos.q, pos.r
            );
        }
    }

    println!(
        "Generated map: {} tiles, {} players placed",
        tiles.len(),
        player_list.len()
    );
}

/// Run condition: back in the lobby with a stale map still around.
pub fn should_cleanup_map(turn_state: Query<&TurnState>, map_tiles: Res<MapTiles>) -> bool {
    turn_state
        .single()
        .map(|s| s.phase == TurnPhase::Lobby)
        .unwrap_or(false)
        && !map_tiles.0.is_empty()
}

/// Tears the board down when a finished game returns to the lobby so the next
/// game regenerates from scratch. Despawns tiles, cities, and any stray units.
pub fn cleanup_map_on_lobby(
    mut commands: Commands,
    mut map_tiles: ResMut<MapTiles>,
    tiles: Query<Entity, With<HexTile>>,
    cities: Query<Entity, With<shared::cities::City>>,
    units: Query<Entity, With<Unit>>,
) {
    for entity in tiles.iter().chain(cities.iter()).chain(units.iter()) {
        commands.entity(entity).despawn();
    }
    map_tiles.0.clear();
    println!("Cleared previous map on return to lobby");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimum hex distance the generator guarantees between any two players'
    /// starting anchors (the farthest-point sampler comfortably exceeds this).
    const MIN_PLAYER_SEPARATION: i32 = 4;

    fn settings() -> MapSettings {
        MapSettings::default()
    }

    fn passable_positions(tiles: &[(HexPosition, Terrain)]) -> Vec<HexPosition> {
        tiles
            .iter()
            .filter(|(_, t)| t.is_passable())
            .map(|(p, _)| *p)
            .collect()
    }

    #[test]
    fn generate_map_is_deterministic_for_a_seed() {
        for seed in 0..6u64 {
            for players in 2..=8usize {
                let mut a = StdRng::seed_from_u64(seed);
                let mut b = StdRng::seed_from_u64(seed);
                let map_a = generate_map(&settings(), players, &mut a);
                let map_b = generate_map(&settings(), players, &mut b);
                assert_eq!(
                    map_a, map_b,
                    "seed {seed}, {players}p: same seed must yield an identical map"
                );
            }
        }
    }

    #[test]
    fn passable_land_is_one_connected_component() {
        for seed in 0..8u64 {
            for players in 2..=8usize {
                let mut rng = StdRng::seed_from_u64(seed);
                let tiles = generate_map(&settings(), players, &mut rng);
                let map: HashMap<HexPosition, Terrain> = tiles.iter().copied().collect();
                let component = largest_passable_component(&map);
                let passable = passable_positions(&tiles).len();
                assert_eq!(
                    component.len(),
                    passable,
                    "seed {seed}, {players}p: every passable tile must be in one component"
                );
                assert!(passable > 0, "seed {seed}, {players}p: map must have land");
            }
        }
    }

    #[test]
    fn map_is_large_enough_to_seat_all_units() {
        for players in 2..=8usize {
            let mut rng = StdRng::seed_from_u64(7);
            let tiles = generate_map(&settings(), players, &mut rng);
            let land = passable_positions(&tiles);
            assert!(
                land.len() >= players * STARTING_UNITS.len(),
                "{players}p: {} land tiles cannot seat {} units",
                land.len(),
                players * STARTING_UNITS.len()
            );
        }
    }

    #[test]
    fn player_anchors_are_well_separated() {
        for seed in 0..8u64 {
            for players in 2..=8usize {
                let mut rng = StdRng::seed_from_u64(seed);
                let tiles = generate_map(&settings(), players, &mut rng);
                let land = passable_positions(&tiles);
                let anchors = pick_player_anchors(&land, players);
                assert_eq!(anchors.len(), players, "must place every player");
                for i in 0..anchors.len() {
                    for j in (i + 1)..anchors.len() {
                        let d = anchors[i].distance(&anchors[j]);
                        assert!(
                            d >= MIN_PLAYER_SEPARATION,
                            "seed {seed}, {players}p: anchors {i},{j} only {d} apart"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn units_are_placed_on_distinct_land_tiles() {
        let mut rng = StdRng::seed_from_u64(1);
        let tiles = generate_map(&settings(), 3, &mut rng);
        let land_vec = passable_positions(&tiles);
        let land: HashSet<HexPosition> = land_vec.iter().copied().collect();
        let anchors = pick_player_anchors(&land_vec, 3);

        let mut occupied = HashSet::new();
        for anchor in &anchors {
            let spots = place_units_near(*anchor, STARTING_UNITS.len(), &land, &mut occupied);
            assert_eq!(
                spots.len(),
                STARTING_UNITS.len(),
                "all units must be seated"
            );
            for s in &spots {
                assert!(land.contains(s), "unit must sit on passable land");
            }
        }
        // No two units share a tile across all players.
        assert_eq!(occupied.len(), 3 * STARTING_UNITS.len());
    }

    /// Counts tiles of each terrain across a map.
    fn histogram(tiles: &[(HexPosition, Terrain)]) -> HashMap<Terrain, usize> {
        let mut counts: HashMap<Terrain, usize> = HashMap::new();
        for (_, t) in tiles {
            *counts.entry(*t).or_default() += 1;
        }
        counts
    }

    fn count_of(tiles: &[(HexPosition, Terrain)], terrain: Terrain) -> usize {
        tiles.iter().filter(|(_, t)| *t == terrain).count()
    }

    /// Builds settings with overridden knobs (size/seed kept at default).
    fn knobs(hilliness: f32, forest: f32, water: f32) -> MapSettings {
        MapSettings {
            hilliness,
            forest,
            water,
            ..MapSettings::default()
        }
    }

    /// Guards the integer-lattice trap: Perlin is exactly 0 at integer
    /// coordinates, so sampling without a fractional frequency yields a flat
    /// single-terrain map. With banding we expect several terrain types.
    #[test]
    fn map_has_multiple_terrain_types() {
        for seed in 0..6u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let tiles = generate_map(&settings(), 4, &mut rng);
            let kinds = histogram(&tiles).len();
            assert!(
                kinds >= 3,
                "seed {seed}: expected coherent bands (>=3 terrain types), got {kinds}"
            );
        }
    }

    /// Determinism across process runs is structural: the noise seed flows from
    /// the RNG and output is built in the disc's fixed scan order, so a fresh RNG
    /// seeded identically reproduces the map. (HashMap hashing is randomized per
    /// run; this asserts it never leaks into the result.)
    #[test]
    fn generate_map_reproduces_from_fresh_rng() {
        for seed in 0..4u64 {
            for players in 2..=6usize {
                let first = {
                    let mut rng = StdRng::seed_from_u64(seed);
                    generate_map(&settings(), players, &mut rng)
                };
                // Drop the RNG entirely, re-seed from scratch (mimics a new run).
                let second = {
                    let mut rng = StdRng::seed_from_u64(seed);
                    generate_map(&settings(), players, &mut rng)
                };
                assert_eq!(first, second, "seed {seed}, {players}p must reproduce");
            }
        }
    }

    /// Different seeds must produce different layouts (the noise field varies),
    /// not just the same proportions in the same places.
    #[test]
    fn different_seeds_give_different_layouts() {
        let a = {
            let mut rng = StdRng::seed_from_u64(1);
            generate_map(&settings(), 4, &mut rng)
        };
        let b = {
            let mut rng = StdRng::seed_from_u64(2);
            generate_map(&settings(), 4, &mut rng)
        };
        assert_ne!(a, b, "distinct seeds should yield distinct maps");
    }

    /// Higher `water` ⇒ more water tiles, while the land stays one connected
    /// component. Aggregated over seeds because a single small map can wobble.
    #[test]
    fn higher_water_means_more_water_but_stays_connected() {
        let players = 4;
        let mut low_water = 0usize;
        let mut high_water = 0usize;
        for seed in 0..8u64 {
            let low = {
                let mut rng = StdRng::seed_from_u64(seed);
                generate_map(&knobs(0.3, 0.3, 0.1), players, &mut rng)
            };
            let high = {
                let mut rng = StdRng::seed_from_u64(seed);
                generate_map(&knobs(0.3, 0.3, 0.6), players, &mut rng)
            };
            low_water += count_of(&low, Terrain::Water);
            high_water += count_of(&high, Terrain::Water);

            // Connectivity invariant must hold at both extremes.
            for tiles in [&low, &high] {
                let map: HashMap<HexPosition, Terrain> = tiles.iter().copied().collect();
                let component = largest_passable_component(&map);
                assert_eq!(
                    component.len(),
                    passable_positions(tiles).len(),
                    "seed {seed}: land must stay one component at any water level"
                );
                assert!(
                    passable_positions(tiles).len() >= players * STARTING_UNITS.len(),
                    "seed {seed}: must still seat units even with high water"
                );
            }
        }
        assert!(
            high_water > low_water,
            "more water knob should yield more water tiles: low={low_water}, high={high_water}"
        );
    }

    /// Higher `hilliness` ⇒ more hills + mountains, aggregated over seeds.
    #[test]
    fn higher_hilliness_means_more_relief() {
        let players = 4;
        let mut low_relief = 0usize;
        let mut high_relief = 0usize;
        for seed in 0..8u64 {
            let low = {
                let mut rng = StdRng::seed_from_u64(seed);
                generate_map(&knobs(0.1, 0.3, 0.2), players, &mut rng)
            };
            let high = {
                let mut rng = StdRng::seed_from_u64(seed);
                generate_map(&knobs(0.9, 0.3, 0.2), players, &mut rng)
            };
            low_relief += count_of(&low, Terrain::Hill) + count_of(&low, Terrain::Mountain);
            high_relief += count_of(&high, Terrain::Hill) + count_of(&high, Terrain::Mountain);
        }
        assert!(
            high_relief > low_relief,
            "more hilliness should yield more hills+mountains: low={low_relief}, high={high_relief}"
        );
    }

    /// Higher `forest` ⇒ more forest tiles, aggregated over seeds.
    #[test]
    fn higher_forest_means_more_forest() {
        let players = 4;
        let mut low_forest = 0usize;
        let mut high_forest = 0usize;
        for seed in 0..8u64 {
            let low = {
                let mut rng = StdRng::seed_from_u64(seed);
                generate_map(&knobs(0.3, 0.1, 0.2), players, &mut rng)
            };
            let high = {
                let mut rng = StdRng::seed_from_u64(seed);
                generate_map(&knobs(0.3, 0.9, 0.2), players, &mut rng)
            };
            low_forest += count_of(&low, Terrain::Forest);
            high_forest += count_of(&high, Terrain::Forest);
        }
        assert!(
            high_forest > low_forest,
            "more forest knob should yield more forest tiles: low={low_forest}, high={high_forest}"
        );
    }

    /// A default map is land-dominant with every terrain represented: rank-based
    /// banding must yield real relief and forest at the *default* knobs, not only
    /// at extremes. Guards the bell-curve-vs-absolute-threshold regression that
    /// produced flat, all-water default maps.
    #[test]
    fn default_map_has_relief_and_is_land_dominant() {
        for seed in 0..6u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let tiles = generate_map(&settings(), 4, &mut rng);
            let h = histogram(&tiles);
            assert!(
                h.get(&Terrain::Hill).copied().unwrap_or(0) > 0,
                "seed {seed}: default map must have hills"
            );
            assert!(
                h.get(&Terrain::Mountain).copied().unwrap_or(0) > 0,
                "seed {seed}: default map must have mountains"
            );
            assert!(
                h.get(&Terrain::Forest).copied().unwrap_or(0) > 0,
                "seed {seed}: default map must have forest"
            );
            // Land should dominate — most of a default map is playable, not sea.
            let water = count_of(&tiles, Terrain::Water);
            assert!(
                water < tiles.len() / 2,
                "seed {seed}: default map should be land-dominant, got {water}/{} water",
                tiles.len()
            );
        }
    }

    /// Map radius (and thus tile count) grows with player count but stays capped
    /// small — short games. Asserts the existing sizing philosophy is preserved.
    #[test]
    fn map_grows_with_players_but_stays_capped() {
        let two = {
            let mut rng = StdRng::seed_from_u64(3);
            generate_map(&settings(), 2, &mut rng).len()
        };
        let eight = {
            let mut rng = StdRng::seed_from_u64(3);
            generate_map(&settings(), 8, &mut rng).len()
        };
        assert!(eight > two, "more players should yield a larger map");
        // Radius is clamped to 16; a full disc of radius 16 is 817 tiles. Stay
        // well under a sprawling map to keep games short.
        assert!(
            eight <= 817,
            "map radius must stay capped, got {eight} tiles"
        );
    }
}
