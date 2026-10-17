use bevy::input::keyboard::KeyboardInput;
use bevy::input::{ButtonState, InputSystems};
use bevy::prelude::*;

use crate::chunks::generate::WATER_LEVEL;
use crate::player::Player;
use crate::world::World;

// Minecraft's: T opens the bar, and / opens it with the slash already typed.
const OPEN_KEY: KeyCode = KeyCode::KeyT;
const OPEN_WITH_SLASH_KEY: KeyCode = KeyCode::Slash;

// How long a line of output stays up once the bar's closed. It's all shown while it's open.
const LINE_SECONDS: f32 = 10.0;
const MAX_LINES: usize = 10;

const TEXT_COLOR: Color = Color::WHITE;
const ERROR_COLOR: Color = Color::srgb(1.0, 0.33, 0.33);
const BAR_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

// How far above the ground `/tp x z` puts the player.
const SURFACE_CLEARANCE: f32 = 2.0;

/// A command bar like Minecraft's chat: T or / opens it, Enter runs what's typed and Escape
/// closes it. While it's open, keys type into it instead of moving the player.
pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandBar>()
            .add_message::<RunCommand>()
            .add_systems(Startup, spawn_bar)
            // straight after input's read, so the keys it takes never reach the game
            .add_systems(PreUpdate, type_into_bar.after(InputSystems))
            .add_systems(Update, (run_commands, draw_bar).chain());
    }
}

#[derive(Resource, Default)]
pub struct CommandBar {
    open: bool,
    input: String,
    // what's been printed, newest last, with when
    lines: Vec<Line>,
    // how many lines have ever been printed, so drawing can tell when there are new ones
    printed: usize,
}

struct Line {
    text: String,
    error: bool,
    at: f32,
}

impl CommandBar {
    fn print(&mut self, text: impl Into<String>, error: bool, at: f32) {
        self.lines.push(Line {
            text: text.into(),
            error,
            at,
        });
        self.printed += 1;
        let excess = self.lines.len().saturating_sub(MAX_LINES);
        self.lines.drain(..excess);
    }
}

/// Whether the bar's closed, so the player can move and look about.
pub fn command_bar_closed(bar: Res<CommandBar>) -> bool {
    !bar.open
}

#[derive(Message)]
struct RunCommand(String);

#[derive(Component)]
struct Output;

#[derive(Component)]
struct Input;

