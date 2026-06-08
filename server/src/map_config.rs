use bevy::prelude::*;
use bevy_replicon::prelude::*;
use shared::components::{Host, TurnPhase, TurnState};
use shared::events::SetMapConfig;
use shared::map_settings::MapSettings;

use crate::players::PlayerMap;

/// Stores the host's chosen map settings while in the lobby. The generator reads
/// `MapSettings` at game start (see `map_gen::generate_map_on_start`), so updating
/// the resource here is all that's needed for the next game to use these settings.
///
/// Only honored when (a) the sender is the host and (b) we're still in the Lobby
/// phase — mirrors `handle_start_game`'s host + phase gate, but without the
/// player-count / defeated / victorious checks (irrelevant to picking a map).
pub fn handle_set_map_config(
    trigger: On<FromClient<SetMapConfig>>,
    player_map: Res<PlayerMap>,
    hosts: Query<(), With<Host>>,
    turn_state: Query<&TurnState>,
    mut map_settings: ResMut<MapSettings>,
) {
    let Ok(state) = turn_state.single() else {
        return;
    };
    if state.phase != TurnPhase::Lobby {
        return;
    }

    let client_entity = match trigger.client_id {
        ClientId::Client(e) => e,
        ClientId::Server => return,
    };
    let Some(&player_entity) = player_map.client_to_player.get(&client_entity) else {
        return;
    };

    if !hosts.contains(player_entity) {
        println!("Rejected SetMapConfig: sender is not the host");
        return;
    }

    *map_settings = trigger.message.0;
    println!("Map settings updated by host: {:?}", *map_settings);
}

#[cfg(test)]
mod tests {
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::state::app::StatesPlugin;
    use bevy_replicon::prelude::*;
    use shared::components::{Host, Player, TurnPhase, TurnState};
    use shared::map_settings::{MapSettings, MapSize};

    use super::*;
    use crate::players::PlayerMap;

    /// Build a minimal app with the observer wired and a default `MapSettings`.
    fn map_config_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
            StatesPlugin,
            RepliconPlugins,
        ));
        app.init_resource::<PlayerMap>();
        app.init_resource::<MapSettings>();
        app.add_observer(handle_set_map_config);
        app.update();
        app
    }

    /// A non-default payload so "did the resource change?" is unambiguous.
    fn sample_settings() -> MapSettings {
        MapSettings {
            size: MapSize::Large,
            seed: Some(42),
            hilliness: 0.5,
            forest: 0.5,
            water: 0.5,
        }
    }

    #[test]
    fn handle_set_map_config_host_in_lobby_updates_settings() {
        let mut app = map_config_app();

        let client = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Lobby,
                turn_number: 0,
                ..Default::default()
            });
            let host_player = world
                .spawn((
                    Player {
                        color_index: 0,
                        gold: 0,
                    },
                    Host,
                ))
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, host_player);
            client
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: SetMapConfig(sample_settings()),
        });
        app.world_mut().flush();

        assert_eq!(
            *app.world().resource::<MapSettings>(),
            sample_settings(),
            "host SetMapConfig in Lobby must update the MapSettings resource"
        );
    }

    #[test]
    fn handle_set_map_config_rejects_non_host_sender() {
        let mut app = map_config_app();

        let non_host_client = {
            let world = app.world_mut();
            world.spawn(TurnState {
                phase: TurnPhase::Lobby,
                turn_number: 0,
                ..Default::default()
            });
            // Host exists, but the message comes from a different (non-host) player.
            world.spawn((
                Player {
                    color_index: 0,
                    gold: 0,
                },
                Host,
            ));
            let non_host_player = world
                .spawn(Player {
                    color_index: 1,
                    gold: 0,
                })
                .id();
            let non_host_client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(non_host_client, non_host_player);
            non_host_client
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(non_host_client),
            message: SetMapConfig(sample_settings()),
        });
        app.world_mut().flush();

        assert_eq!(
            *app.world().resource::<MapSettings>(),
            MapSettings::default(),
            "non-host SetMapConfig must be rejected; settings stay at default"
        );
    }

    #[test]
    fn handle_set_map_config_rejects_outside_lobby() {
        let mut app = map_config_app();

        let client = {
            let world = app.world_mut();
            // Game already started — phase is Accepting, not Lobby.
            world.spawn(TurnState {
                phase: TurnPhase::Accepting,
                turn_number: 1,
                ..Default::default()
            });
            let host_player = world
                .spawn((
                    Player {
                        color_index: 0,
                        gold: 0,
                    },
                    Host,
                ))
                .id();
            let client = world.spawn_empty().id();
            world
                .resource_mut::<PlayerMap>()
                .client_to_player
                .insert(client, host_player);
            client
        };

        app.world_mut().trigger(FromClient {
            client_id: ClientId::Client(client),
            message: SetMapConfig(sample_settings()),
        });
        app.world_mut().flush();

        assert_eq!(
            *app.world().resource::<MapSettings>(),
            MapSettings::default(),
            "SetMapConfig outside Lobby must be rejected; settings stay at default"
        );
    }
}
