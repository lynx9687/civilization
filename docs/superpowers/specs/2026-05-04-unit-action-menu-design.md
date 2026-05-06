# Unit Action Menu — Design Spec

## Goal

Replace the implicit "right-click hex = move" interaction with a Civilization-style action menu. When the player selects a unit, a bottom-anchored bar shows the unit's available verbs; the player picks a verb and (for targeted verbs) a destination on the map. The picked action is queued on the server and executed at turn end.

This spec covers the **UI, event plumbing, server-side validation, and queued-action storage** for all five verbs defined in `2026-05-01-unit-system-design.md`. Server-side resolution for non-move verbs is **deliberately stubbed** — combat, fortify mechanics, build mechanics, and passive heal are each their own follow-up spec.

This spec also folds in a related cleanup: switching from a server-assigned `Unit.id: u32` identifier on the wire to a replicon-mapped `Entity`. The two changes share the same surface (`MoveAction` is being replaced anyway) and the `Unit.id` / `UnitCounter` machinery exists *only* to give events something stable to reference.

## Architectural Overview

Three layers, each with one responsibility:

| Layer | Where | Responsibility |
|-------|-------|----------------|
| Wire  | `shared/src/events.rs` | One client→server event (`UnitActionEvent`) carrying the verb and its arguments. |
| Server | `server/src/turn.rs` | One observer validates and queues; per-verb resolver systems apply queued markers at turn end. |
| Client | `client/src/{input,ui}.rs` | Selection/targeting state machine; bottom action bar rendered from that state; map highlights driven by targeting. |

**Locked decisions** (each chosen during brainstorm):

- **Single coherent flow.** Right-click as a move shortcut is removed. Every verb (including Move) goes through the action bar.
- **Bottom action bar.** A persistent strip anchored bottom-left, mirroring the existing Finish Turn button anchored bottom-right.
- **Unified `UnitAction` enum on the wire**, registered as one event. One observer on the server matches on the variant.
- **One action per unit per turn, mutable.** The latest submission wins; submitting a new verb removes the prior queued marker.
- **Marker-component-per-verb in the ECS.** `MoveTo` (exists), `AttackTarget`, `Fortifying`, `BuildProject`, `Skipping`. Each verb's resolver queries for its own marker.
- **Validation is per-event-on-arrival**, not batched at turn end. The queue is the source of truth; Finish Turn only triggers resolution, not validation.
- **Shared verb-availability helper.** `available_verbs(def)` lives in `shared/`; both client (greying out buttons) and server (rejecting submissions) call it.
- **Silent reject + log on invalid submissions.** No error event back to the client. The client should never submit invalid actions; bad submissions are bugs or malicious clients.
- **`Entity` on the wire** with replicon's entity-mapping (the same `#[entities]` pattern used by `Owner`). `Unit.id` and `UnitCounter` are removed.

## Data Shapes

### Wire event

Replaces the existing `MoveAction` in `shared/src/events.rs`:

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum UnitAction {
    Move    { target: HexPosition },
    Attack  { target: HexPosition },
    Fortify,
    Build   { project: String },
    Skip,
}

#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct UnitActionEvent {
    // replicon remaps server↔client entity IDs via the same pattern as Owner
    #[entities] pub unit: Entity,
    pub action: UnitAction,
}
```

The exact replicon API for mapping entities in client events (e.g. `add_mapped_client_event` vs the derive-based form) is pinned during implementation; the conceptual contract is "the server resolves `unit` to its own server-side entity before validation."

### Verb identity (for UI rendering and validation)

In `shared/src/unit_definition.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitVerb { Move, Attack, Fortify, Build, Skip }

pub fn available_verbs(def: &UnitDefinition) -> Vec<UnitVerb> {
    let mut v = vec![UnitVerb::Move, UnitVerb::Fortify, UnitVerb::Skip];
    if def.attack_damage > 0       { v.push(UnitVerb::Attack); }
    if !def.build_targets.is_empty() { v.push(UnitVerb::Build); }
    v
}

impl UnitAction {
    pub fn verb(&self) -> UnitVerb { /* discriminant mapping */ }
}
```

`UnitVerb` is the *kind* (UI button identity, validator key); `UnitAction` is *kind + payload* (wire shape). Both live in `shared/`.

### Server-side marker components

Beside the existing `MoveTo` in `shared/src/units.rs`. Not replicated, not serialized — they're server-internal queued state, written by the action handler and removed by the resolver:

```rust
#[derive(Component, Debug)] pub struct Fortifying;
#[derive(Component, Debug)] pub struct Skipping;
#[derive(Component, Debug)] pub struct AttackTarget { pub pos: HexPosition }
#[derive(Component, Debug)] pub struct BuildProject { pub name: String }
```

### Removed

- `Unit.id: u32` field — no longer needed; `Entity` is the wire identity.
- `UnitCounter` resource and its `next_id()` callsite in player setup.

### Client-side UI state

Replaces the `selected_unit` field on `Controller` (in `client/src/input.rs`). `Controller.player_id` stays — it's permanent identity, not transient UI state.

```rust
#[derive(Resource, Default)]
pub enum UiState {
    #[default]
    Idle,
    UnitSelected { unit: Entity },
    Targeting   { unit: Entity, verb: TargetableVerb },
}