fn spawn_bar(mut commands: Commands) {
    let font = TextFont {
        font_size: bevy::text::FontSize::Px(16.0),
        ..default()
    };
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(6.0),
            right: Val::Px(6.0),
            bottom: Val::Px(6.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|bar| {
            bar.spawn((
                Output,
                Text::default(),
                font.clone(),
                Node {
                    max_width: Val::Px(600.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(BAR_COLOR),
                Visibility::Hidden,
            ));
            bar.spawn((
                Input,
                Text::default(),
                font,
                TextColor(TEXT_COLOR),
                Node {
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(BAR_COLOR),
                Visibility::Hidden,
            ));
        });
}

fn type_into_bar(
    mut bar: ResMut<CommandBar>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut run: MessageWriter<RunCommand>,
) {
    if !bar.open {
        let slash = keys.just_pressed(OPEN_WITH_SLASH_KEY);
        if slash || keys.just_pressed(OPEN_KEY) {
            bar.open = true;
            bar.input = if slash { "/".into() } else { String::new() };
            // the key that opened it would type itself too
            typed.clear();
            keys.reset_all();
        }
        return;
    }

    let modified = keys.any_pressed([
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
    ]);
    for key in typed.read() {
        if key.state != ButtonState::Pressed {
            continue;
        }
        match key.key_code {
            KeyCode::Escape => {
                bar.open = false;
                break;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let line = std::mem::take(&mut bar.input);
                if !line.trim().is_empty() {
                    run.write(RunCommand(line));
                }
                bar.open = false;
                break;
            }
            KeyCode::Backspace => {
                bar.input.pop();
            }
            _ if modified => {}
            _ => {
                if let Some(text) = &key.text {
                    bar.input.extend(text.chars().filter(|c| !c.is_control()));
                }
            }
        }
    }
    // nothing typed moves the player, toggles the overlay or turns the sun
    keys.reset_all();
}

fn run_commands(
    mut lines: MessageReader<RunCommand>,
    mut bar: ResMut<CommandBar>,
    time: Res<Time<Real>>,
    world: Option<Res<World>>,
    mut player: Query<&mut Transform, With<Player>>,
) {
    let now = time.elapsed_secs();
    for RunCommand(line) in lines.read() {
        let Ok(mut transform) = player.single_mut() else {
            continue;
        };
        let result = parse(line, transform.translation).map(|command| match command {
            Command::Teleport(destination) => {
                let to = match destination {
                    Destination::Exact(to) => to,
                    Destination::Surface { x, z } => {
                        let ground = world.as_ref().map_or(WATER_LEVEL as f64, |world| {
                            world.region_store.noise().sample(x as f64, z as f64).height
                        });
                        let top = ground.max(WATER_LEVEL as f64) as f32;
                        Vec3::new(x, top + SURFACE_CLEARANCE, z)
                    }
                };
                transform.translation = to;
                format!("Teleported to {:.1}, {:.1}, {:.1}", to.x, to.y, to.z)
            }
            Command::Seed => match &world {
                Some(world) => format!("Seed: {}", world.seed()),
                None => "The world isn't made yet".into(),
            },
            Command::Help => HELP.into(),
        });
        match result {
            Ok(text) => bar.print(text, false, now),
            Err(text) => bar.print(text, true, now),
        }
    }
}

#[allow(clippy::type_complexity)]
fn draw_bar(
    bar: Res<CommandBar>,
    time: Res<Time<Real>>,
    mut commands: Commands,
    mut output: Query<(Entity, &mut Visibility), (With<Output>, Without<Input>)>,
    mut input: Query<(&mut Text, &mut Visibility), (With<Input>, Without<Output>)>,
    // the lines drawn last, as how many had been printed and how many of the newest were up
    mut drawn: Local<(usize, usize)>,
) {
    let now = time.elapsed_secs();
    if let Ok((mut text, mut visibility)) = input.single_mut() {
        let wanted = format!("{}_", bar.input);
        if text.0 != wanted {
            text.0 = wanted;
        }
        visibility.set_if_neq(if bar.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }

    // the lines still up, as spans so errors can be coloured
    let shown: Vec<&Line> = bar
        .lines
        .iter()
        .filter(|line| bar.open || now - line.at < LINE_SECONDS)
        .collect();
    let Ok((entity, mut visibility)) = output.single_mut() else {
        return;
    };
    visibility.set_if_neq(if shown.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
    if *drawn == (bar.printed, shown.len()) {
        return;
    }
    *drawn = (bar.printed, shown.len());
    let mut entity = commands.entity(entity);
    entity.despawn_related::<Children>();
    entity.with_children(|output| {
        for (i, line) in shown.iter().enumerate() {
            let newline = if i + 1 < shown.len() { "\n" } else { "" };
            output.spawn((
                TextSpan::new(format!("{}{newline}", line.text)),
                TextColor(if line.error { ERROR_COLOR } else { TEXT_COLOR }),
            ));
        }
    });
}

const HELP: &str = "Commands:\n\
    /tp <x> <y> <z>  teleport there; ~ means where you are, ~10 ten blocks on from it\n\
    /tp <x> <z>  teleport above the ground there\n\
    /seed  print the world's seed\n\
    /help  print this";

#[derive(Debug, PartialEq)]
enum Command {
    Teleport(Destination),
    Seed,
    Help,
}

#[derive(Debug, PartialEq)]
enum Destination {
    Exact(Vec3),
    // above the ground at x, z
    Surface { x: f32, z: f32 },
}

// Reads a line typed into the bar, by a player standing at `from`.
fn parse(line: &str, from: Vec3) -> Result<Command, String> {
    let line = line.trim();
    let Some(line) = line.strip_prefix('/') else {
        return Err("Commands start with /, try /help".into());
    };
    let mut words = line.split_whitespace();
    let name = words.next().unwrap_or("");
    let args: Vec<&str> = words.collect();
    match name {
        "tp" | "teleport" => {
            let coord = |word: &str, here: f32| coordinate(word, here);
            match args[..] {
                [x, y, z] => Ok(Command::Teleport(Destination::Exact(Vec3::new(
                    coord(x, from.x)?,
                    coord(y, from.y)?,
                    coord(z, from.z)?,
                )))),
                [x, z] => Ok(Command::Teleport(Destination::Surface {
                    x: coord(x, from.x)?,
                    z: coord(z, from.z)?,
                })),
                _ => Err("Usage: /tp <x> <y> <z> or /tp <x> <z>".into()),
            }
        }
        "seed" => Ok(Command::Seed),
        "help" => Ok(Command::Help),
        "" => Err("Type a command after the /, try /help".into()),
        other => Err(format!("Unknown command /{other}, try /help")),
    }
}

// A number, or ~ for `here`, or ~n for n on from it.
fn coordinate(word: &str, here: f32) -> Result<f32, String> {
    let number = |text: &str| {
        text.parse::<f32>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{word} isn't a number"))
    };
    match word.strip_prefix('~') {
        Some("") => Ok(here),
        Some(offset) => Ok(here + number(offset)?),
        None => number(word),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: Vec3 = Vec3::new(10.0, 64.0, -20.0);

    #[test]
    fn test_teleports_to_exact_coordinates() {
        assert_eq!(
            Ok(Command::Teleport(Destination::Exact(Vec3::new(
                100.0, 80.5, -3000.0
            )))),
            parse("/tp 100 80.5 -3000", HERE)
        );
        assert_eq!(
            Ok(Command::Teleport(Destination::Exact(Vec3::ZERO))),
            parse("  /teleport   0 0 0  ", HERE)
        );
    }

    #[test]
    fn test_tilde_is_relative_to_where_you_are() {
        assert_eq!(
            Ok(Command::Teleport(Destination::Exact(Vec3::new(
                10.0, 74.0, -25.5
            )))),
            parse("/tp ~ ~10 ~-5.5", HERE)
        );
    }

    #[test]
    fn test_two_coordinates_teleport_above_the_ground() {
        assert_eq!(
            Ok(Command::Teleport(Destination::Surface {
                x: 500.0,
                z: -20.0
            })),
            parse("/tp 500 ~", HERE)
        );
    }

    #[test]
    fn test_bad_teleports_are_errors() {
        assert!(parse("/tp", HERE).is_err());
        assert!(parse("/tp 1", HERE).is_err());
        assert!(parse("/tp 1 2 3 4", HERE).is_err());
        assert!(parse("/tp a 2 3", HERE).is_err());
        assert!(parse("/tp ~x 2 3", HERE).is_err());
        assert!(parse("/tp NaN 2 3", HERE).is_err());
        assert!(parse("/tp inf 2 3", HERE).is_err());
    }

    #[test]
    fn test_other_commands() {
        assert_eq!(Ok(Command::Seed), parse("/seed", HERE));
        assert_eq!(Ok(Command::Help), parse("/help", HERE));
        assert!(parse("/fly", HERE)
            .unwrap_err()
            .contains("Unknown command /fly"));
        assert!(parse("tp 1 2 3", HERE).is_err());
        assert!(parse("/", HERE).is_err());
    }

    // The whole plugin, headless: key presses in, the player's position out.
    mod in_app {
        use bevy::input::keyboard::{Key, KeyboardInput};
        use bevy::input::{ButtonState, InputPlugin};
        use bevy::prelude::*;

        use super::super::{CommandBar, CommandPlugin};
        use crate::player::Player;

        fn app() -> App {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, InputPlugin, CommandPlugin));
            app.world_mut().spawn((Player, Transform::default()));
            app.update();
            app
        }

        fn press(app: &mut App, key_code: KeyCode, text: Option<&str>) {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(KeyboardInput {
                    key_code,
                    logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                    state,
                    text: text
                        .filter(|_| state == ButtonState::Pressed)
                        .map(Into::into),
                    repeat: false,
                    window: Entity::PLACEHOLDER,
                });
                app.update();
            }
        }

        fn type_text(app: &mut App, text: &str) {
            for c in text.chars() {
                let key = match c {
                    ' ' => KeyCode::Space,
                    '-' => KeyCode::Minus,
                    _ => KeyCode::KeyA,
                };
                press(app, key, Some(&c.to_string()));
            }
        }

        fn player(app: &mut App) -> Vec3 {
            app.world_mut()
                .query_filtered::<&Transform, With<Player>>()
                .single(app.world())
                .unwrap()
                .translation
        }

        #[test]
        fn test_slash_opens_the_bar_and_enter_teleports() {
            let mut app = app();
            press(&mut app, KeyCode::Slash, Some("/"));
            assert!(app.world().resource::<CommandBar>().open);
            assert_eq!("/", app.world().resource::<CommandBar>().input);
            type_text(&mut app, "tp 12 34 -56");
            assert_eq!("/tp 12 34 -56", app.world().resource::<CommandBar>().input);
            press(&mut app, KeyCode::Enter, None);
            assert!(!app.world().resource::<CommandBar>().open);
            assert_eq!(Vec3::new(12.0, 34.0, -56.0), player(&mut app));
        }

        #[test]
        fn test_typing_doesnt_reach_the_game_and_escape_cancels() {
            let mut app = app();
            press(&mut app, KeyCode::KeyT, Some("t"));
            assert_eq!("", app.world().resource::<CommandBar>().input);
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::KeyW,
                logical_key: Key::Character("w".into()),
                state: ButtonState::Pressed,
                text: Some("w".into()),
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
            app.update();
            assert!(!app
                .world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyW));
            press(&mut app, KeyCode::Backspace, None);
            type_text(&mut app, "/tp 1 2 3");
            press(&mut app, KeyCode::Escape, None);
            assert!(!app.world().resource::<CommandBar>().open);
            assert_eq!(Vec3::ZERO, player(&mut app));
        }
    }
}
