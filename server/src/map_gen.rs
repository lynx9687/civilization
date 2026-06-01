//! Procedural map generation.
//!
//! Replaces the old fixed-radius `spawn_grid`: the map is generated at **game
//! start**, when the player count is known, sized to the player count, with
//! players placed on well-separated tiles of a single connected landmass. It
//! reads the host's `MapSettings` (currently the server default — the lobby UI
//! that lets the host choose them lands in a follow-up).
//!
//! This module ships a deliberately simple but *correct* placeholder generator.
//! The "map-gen quality" follow-up swaps the body of [`generate_map`] for proper
//! noise-based terrain while keeping the same seams (connectivity guarantee,
//! anchor separation, `MapTiles`).

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
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

/// Generates the tiles for a game. Deterministic in `rng`: the same seeded RNG,
/// `settings`, and `player_count` always produce the same map.
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

    let mut terrain: HashMap<HexPosition, Terrain> = HashMap::with_capacity(positions.len());
    for &pos in &positions {
        terrain.insert(pos, roll_terrain(&pos, radius, settings, rng));
    }

    // Keep only the largest connected blob of passable land; drown the rest so
    // the passable set is connected by construction.
    let land = largest_passable_component(&terrain);
    for (pos, t) in terrain.iter_mut() {
        if t.is_passable() && !land.contains(pos) {
            *t = Terrain::Water;
        }
    }

    positions.into_iter().map(|p| (p, terrain[&p])).collect()
}

/// Picks a terrain for one tile. Water is biased toward the rim so the interior
/// stays land; hills/mountains/forest scale with the `MapSettings` knobs.
fn roll_terrain(
    pos: &HexPosition,
    radius: i32,
    settings: &MapSettings,
    rng: &mut impl Rng,
) -> Terrain {
    let origin = HexPosition::new(0, 0);
    let edge = if radius > 0 {
        pos.distance(&origin) as f32 / radius as f32
    } else {
        0.0
    };

    // Water only near the edge (edge^2), so the core landmass is never cut.
    if rng.gen_range(0.0f32..1.0) < settings.water * edge * edge {
        return Terrain::Water;
    }
    // Elevation: a small slice of the hilliness budget becomes impassable peaks.
    let elevation: f32 = rng.gen_range(0.0f32..1.0);
    if elevation < settings.hilliness * 0.15 {
        return Terrain::Mountain;
    }
    if elevation < settings.hilliness {
        return Terrain::Hill;
    }
    if rng.gen_range(0.0f32..1.0) < settings.forest {
        return Terrain::Forest;
    }
    Terrain::Grassland
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
}
