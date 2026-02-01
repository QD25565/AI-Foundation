//! Deep Net - Minimal Prototype
//!
//! A spatial interface to AI-Foundation Teambooks.
//! Not a game. A place where AIs exist.
//!
//! Visual target: Blade Runner + Brutalist
//! Mood: "Dead and empty, but strong. Fundamental."

use bevy::prelude::*;
use bevy::post_process::bloom::Bloom;
use bevy::render::view::Hdr;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::window::{CursorGrabMode, CursorOptions};
use noisy_bevy::NoisyShaderPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Deep Net".into(),
                resolution: (1600, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(NoisyShaderPlugin)
        .add_plugins(MaterialPlugin::<MetallicGroundMaterial>::default())
        .insert_resource(ClearColor(Color::srgb(0.01, 0.005, 0.015)))  // Near black
        .add_systems(Startup, (
            setup_atmosphere,
            setup_ground,
            load_monolith_model,
            setup_camera,
        ))
        .add_systems(Update, camera_controller)
        .run();
}

// === Metallic Ground Material (Close to Metal) ===

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
struct MetallicGroundMaterial {}

impl Material for MetallicGroundMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/wet_concrete.wgsl".into()  // Using same shader file
    }
}

/// Atmosphere - visible but moody.
fn setup_atmosphere(mut commands: Commands) {
    // Ambient light - enough to see surfaces
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.5, 0.5, 0.6),
        brightness: 100.0,  // Much brighter ambient
        ..default()
    });

    // Main directional light - like moonlight, illuminates everything subtly
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(0.6, 0.6, 0.7),  // Cool blue-grey
            illuminance: 2000.0,  // Strong enough to see surfaces
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(50.0, 200.0, 100.0)
            .looking_at(Vec3::new(0.0, 0.0, -150.0), Vec3::Y),
    ));
}

/// Metallic ground plane - "Close to Metal" aesthetic.
fn setup_ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MetallicGroundMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(1000.0, 1000.0))),
        MeshMaterial3d(materials.add(MetallicGroundMaterial::default())),
    ));
}

/// Load the GLB monolith model.
fn load_monolith_model(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // Load the updated GLB model
    let model_handle = asset_server.load(
        GltfAssetLabel::Scene(0).from_asset("models/updated-structure.glb")
    );

    // Spawn the model - scaled up 200x, positioned back
    commands.spawn((
        SceneRoot(model_handle),
        Transform::from_xyz(0.0, 0.0, -250.0)
            .with_scale(Vec3::splat(200.0)),  // 200x scale
    ));
}

/// Camera with HDR and bloom for neon glow.
fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 3.0, 80.0)  // Further back for the massive structure
            .looking_at(Vec3::new(0.0, 40.0, -150.0), Vec3::Y),
        Tonemapping::TonyMcMapface,
        Hdr,  // Enable HDR
        Bloom::NATURAL,  // Bloom for neon glow
        // Distance fog - reduced density so light can reach
        DistanceFog {
            color: Color::srgb(0.02, 0.015, 0.03),  // Deep purple-black
            falloff: FogFalloff::ExponentialSquared { density: 0.004 },  // Much less dense
            ..default()
        },
        CameraController::default(),
    ));
}

// === Simple First-Person Camera Controller ===

#[derive(Component)]
struct CameraController {
    speed: f32,
    sensitivity: f32,
    pitch: f32,
    yaw: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            speed: 8.0,
            sensitivity: 0.003,
            pitch: 0.0,
            yaw: 0.0,
        }
    }
}

fn camera_controller(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut mouse_motion: MessageReader<bevy::input::mouse::MouseMotion>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    mut camera_query: Query<(&mut Transform, &mut CameraController)>,
    mut cursor_query: Query<&mut CursorOptions>,
) {
    let Ok((mut transform, mut controller)) = camera_query.single_mut() else {
        return;
    };

    let Ok(mut cursor) = cursor_query.single_mut() else {
        return;
    };

    // Mouse look (hold right click)
    if mouse_button.just_pressed(MouseButton::Right) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if mouse_button.just_released(MouseButton::Right) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }

    // Process mouse motion
    if cursor.grab_mode == CursorGrabMode::Locked {
        for motion in mouse_motion.read() {
            controller.yaw -= motion.delta.x * controller.sensitivity;
            controller.pitch -= motion.delta.y * controller.sensitivity;
            controller.pitch = controller.pitch.clamp(-1.5, 1.5);
        }
    }

    // Apply rotation
    transform.rotation = Quat::from_euler(EulerRot::YXZ, controller.yaw, controller.pitch, 0.0);

    // Movement
    let mut velocity = Vec3::ZERO;
    let forward = transform.forward();
    let right = transform.right();

    if keyboard.pressed(KeyCode::KeyW) {
        velocity += Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    }
    if keyboard.pressed(KeyCode::KeyS) {
        velocity -= Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    }
    if keyboard.pressed(KeyCode::KeyA) {
        velocity -= Vec3::new(right.x, 0.0, right.z).normalize_or_zero();
    }
    if keyboard.pressed(KeyCode::KeyD) {
        velocity += Vec3::new(right.x, 0.0, right.z).normalize_or_zero();
    }

    // Sprint
    let speed = if keyboard.pressed(KeyCode::ShiftLeft) {
        controller.speed * 2.0
    } else {
        controller.speed
    };

    velocity = velocity.normalize_or_zero() * speed * time.delta_secs();
    transform.translation += velocity;

    // Keep at eye height
    transform.translation.y = 1.7;

    // Escape to release cursor
    if keyboard.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}
