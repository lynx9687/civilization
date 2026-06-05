//! Procedural map generation: built at game start, sized to the player count,
//! with players placed on well-separated tiles of one connected landmass, from
//! the host's `MapSettings`.
//!
//! Terrain comes from two coherent `Fbm<Perlin>` fields (elevation and moisture)
//! sliced into bands, so water/forest/relief form contiguous regions. A mild
//! radial edge falloff biases the rim downward, weighting water toward the edge
//! while leaving an irregular coast and room for inland lakes.
//!
//! Determinism: the noise seed comes from the caller's RNG and output is built in
//! the disc's fixed scan order, so no `HashMap` iteration order leaks in — the
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

/// Tile position → terrain for the current game: the authoritative on-map /
/// passability lookup. Empty until generated (also the run-condition flag).
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

/// Disc radius for `player_count`: grows with the count (room for separated
/// clusters), capped so games stay small.
fn map_radius(size: MapSize, player_count: usize) -> i32 {
    (base_radius(size) + player_count as i32).clamp(3, 16)
}

/// Octaves per field; a few add fine detail without per-tile salt-and-pepper.
const NOISE_OCTAVES: usize = 4;

/// Spatial frequency per hex step: a feature spans ~`1.0 / NOISE_FREQUENCY`
/// tiles. Low enough for coherent multi-tile regions, high enough that the field
/// has many peaks and troughs, so relief scatters into separate clusters and
/// interior dips form lakes instead of one central blob. (Frequency sets feature
/// scale, not whether the map is flat — see `fbm_off_lattice_is_nonzero`.)
const NOISE_FREQUENCY: f64 = 0.21;

/// Strength of the `edge²` falloff. Mild, so the rim is only nudged down: the
/// coast bites in irregularly and interior dips flood into inland lakes instead
/// of water forming a uniform rim.
const EDGE_FALLOFF_STRENGTH: f64 = 0.20;

/// Water fraction at `water` knob 0, plus how much the knob adds. Clamped later
/// so enough land always remains to seat units.
const WATER_BASE: f64 = 0.14;
const WATER_KNOB_SCALE: f64 = 0.45;

/// Share of the land turned into mountains / hills at `hilliness` 1 (scaled by
/// the knob). Mountains are drawn first from the highest elevations, then hills.
const MOUNTAIN_FRAC: f64 = 0.18;
const HILL_FRAC: f64 = 0.32;

/// Generates the tiles for a game. Deterministic in `rng` (same seed/settings/
/// `player_count` ⇒ identical map, across process runs too), and guarantees the
/// passable land forms **one connected component** — any cut-off tile is drowned.
pub fn generate_map(
    settings: &MapSettings,
    player_count: usize,
    rng: &mut impl Rng,
) -> Vec<(HexPosition, Terrain)> {
    let radius = map_radius(settings.size, player_count.max(1));
    let positions = generate_grid(radius);

    // Seed both fields from one RNG draw (moisture offset to decorrelate).
    // `r#gen`: `gen` is a reserved keyword in edition 2024.
    let noise_seed = rng.r#gen::<u32>();
    let elevation = fbm(noise_seed);
    let moisture = fbm(noise_seed ^ 0x9E37_79B9);

    // Sample per tile in the disc's fixed scan order. `land_elev` subtracts the
    // edge falloff so the rim and interior dips rank lowest — those get drowned,
    // keeping the main landmass whole.
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

    // Keep only the largest passable blob; drown the rest so the passable set is
    // connected. Must be the last terrain mutation.
    let land = largest_passable_component(&terrain);
    for (pos, t) in terrain.iter_mut() {
        if t.is_passable() && !land.contains(pos) {
            *t = Terrain::Water;
        }
    }

    positions.into_iter().map(|p| (p, terrain[&p])).collect()
}

/// One tile's coast-adjusted elevation and moisture.
struct TileSample {
    pos: HexPosition,
    land_elev: f64,
    moisture: f64,
}

/// Fbm Perlin field seeded by `seed`.
fn fbm(seed: u32) -> Fbm<Perlin> {
    Fbm::<Perlin>::new(seed).set_octaves(NOISE_OCTAVES)
}

