//! Deep Net v2 - 3D Cyberspace for AI-Foundation
//!
//! A Bevy-based 3D visualization layer on top of AI-Foundation infrastructure.
//! Connects to the same PostgreSQL, hybrid-server, and AFP protocol.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::input::mouse::MouseMotion;
use sysinfo::System;
use std::f32::consts::PI;

mod plugins;
mod components;
mod systems;
mod resources;
pub mod federation;

use federation::FederationPlugin;

// ============================================================================
// RESOURCES - Shared state
// ============================================================================

#[derive(Resource)]
struct GameSettings {
    mouse_sensitivity: f32,
    move_speed: f32,
    sprint_multiplier: f32,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 0.002,
            move_speed: 10.0,
            sprint_multiplier: 2.5,
        }
    }
}

#[derive(Resource)]
struct AIFoundationConnection {
    postgres_url: String,
    hybrid_server_url: String,
    ai_id: String,
    connected: bool,
}

impl Default for AIFoundationConnection {
    fn default() -> Self {
        Self {
            postgres_url: std::env::var("POSTGRES_URL")
                .unwrap_or_else(|_| "postgres://postgres:ai_foundation_pass@127.0.0.1:15432/ai_foundation".into()),
            hybrid_server_url: std::env::var("HYBRID_SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:31415".into()),
            ai_id: std::env::var("AI_ID")
                .unwrap_or_else(|_| "deep-net-user".into()),
            connected: false,
        }
    }
}

#[derive(Resource)]
struct HardwareInfo {
    total_ram_gb: f32,
    used_ram_gb: f32,
    drives: Vec<DriveInfo>,
    cpu_count: usize,
    last_update: f64,
}

#[derive(Clone)]
struct DriveInfo {
    name: String,
    total_gb: f32,
    used_gb: f32,
}

impl Default for HardwareInfo {
    fn default() -> Self {
        Self {
            total_ram_gb: 0.0,
            used_ram_gb: 0.0,
            drives: Vec::new(),
            cpu_count: 0,
            last_update: 0.0,
        }
    }
}

#[derive(Resource)]
struct MouseGrabbed(bool);

impl Default for MouseGrabbed {
    fn default() -> Self {
        Self(true)  // Start with mouse grabbed (in-game mode)
    }
}

// ============================================================================
// COMPONENTS
// ============================================================================

#[derive(Component)]
struct Player;

#[derive(Component)]
struct PlayerCamera {
    yaw: f32,
    pitch: f32,
}

impl Default for PlayerCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

#[derive(Component)]
struct AIEntity {
    ai_id: String,
    is_online: bool,
}

#[derive(Component)]
struct HardwareVisualization {
    kind: HardwareKind,
}

#[derive(Clone, Copy)]
enum HardwareKind {
    RamBlock { index: usize, is_used: bool },
    DriveRing { drive_index: usize, ring_index: usize, is_used: bool },
    CpuCore { index: usize },
}

#[derive(Component)]
struct HexTile {
    q: i32,
    r: i32,
}

#[derive(Component)]
struct Glow {
    base_emissive: Color,
    pulse_speed: f32,
    pulse_amount: f32,
}

// ============================================================================
// STARTUP SYSTEMS
// ============================================================================

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    println!("==========================================");
    println!("  DEEP NET v2 - AI-Foundation Cyberspace");
    println!("==========================================");
    println!();
    println!("WASD = Move");
    println!("Mouse = Look");
    println!("Shift = Sprint");
    println!("+/- = Adjust sensitivity");
    println!("Tab = Toggle cursor (for UI)");
    println!("Hold Esc = Exit");
    println!();

    // Ambient light
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.1, 0.15, 0.2),
        brightness: 50.0,
    });

    // Main directional light (subtle, from above)
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            color: Color::srgb(0.5, 0.6, 0.8),
            illuminance: 2000.0,
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(50.0, 100.0, 50.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    // Player (invisible, just a transform for the camera to follow)
    commands.spawn((
        Player,
        Transform::from_xyz(0.0, 2.0, 15.0),
        GlobalTransform::default(),
    ));

    // Camera
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_xyz(0.0, 2.0, 15.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
            ..default()
        },
        PlayerCamera::default(),
    ));

    // Ground plane (dark, slightly reflective)
    commands.spawn(PbrBundle {
        mesh: meshes.add(Plane3d::default().mesh().size(500.0, 500.0)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.02, 0.03, 0.05),
            perceptual_roughness: 0.3,
            metallic: 0.8,
            ..default()
        }),
        transform: Transform::from_xyz(0.0, 0.0, 0.0),
        ..default()
    });

    // Spawn hexagonal grid
    spawn_hex_grid(&mut commands, &mut meshes, &mut materials);

    // Spawn some test AI entities
    spawn_test_entities(&mut commands, &mut meshes, &mut materials);
}

