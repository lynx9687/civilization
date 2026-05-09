//! Combat resolution: melee via Move-into-enemy, ranged via Attack pre-movement,
//! all-vs-all damage at contested tiles, and a rollback chain when conflicts
//! leave multiple survivors.
//!
//! - `types`: snapshot/delta data shapes shared between the algorithm and ECS layer.
//! - `algorithm`: pure `resolve_movement_pure` and helpers; no Bevy types.
//! - `systems`: ECS wrappers (`resolve_ranged_attacks`, `resolve_movement`, `cleanup_dead_units`).

mod algorithm;
mod systems;
mod types;

pub use systems::{cleanup_dead_units, resolve_movement, resolve_ranged_attacks};