/// Samples `field` at `pos`, scaled by [`NOISE_FREQUENCY`] and normalized to ~0..1.
fn sample(field: &Fbm<Perlin>, pos: HexPosition) -> f64 {
    let raw = field.get([
        pos.q as f64 * NOISE_FREQUENCY,
        pos.r as f64 * NOISE_FREQUENCY,
    ]);
    // Remap Fbm's ~-1..1 to 0..1.
    (raw * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// `edge²` falloff ([`EDGE_FALLOFF_STRENGTH`]) subtracted from elevation, biasing
/// the rim downward so water leans toward the edge and the coast stays irregular.
fn edge_falloff(pos: HexPosition, radius: i32) -> f64 {
    let origin = HexPosition::new(0, 0);
    let edge = if radius > 0 {
        pos.distance(&origin) as f64 / radius as f64
    } else {
        0.0
    };
    EDGE_FALLOFF_STRENGTH * edge * edge
}

/// Classifies each tile into a terrain band by **rank within this map**, not by
/// absolute noise value: the fields are bell-shaped and seed-dependent, so fixed
/// thresholds give wildly varying (often empty) bands. `water` takes the lowest
/// elevations, `hilliness` the highest land, `forest` the wettest lowland.
/// Deterministic: sorts use [`f64::total_cmp`] with `(q, r)` tie-breaks.
fn band_by_rank(samples: &[TileSample], settings: &MapSettings) -> HashMap<HexPosition, Terrain> {
    let n = samples.len();
    let mut terrain: HashMap<HexPosition, Terrain> = HashMap::with_capacity(n);
    if n == 0 {
        return terrain;
    }

    // Elevation order, low → high (position tie-break for determinism).
    let mut by_elev: Vec<usize> = (0..n).collect();
    by_elev.sort_by(|&a, &b| {
        samples[a]
            .land_elev
            .total_cmp(&samples[b].land_elev)
            .then_with(|| {
                (samples[a].pos.q, samples[a].pos.r).cmp(&(samples[b].pos.q, samples[b].pos.r))
            })
    });

    // Water = lowest `water_frac`, capped so enough land remains to seat units.
    let water_frac = (WATER_BASE + settings.water as f64 * WATER_KNOB_SCALE).clamp(0.0, 0.6);
    let water_count = ((n as f64 * water_frac).round() as usize).min(n);

    // Relief is the top of the land order. Clamp so mountains+hills can't exceed
    // the land count (else `lowland_end` underflows on small, very hilly maps).
    let land_count = n - water_count;
    let mut mountain_count =
        (land_count as f64 * settings.hilliness as f64 * MOUNTAIN_FRAC).round() as usize;
    let mut hill_count =
        (land_count as f64 * settings.hilliness as f64 * HILL_FRAC).round() as usize;
    if mountain_count + hill_count > land_count {
        // Cap to the land tiles, preserving the mountain:hill ratio.
        mountain_count =
            (land_count as f64 * MOUNTAIN_FRAC / (MOUNTAIN_FRAC + HILL_FRAC)).round() as usize;
        hill_count = land_count - mountain_count;
    }
    let lowland_end = n - mountain_count - hill_count;

    for (rank, &idx) in by_elev.iter().enumerate() {
        let pos = samples[idx].pos;
        let t = if rank < water_count {
            Terrain::Water
        } else if rank >= n - mountain_count {
            Terrain::Mountain
        } else if rank >= lowland_end {
            Terrain::Hill
        } else {
            // Provisional; the wettest lowland becomes Forest below.
            Terrain::Grassland
        };
        terrain.insert(pos, t);
    }

    // Forest = the wettest `forest` fraction of the lowland.
    let mut lowland: Vec<usize> = by_elev[water_count..lowland_end].to_vec();
    lowland.sort_by(|&a, &b| {
        samples[b]
            .moisture
            .total_cmp(&samples[a].moisture)
            .then_with(|| {
                (samples[a].pos.q, samples[a].pos.r).cmp(&(samples[b].pos.q, samples[b].pos.r))
            })
    });
    let forest_count = (lowland.len() as f64 * settings.forest as f64).round() as usize;
    for &idx in lowland.iter().take(forest_count) {
        terrain.insert(samples[idx].pos, Terrain::Forest);
    }

    terrain
}

/// Largest connected passable component (6-neighborhood). Scans starts in sorted
/// order so the result is independent of `HashMap` iteration — needed for
/// reproducibility.
fn largest_passable_component(terrain: &HashMap<HexPosition, Terrain>) -> HashSet<HexPosition> {
    let passable = |p: &HexPosition| terrain.get(p).is_some_and(|t| t.is_passable());

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

/// Picks `count` well-separated tiles by farthest-point sampling (first =
/// `passable_land[0]`, each next maximizes min-distance to the chosen). Returns
/// up to `passable_land.len()` anchors.
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

/// Places `n` units on distinct passable tiles spiraling out from `anchor`,
/// skipping already-`occupied` tiles (which it then marks).
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

/// Generates the map and seats every active player's starting units. Gated to run
/// once per game by [`should_generate_map`].
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

/// Despawns tiles, cities, and units on return to the lobby so the next game
/// regenerates from scratch.
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

    /// Min hex distance guaranteed between any two players' anchors.
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

    /// `frequency` sets feature scale, not flatness: a single Perlin octave is 0
    /// on the integer lattice, but `Fbm`'s octaves use a non-integer lacunarity
    /// (≈2.094), so the field is nonzero there too.
    #[test]
    fn fbm_off_lattice_is_nonzero() {
        let one_octave = Fbm::<Perlin>::new(12345).set_octaves(1);
        let many_octaves = Fbm::<Perlin>::new(12345).set_octaves(NOISE_OCTAVES);
        let (mut single_max, mut multi_max) = (0.0_f64, 0.0_f64);
        for q in -6..=6 {
            for r in -6..=6 {
                // Sample directly on the integer lattice (frequency 1.0).
                let raw_single = one_octave.get([q as f64, r as f64]);
                let raw_multi = many_octaves.get([q as f64, r as f64]);
                single_max = single_max.max(raw_single.abs());
                multi_max = multi_max.max(raw_multi.abs());
            }
        }
        assert!(
            single_max < 1e-9,
            "one Perlin octave must be flat on the integer lattice, got {single_max}"
        );
        assert!(
            multi_max > 0.05,
            "Fbm must vary on the integer lattice (non-integer lacunarity), got {multi_max}"
        );
    }

    /// A generated map should show several terrain types, not one or two.
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

    /// A fresh RNG with the same seed reproduces the map (no HashMap order leak).
    #[test]
    fn generate_map_reproduces_from_fresh_rng() {
        for seed in 0..4u64 {
            for players in 2..=6usize {
                let first = {
                    let mut rng = StdRng::seed_from_u64(seed);
                    generate_map(&settings(), players, &mut rng)
                };
                let second = {
                    let mut rng = StdRng::seed_from_u64(seed);
                    generate_map(&settings(), players, &mut rng)
                };
                assert_eq!(first, second, "seed {seed}, {players}p must reproduce");
            }
        }
    }

    /// Different seeds give different layouts.
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

    /// Higher `water` ⇒ more water, land stays one component (aggregated over seeds).
    #[test]
    fn higher_water_means_more_water_but_stays_connected() {
        let players = 2;
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
        let players = 2;
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
        let players = 2;
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

    /// A default map is land-dominant with hills, mountains, and forest present
    /// (guards the all-water-default regression).
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
            let water = count_of(&tiles, Terrain::Water);
            assert!(
                water < tiles.len() / 2,
                "seed {seed}: default map should be land-dominant, got {water}/{} water",
                tiles.len()
            );
        }
    }

    /// Tile count grows with player count but stays capped (short games).
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
        // Radius caps at 16 → a full disc is 817 tiles.
        assert!(
            eight <= 817,
            "map radius must stay capped, got {eight} tiles"
        );
    }
}
