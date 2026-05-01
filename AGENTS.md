## Project overview
The goal of this project is to build an online turn-based 4X game loosely inspired by games like Civilization.

## Technologies
The game is written in Rust and built using `cargo`. The game uses the Bevy engine, especially its ECS system. Network communication is handled using `bevy_replicon`, with `bevy_replicon_renet` as the backend.

## Project structure
Cargo workspace with three crates:

- **`server`** (bin) — headless Bevy app. Authoritative game logic: grid spawning, player management, move collection, turn resolution. Runs `bevy_replicon` in server mode.
- **`client`** (bin) — rendered Bevy app. Hex grid rendering, player square rendering, mouse input (click-to-move), turn state display. Runs `bevy_replicon` in client mode.
- **`shared`** (lib) — shared code such as hex coordinate types and math, ECS components, events, and protocol-facing types. Used by both server and client.

## Agent-User Interaction
Before implementing a high-level feature, first create a plan, discuss it with the user, and wait for explicit confirmation. For small changes or direct commands, you may start work immediately.

Keep in mind that the game should be designed using Bevy's ECS system. For high-level features, discuss:
- Which existing components you will use
- Which components you plan to modify and how
- Which new components you plan to add
- Which resources you plan to use, modify, or add
- Which events will be used for client-to-server communication
- Which replicated components or entities will be used for server-to-client communication
- Which systems you will create or modify
- Which crates/files you plan to work in

## Coding guidelines
- Format code with `cargo fmt`
- Run the most relevant `cargo check` or `cargo test` command when feasible. If a command cannot be run, explain why.
- Ensure code quality by running `cargo clippy`. Try to fix the underlying problem rather than supressing the warning. 
- Think about proper code organization. Components and systems should be separated based on their purpose, such as combat, visuals, networking, movement, turns, map generation, or input. Avoid throwing all components into one file.
- Create new files if necessary
- Use Rust `///` comments to describe public components, resources, events, and non-obvious systems
- Use replication for server-to-client communication
- Use events for client-to-server communication
- The server is authoritative for game state. Clients may request actions, but validation and final state changes must happen on the server
- When adding replicated components or client/server events, register them in the appropriate app setup for every crate that needs them
- Prefer Bevy resources for global state instead of singleton entities. Singleton entities are fine when it is necessary to replicate global state from the server to clients
- Use `#[require(Replicated)]` for appropriate components rather than adding `Replicated` directly at entity creation