#[derive(Clone, Copy)]
pub enum TargetableVerb { Move, Attack }
```

Only Move and Attack require a hex pick; Fortify, Skip, and Build resolve their argument client-side at button-press time and dispatch immediately.

## Data Flow — One Verb's Lifecycle

Worked example: warrior selects Attack.

1. **Selection.** Player left-clicks the warrior. `handle_left_click` matches on `UiState::Idle`, transitions to `UnitSelected { unit }`. No map highlight yet.
2. **Bar appears.** A reactive system (`update_action_bar`) detects the `UiState` change, looks up the unit's `UnitDefinition`, calls `available_verbs(def)`, and renders the bar with `[Move, Attack, Fortify, Skip]` enabled and `[Build]` greyed out.
3. **Verb pick.** Player clicks the Attack button. The button observer transitions `UiState` to `Targeting { unit, verb: Attack }`. `update_hex_highlights` reads the new state and lights enemy-occupied hexes within `attack_range` in red.
4. **Target pick.** Player clicks a highlighted enemy hex. `handle_left_click` (now branching on `UiState`) dispatches `UnitActionEvent { unit, action: UnitAction::Attack { target } }` and transitions back to `Idle`.
5. **Server validates.** The `handle_unit_action` observer fires. Common-path: turn phase is `Accepting`, player hasn't submitted Finish Turn, the resolved `Entity` is owned by this client's player, and `available_verbs(def)` contains `Attack`. Verb-specific: target is enemy-occupied within `attack_range`.
6. **Mark queued.** Server removes any prior marker on the unit (`MoveTo`, `AttackTarget`, `Fortifying`, `BuildProject`, `Skipping` — single-action invariant) and inserts `AttackTarget { pos: target }`.
7. **Re-pick allowed.** Until Finish Turn, the player can re-select the warrior, pick Move instead, and the marker swaps. Last-write-wins.
8. **Resolution.** When all players Finish Turn, per-verb resolver systems run. Only `resolve_moves` does real work in this spec; other resolvers log "would resolve attack/fortify/build/skip" and remove their marker. Combat math is the next spec.

### Cancel paths

| From state | Trigger | To state |
|-----------|---------|---------|
| `Targeting { unit, verb }` | ESC pressed | `UnitSelected { unit }` |
| `Targeting { unit, verb }` | Click invalid hex | `UnitSelected { unit }` |
| `Targeting { unit, verb }` | Re-click same verb on bar | `UnitSelected { unit }` |
| `UnitSelected { unit }` | ESC pressed | `Idle` |
| `UnitSelected { unit }` | Click empty hex | `Idle` |
| `UnitSelected { _ }` or `Targeting { _, _ }` | Click another owned unit | `UnitSelected { new_unit }` |

## Server Handler

The single observer that replaces `handle_move`. Sketch (final code lands during implementation):

```rust
pub fn handle_unit_action(
    trigger: On<FromClient<UnitActionEvent>>,
    mut commands: Commands,
    player_map: Res<PlayerMap>,
    units: Query<(&HexPosition, &Owner, &Unit)>,
    enemy_units: Query<(&HexPosition, &Owner), With<Unit>>,
    turn_state: Query<&TurnState>,
    registry: Res<UnitRegistry>,
) {
    // common-path validation
    let Ok(state) = turn_state.single() else { return; };
    if state.phase != TurnPhase::Accepting { return; }

    let ClientId::Client(client) = trigger.client_id else { return; };
    let Some(player) = player_map.client_to_player.get(&client) else { return; };

    let entity = trigger.message.unit;
    let Ok((pos, owner, unit)) = units.get(entity) else { return; };
    if owner.0 != *player { return; }

    let Some(def) = registry.get(&unit.type_id) else { return; };
    let verb = trigger.message.action.verb();
    if !available_verbs(def).contains(&verb) { return; }

    // single-action invariant: clear any prior marker before inserting
    let mut e = commands.entity(entity);
    e.remove::<MoveTo>()
     .remove::<AttackTarget>()
     .remove::<Fortifying>()
     .remove::<BuildProject>()
     .remove::<Skipping>();

    match &trigger.message.action {
        UnitAction::Move { target } => {
            if !is_within_move_range(pos, target, def.move_budget) { return; }
            if !target.in_bounds(GRID_RADIUS) { return; }
            e.insert(MoveTo { pos: *target });
        }
        UnitAction::Attack { target } => {
            let in_range = (pos.distance(target) as u32) <= def.attack_range
                        && pos.distance(target) > 0;
            let enemy_here = enemy_units.iter()
                .any(|(p, o)| p == target && o.0 != *player);
            if !in_range || !enemy_here { return; }
            e.insert(AttackTarget { pos: *target });
        }
        UnitAction::Fortify => { e.insert(Fortifying); }
        UnitAction::Skip    => { e.insert(Skipping);  }
        UnitAction::Build { project } => {
            if !def.build_targets.contains(project) { return; }
            e.insert(BuildProject { name: project.clone() });
        }
    }
}
```

### Resolver split

The existing `resolve_turn` in `server/src/turn.rs:138` bundles "apply moves + advance turn + reset finished" into one system. It is split into per-verb resolvers plus a turn-advance step. All resolvers run before turn advance, all gated by "all players finished":

| System | Job |
|-------|-----|
| `resolve_moves`   | Apply `MoveTo` to `HexPosition`; remove `MoveTo`. (Existing logic, extracted.) |
| `resolve_attacks` | Stub: log + remove `AttackTarget`. (Replaced by combat-resolution spec.) |
| `resolve_fortify` | Stub: log + remove `Fortifying`. (Replaced by combat-resolution spec — adds persistent `Fortified` state.) |
| `resolve_skip`    | Stub: log + remove `Skipping`. (Replaced by passive-heal spec.) |
| `resolve_builds`  | Stub: log + remove `BuildProject`. (Replaced by city/economy spec.) |
| `advance_turn`    | Bump turn number, reset `finished_cnt` and per-player turn state. (Existing logic, extracted.) |

Ordering is explicit (system set with `.before(advance_turn)` on the resolvers).

## Client UI & Input

### Bottom action bar

Spawned once at startup beside the Finish Turn button. A parent `Node` containing five child `Button` entities, each with a `VerbButton(UnitVerb)` marker:

```rust
#[derive(Component)] pub struct ActionBar;
#[derive(Component)] pub struct VerbButton(pub UnitVerb);
```

`update_action_bar` reacts to `UiState` changes:

- `Idle` → bar's `Display::None`.
- `UnitSelected { unit }` or `Targeting { unit, .. }` → look up the unit's `UnitDefinition`, compute `available_verbs(def)`, set each button's `BackgroundColor` (active vs greyed) and disabled state. Highlight the currently-targeting verb.

### Click handlers

`handle_left_click` is rewritten to branch on `UiState`:

| Current state | Click target | Next state | Side effect |
|---------------|--------------|------------|-------------|
| `Idle` | Owned unit | `UnitSelected { unit }` | — |
| `Idle` | anything else | `Idle` | — |
| `UnitSelected { _ }` | Owned unit | `UnitSelected { new_unit }` | — |
| `UnitSelected { _ }` | anything else | `Idle` | — |
| `Targeting { unit, verb }` | Valid hex (move/attack) | `Idle` | dispatch `UnitActionEvent` |
| `Targeting { unit, _ }` | Invalid hex | `UnitSelected { unit }` | — |
| `Targeting { _, _ }` | Owned unit | `UnitSelected { new_unit }` | — |

`handle_right_click` is **deleted** — right-click is no longer a move shortcut.

`handle_verb_button_click` (new observer for `Pointer<Click>` on `VerbButton` entities):

| Verb | Behavior |
|------|----------|
| Move / Attack | Transition `UiState` to `Targeting { unit, verb }`. Re-click toggles back to `UnitSelected`. |
| Fortify / Skip | Dispatch `UnitActionEvent` immediately; transition to `Idle`. |
| Build | Stub: dispatch with `def.build_targets[0]` (only Settler→city today). A picker UI for multi-target Build is a future extension. |

`handle_escape_key` (new): `Targeting` → `UnitSelected`; `UnitSelected` → `Idle`.

### Map highlights

`update_hex_highlights` reads `UiState`:

- `Targeting { unit, verb: Move }` → blue tint on hexes within `move_budget`.
- `Targeting { unit, verb: Attack }` → red tint on enemy-occupied hexes within `attack_range`.
- All other states → only cursor-hover highlight; no range overlay.

`HexMaterials` gains a `valid_attack` material (red-tinted). Existing `valid_move` reused.

### Turn submission gating

Same as today: the `last_submitted` resource short-circuits clicks after Finish Turn for the current turn. Applied uniformly in `handle_left_click`, `handle_verb_button_click`, and `handle_escape_key` (so ESC after submitting doesn't drop you out of "I'm done" UX).

## Edge Cases

1. **Unit despawns mid-turn.** Markers are components on the unit and despawn with it. Client-side, a small system validates the entity in `UiState::UnitSelected/Targeting` still exists each frame; if not, falls back to `Idle`.
2. **Click on owned unit while `Targeting`.** Treated as switching selection: cancel targeting, transition to `UnitSelected { new_unit }`. Bar re-renders with the new unit's verbs.
3. **Player not yet assigned.** Initial state is `Idle` and clicks no-op until `Controller.player_id` is set by `YourPlayer`.
4. **Phase ≠ `Accepting`.** Bar hidden regardless of `UiState`. Same gate as today.
5. **Idempotent re-submit.** Picking the same verb-target combo twice causes the server to remove-and-reinsert the same marker. Harmless.

## Validation Strategy

Per-event-on-arrival; silent reject + log on failure (matches existing `handle_move` style). The client should never submit invalid actions because the bar greys out unavailable verbs, the targeting highlights restrict valid clicks, and `last_submitted` blocks all input after Finish Turn. A bad submission is a bug or a malicious client; logging is sufficient for the prototype.

Validation only confirms "this verb is plausibly executable given the data and current state." For Attack, the eventual combat resolver will need to handle "target moved away" or "target is dead" since turns are simultaneous — that's a combat-resolution concern, not the action menu's.

## Testing

### Unit tests (in `shared/`)

- `available_verbs` — for each unit type in `assets/units/*.ron`, verify the expected set:
  - warrior, archer, cavalry, knight → `{Move, Attack, Fortify, Skip}`
  - settler → `{Move, Fortify, Build, Skip}`
- `UnitAction::verb()` — round-trip each variant.
- `is_within_attack_range` (new helper, parallel to `is_within_move_range`) — boundary cases: distance 0 = false (same hex), exactly `attack_range` = true, `attack_range + 1` = false.

### Server-side integration tests (extend the pattern in `server/src/turn.rs`)

- Submit each verb for a valid unit type → corresponding marker is inserted.
- Submit `Attack` for a settler (`attack_damage = 0`) → no marker (rejected).
- Submit `Build` for a warrior (`build_targets = []`) → no marker (rejected).
- Submit `Move` then `Attack` for the same unit → only `AttackTarget` remains, `MoveTo` is gone.
- Submit any verb when phase is `WaitingForPlayers` → no marker.
- Submit any verb for another player's unit → no marker.

### Client-side

UI logic (state transitions, button enable/disable, targeting highlights) is hard to test without a Bevy test harness. For this spec, manual verification per a checklist in the implementation plan: each cancel path, each verb type, the targeting flow, and the despawn-during-target edge case.

## Modules Touched

| File | Change |
|------|--------|
| `shared/src/events.rs` | Replace `MoveAction` with `UnitAction` enum + `UnitActionEvent`. |
| `shared/src/unit_definition.rs` | Add `UnitVerb`, `available_verbs(def)`, `is_within_attack_range`. |
| `shared/src/units.rs` | Add `Fortifying`, `Skipping`, `AttackTarget`, `BuildProject`. Remove `Unit.id` and `UnitCounter`. |
| `shared/src/plugin.rs` | Replace `MoveAction` registration with mapped `UnitActionEvent`. |
| `server/src/turn.rs` | Replace `handle_move` with `handle_unit_action`; split `resolve_turn` into per-verb resolvers + `advance_turn`. |
| `server/src/players.rs` | Remove `UnitCounter::next_id()` use at unit spawn. |
| `client/src/input.rs` | Add `UiState`, refactor `handle_left_click`, delete `handle_right_click`, add ESC handler. |
| `client/src/ui.rs` | Add `ActionBar`, `VerbButton`, `update_action_bar`, `handle_verb_button_click`. |
| `client/src/visuals.rs` | Add `valid_attack` material to `HexMaterials`. |

## Out of Scope (separate specs)

- **Combat resolution.** What `resolve_attacks` actually does. Damage math, retaliation, simultaneous-attack ordering, kill-on-zero-HP.
- **Persistent fortify.** The `Fortified` state that persists across turns and applies the universal defense multiplier.
- **City/economy.** What `resolve_builds` actually does — project advancement, settler→city outcome, build-target validation against world state.
- **Passive heal.** What `resolve_skip` actually does on friendly territory.
- **Build picker UI.** When a unit has multiple `build_targets`, a sub-menu to pick which one. Today there's only one project per unit so we use `build_targets[0]`.

## Future Extensions

- **Keyboard hotkeys for verbs** (M/A/F/B/S). Easy to add once the `VerbButton` -> `UiState` flow exists.
- **Tooltips on greyed verbs** explaining why they're disabled ("settlers cannot attack").
- **Build-target sub-menu** as units gain multiple projects.
- **Auto-action queues** (Civ-VI "Sleep until healed", "Auto-explore") — additional verbs that schedule recurring actions.
