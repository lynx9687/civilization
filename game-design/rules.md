# [INVENT-COOL-NAME] Rules
## General
Each player controls a civilization consisting of cities and units. Cities control terrain, extract resources, and produce units. Units are used for combat. The goal of the game is to conquer the whole land. To achieve this, a player needs to destroy all his opponents. An opponent loses when he doesn't have any cities or settler units.

## Terrain
The terrain has the form of a hex grid. Each tile provides the controlling city with the following resources:
- **Production** used for producing units and potentially buildings. Per-city resource.
- **Food** needed to sustain population and allow the city to grow. Per-city resource.
- **Gold** needed for the upkeep of units. Global player resource.

Different terrain types have different values. Terrain type also affects unit movement.

## Cities
Cities control the terrain around them and extract resources. By default, they control the neighboring 6 tiles. As the population grows, the city borders expand and it extracts resources from more tiles. If the population falls, the city keeps claimed borders but stops receiving resources from them. Population growth is based on food. Cities can produce units. Each unit has a production cost. The city needs to work on it for the number of turns necessary to gather the required production. Cities have their own HP. A city can be captured by an enemy melee unit if it has no HP. In that case, any defender unit in the city dies and the city goes under the control of a new player. Cities lose population during attacks.

## Units
Units consume a certain amount of gold each turn. If the player doesn't have enough gold they die. Good starting points should be the following units:
- **Warrior** - basic melee unit
- **Archer** - unit with ranged attack
- **Cavalry** - unit with improved movement
- **Knight** - more powerful but expensive melee unit
- **Settler** - used for creating new cities. Production of a settler consumes one population from the creating city. Settlers have no combat capabilities and can be captured by enemy units.

Other types of units potentially worth considering:
- **Catapult** - unit with improved damage against cities
- **Boat** - if we want to implement naval combat

Two units can't enter the same tile with the exception of a settler. It can occupy the same tile with one other military unit. In case of attacks, the military unit takes damage for it and prevents it from being captured.

## Combat
Combat is turn-based. Unit damage should decrease with falling HP. Units can heal by skipping movement in friendly territory. A unit can't heal if it is attacked each turn.

Combat in multiplayer leads to the problem of move order. I see three ways to solve it:
- **Instant movement** - the server performs unit movement immediately after a player order. The main drawback is that combats are affected by who can click faster.
- **Separate turns** - when one player moves, others wait for him. Most predictable but slows down the game significantly.
- **Random move order** - after all players submit moves they would want to perform, the units move one by one in a random order. Units could also have a speed stat affecting this order.

## Game start
Each player starts with one warrior and settler in a random location. The game should ensure minimal player separation at the start.

## Other ideas and questions
- **Strategic resources** - we could add horses/iron which randomly spawn on some tiles and are required to produce cavalry/knights
- **War** - should players be at war by default (their units can attack each other) or should they declare it?
- **Fog of war** - do we want to implement it?