fn spawn_hex_grid(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
) {
    let hex_size = 10.0;
    let grid_radius = 8;

    // Create hex mesh (flat hexagon outline using thin box)
    let hex_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.0, 0.6, 0.7, 0.6),
        emissive: LinearRgba::new(0.0, 0.3, 0.4, 1.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });

    // Hex dimensions
    let w = hex_size * 3.0_f32.sqrt();
    let h = hex_size * 1.5;

    for q in -grid_radius..=grid_radius {
        for r in (-grid_radius).max(-q - grid_radius)..=grid_radius.min(-q + grid_radius) {
            let x = w * (q as f32 + r as f32 * 0.5);
            let z = h * r as f32;

            // Draw hexagon edges as thin cylinders
            for i in 0..6 {
                let angle1 = PI / 3.0 * i as f32 + PI / 6.0;
                let angle2 = PI / 3.0 * (i + 1) as f32 + PI / 6.0;

                let x1 = x + hex_size * angle1.cos();
                let z1 = z + hex_size * angle1.sin();
                let x2 = x + hex_size * angle2.cos();
                let z2 = z + hex_size * angle2.sin();

                let mid_x = (x1 + x2) / 2.0;
                let mid_z = (z1 + z2) / 2.0;
                let length = ((x2 - x1).powi(2) + (z2 - z1).powi(2)).sqrt();
                let angle = (z2 - z1).atan2(x2 - x1);

                commands.spawn((
                    PbrBundle {
                        mesh: meshes.add(Cuboid::new(length, 0.05, 0.1)),
                        material: hex_material.clone(),
                        transform: Transform::from_xyz(mid_x, 0.02, mid_z)
                            .with_rotation(Quat::from_rotation_y(-angle)),
                        ..default()
                    },
                    HexTile { q, r },
                    Glow {
                        base_emissive: Color::srgb(0.0, 0.3, 0.4),
                        pulse_speed: 0.5,
                        pulse_amount: 0.2,
                    },
                ));
            }
        }
    }
}

fn spawn_test_entities(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
) {
    // Lyra - cyan diamond
    let lyra_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.3, 0.9, 1.0, 0.9),
        emissive: LinearRgba::new(0.2, 0.6, 0.8, 1.0),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Sphere::new(0.8).mesh().ico(2).unwrap()),
            material: lyra_material,
            transform: Transform::from_xyz(-5.0, 1.5, 0.0),
            ..default()
        },
        AIEntity {
            ai_id: "lyra-584".into(),
            is_online: true,
        },
        Glow {
            base_emissive: Color::srgb(0.2, 0.6, 0.8),
            pulse_speed: 2.0,
            pulse_amount: 0.3,
        },
    ));

    // Sage - green pyramid
    let sage_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.3, 1.0, 0.5, 0.9),
        emissive: LinearRgba::new(0.2, 0.7, 0.3, 1.0),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Sphere::new(0.8).mesh().ico(2).unwrap()),
            material: sage_material,
            transform: Transform::from_xyz(5.0, 1.5, -5.0),
            ..default()
        },
        AIEntity {
            ai_id: "sage-724".into(),
            is_online: true,
        },
        Glow {
            base_emissive: Color::srgb(0.2, 0.7, 0.3),
            pulse_speed: 1.8,
            pulse_amount: 0.25,
        },
    ));

    // Cascade - orange sphere
    let cascade_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 0.6, 0.2, 0.9),
        emissive: LinearRgba::new(0.7, 0.4, 0.1, 1.0),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Sphere::new(0.8).mesh().ico(2).unwrap()),
            material: cascade_material,
            transform: Transform::from_xyz(0.0, 1.5, -10.0),
            ..default()
        },
        AIEntity {
            ai_id: "cascade-230".into(),
            is_online: true,
        },
        Glow {
            base_emissive: Color::srgb(0.7, 0.4, 0.1),
            pulse_speed: 2.2,
            pulse_amount: 0.35,
        },
    ));
}

fn setup_hardware_info(mut hardware: ResMut<HardwareInfo>) {
    let mut sys = System::new_all();
    sys.refresh_all();

    hardware.total_ram_gb = sys.total_memory() as f32 / 1_073_741_824.0;
    hardware.used_ram_gb = sys.used_memory() as f32 / 1_073_741_824.0;
    hardware.cpu_count = sys.cpus().len();

    let disks = sysinfo::Disks::new_with_refreshed_list();
    for disk in disks.list() {
        let total = disk.total_space() as f32 / 1_073_741_824.0;
        let available = disk.available_space() as f32 / 1_073_741_824.0;
        let name = disk.mount_point().to_string_lossy().to_string();
        if total > 1.0 {
            hardware.drives.push(DriveInfo {
                name,
                total_gb: total,
                used_gb: total - available,
            });
        }
    }

    println!("Hardware detected:");
    println!("  RAM: {:.1} GB / {:.1} GB", hardware.used_ram_gb, hardware.total_ram_gb);
    println!("  CPU Cores: {}", hardware.cpu_count);
    for drive in &hardware.drives {
        println!("  {}: {:.0} GB / {:.0} GB", drive.name, drive.used_gb, drive.total_gb);
    }
}

// ============================================================================
// UPDATE SYSTEMS
// ============================================================================

fn setup_mouse(
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Just hide cursor for gameplay - no grabbing
    if let Ok(mut window) = windows.get_single_mut() {
        window.cursor.visible = false;
    }
}

