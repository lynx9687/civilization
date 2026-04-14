# Multiplayer Hex Grid Prototype — Design Spec

## Goal

Build a minimal multiplayer hex-grid game prototype with a dedicated server, simultaneous turns, and click-to-move interaction. Players are black squares on a hex grid. All players submit moves each turn, then moves resolve simultaneously.

## Project Structure

Cargo workspace with three crates:

- **`shared`** (lib) — hex coordinate types & math, ECS components, turn-related types, replicon component registration. Depended on by both server and client.
- **`server`** (bin) — headless Bevy app. Authoritative game logic: grid spawning, player management, move collection, turn resolution. Runs `bevy_replicon` in server mode.
- **`client`** (bin) — rendered Bevy app. Hex grid rendering, player square rendering, mouse input (click-to-move), turn state display. Runs `bevy_replicon` in client mode.

Dependencies: `bevy 0.18`, `bevy_replicon 0.12.1`, `bevy_replicon_renet`.

## Hex Grid

- **Axial coordinates** (q, r) — standard hex math representation.
- **Flat-top orientation** for rendering.
- **`HexPosition { q: i32, r: i32 }`** — shared component, replicated via replicon.
- **6 neighbors** computed via standard axial direction offsets.
- **Grid shape** — hexagonal grid of configurable radius, defaulting to 5 (61 hexes). The radius is hardcoded for the prototype; configurable grid sizing will be added later.
- **Hex tiles are ECS entities** — each tile has a `HexPosition` component. This makes it easy to add per-tile state later (terrain, ownership, fog).
- **Pixel-to-hex and hex-to-pixel conversion** — client-side only. Server operates purely in axial coordinates.

## Turn System

Server-managed state machine with two phases:

### TurnState Resource (replicated)

- **`Accepting`** — players can submit moves. Server tracks which players have submitted.
- **`Resolving`** — all connected players have submitted. Server applies all moves simultaneously, then transitions back to `Accepting`.

### Move Submission

- Client sends a `MoveAction` event to the server (via replicon server events) containing the target `HexPosition`.
- Server validates: is the target a neighbor of the player's current position? Is it within grid bounds? Has this player already submitted this turn?
- Valid moves are stored until all players have submitted.

### Resolution

- Once all connected players have submitted, the server applies all stored moves at once — updating each player's `HexPosition`.
- Updated positions replicate automatically to all clients via replicon.
- No collision detection — players can overlap on the same hex.

### Not in scope for prototype

- Turn timer.
- Skip-turn mechanic.
- Conflict/collision resolution when two players target the same hex.

## Client Rendering & Input

### Hex Rendering

- Each hex tile drawn as a flat-colored hexagon (outline or subtle fill) using Bevy's 2D mesh system.
- No textures or sprites for hexes.

### Player Rendering

- Players rendered as colored squares at the center of their current hex.
- Each player gets a distinct color assigned by the server based on connection order (e.g., first player is dark gray, second is blue, etc.).

### Input

- **Click-to-move** — on mouse click, convert screen position to axial coordinates using pixel-to-hex formula.
- **Hover highlight** — highlight the hex under the mouse cursor so the player knows what they'll click.
- Only adjacent hexes are valid move targets (server validates, but client can also show valid moves).

### Camera

- Fixed 2D camera centered on the grid. No zoom or pan.

### Turn Feedback

- Simple text label in the corner:
  - `"Click a hex to move"` — when in `Accepting` state and player hasn't submitted.
  - `"Waiting for other players..."` — when player has submitted but others haven't.

## Networking & Connection Flow

### Server Startup

- Server binary accepts a port via CLI argument (default: `5000`).
- Starts a headless Bevy app, initializes replicon in server mode with renet transport.
- Generates the hex grid (spawns tile entities).
- Waits for client connections.

### Client Startup

- Client binary accepts a server address via CLI argument (default: `127.0.0.1:5000`).
- Connects via renet transport, replicon syncs the world — client receives all hex tile entities and existing player entities.

### Player Join

- When a client connects, the server spawns a new player entity with a `HexPosition` at the center hex (0, 0). Multiple players may start on the same hex since overlap is allowed.
- The player entity replicates to all clients automatically.

### Player Disconnect

- Server despawns the player entity.
- If mid-turn and the disconnected player hadn't submitted, the server resolves the turn with the remaining players' moves.

### Game Start

- No lobby or ready-up screen. Server begins accepting turns as soon as at least 2 players are connected.

## Testing Strategy

### shared — Unit Tests

- Hex math: neighbor calculation, distance, coordinate conversions.
- Grid generation: correct tile count for given radius, boundary validation.
- Movement validation: valid neighbor check, out-of-bounds rejection.

### server — Integration Tests

- Turn resolution: all moves applied simultaneously, partial submission waits for remaining players.
- Player connect/disconnect handling.

### client — Manual Testing

- Visual verification: hexes render, clicks register, moves replicate across clients.
- No automated rendering or network integration tests for the prototype.

## Future Considerations (not in scope)

- Lobby/matchmaking server managing multiple game instances.
- Configurable grid size at game initialization.
- Turn timer.
- Collision resolution for same-hex moves.
- Terrain, ownership, fog of war on hex tiles.