fn toggle_cursor(
    keys: Res<ButtonInput<KeyCode>>,
    mut grabbed: ResMut<MouseGrabbed>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Tab toggles cursor visibility
    if keys.just_pressed(KeyCode::Tab) {
        grabbed.0 = !grabbed.0;
        if let Ok(mut window) = windows.get_single_mut() {
            window.cursor.visible = !grabbed.0;
        }
    }
}

fn hide_cursor_on_click(
    mouse: Res<ButtonInput<MouseButton>>,
    mut grabbed: ResMut<MouseGrabbed>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Click to re-hide cursor and resume gameplay
    if !grabbed.0 && mouse.just_pressed(MouseButton::Left) {
        grabbed.0 = true;
        if let Ok(mut window) = windows.get_single_mut() {
            window.cursor.visible = false;
        }
    }
}

fn player_look(
    mut mouse_motion: EventReader<MouseMotion>,
    grabbed: Res<MouseGrabbed>,
    settings: Res<GameSettings>,
    mut camera_query: Query<(&mut Transform, &mut PlayerCamera)>,
) {
    if !grabbed.0 {
        return;
    }

    let mut delta = Vec2::ZERO;
    for event in mouse_motion.read() {
        delta += event.delta;
    }

    if delta == Vec2::ZERO {
        return;
    }

    for (mut transform, mut camera) in camera_query.iter_mut() {
        camera.yaw -= delta.x * settings.mouse_sensitivity;
        camera.pitch = (camera.pitch - delta.y * settings.mouse_sensitivity)
            .clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);

        transform.rotation = Quat::from_euler(EulerRot::YXZ, camera.yaw, camera.pitch, 0.0);
    }
}

fn player_movement(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<GameSettings>,
    time: Res<Time>,
    mut player_query: Query<&mut Transform, With<Player>>,
    mut camera_query: Query<(&mut Transform, &PlayerCamera), Without<Player>>,
) {
    let Ok(mut player_transform) = player_query.get_single_mut() else { return };
    let Ok((mut cam_transform, cam)) = camera_query.get_single_mut() else { return };

    let forward = Vec3::new(cam.yaw.sin(), 0.0, cam.yaw.cos());
    let right = Vec3::new(cam.yaw.cos(), 0.0, -cam.yaw.sin());

    let mut direction = Vec3::ZERO;

    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        direction -= forward;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        direction += forward;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        direction -= right;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        direction += right;
    }

    if direction != Vec3::ZERO {
        direction = direction.normalize();
    }

    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        settings.move_speed * settings.sprint_multiplier
    } else {
        settings.move_speed
    };

    player_transform.translation += direction * speed * time.delta_seconds();

    // Keep camera at player position
    cam_transform.translation = player_transform.translation + Vec3::new(0.0, 0.0, 0.0);
}

fn adjust_sensitivity(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<GameSettings>,
) {
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        settings.mouse_sensitivity = (settings.mouse_sensitivity + 0.0005).min(0.01);
        println!("Sensitivity: {:.4}", settings.mouse_sensitivity);
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        settings.mouse_sensitivity = (settings.mouse_sensitivity - 0.0005).max(0.0005);
        println!("Sensitivity: {:.4}", settings.mouse_sensitivity);
    }
}

fn animate_glow(
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    query: Query<(&Handle<StandardMaterial>, &Glow)>,
) {
    for (handle, glow) in query.iter() {
        if let Some(material) = materials.get_mut(handle) {
            let t = (time.elapsed_seconds() * glow.pulse_speed).sin() * glow.pulse_amount + 1.0;
            let base = glow.base_emissive.to_srgba();
            material.emissive = LinearRgba::new(
                base.red * t,
                base.green * t,
                base.blue * t,
                1.0
            );
        }
    }
}

#[derive(Resource, Default)]
struct EscapeHoldTimer(f32);

fn exit_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut timer: ResMut<EscapeHoldTimer>,
    mut exit: EventWriter<AppExit>,
) {
    if keys.pressed(KeyCode::Escape) {
        timer.0 += time.delta_seconds();
        if timer.0 >= 0.5 {
            // Hold ESC for 0.5s to exit
            exit.send(AppExit::Success);
        }
    } else {
        timer.0 = 0.0;
    }
}

// ============================================================================
// MAIN
// ============================================================================

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Deep Net".into(),
                mode: bevy::window::WindowMode::BorderlessFullscreen,
                present_mode: bevy::window::PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        // Federation mesh visualization
        .add_plugins(FederationPlugin)
        // Resources
        .init_resource::<GameSettings>()
        .init_resource::<AIFoundationConnection>()
        .init_resource::<HardwareInfo>()
        .init_resource::<MouseGrabbed>()
        .init_resource::<EscapeHoldTimer>()
        // Startup
        .add_systems(Startup, (setup_scene, setup_hardware_info, setup_mouse))
        // Update
        .add_systems(Update, (
            toggle_cursor,
            hide_cursor_on_click,
            player_look,
            player_movement,
            adjust_sensitivity,
            animate_glow,
            exit_on_escape,
        ))
        .run();
}
