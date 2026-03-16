//! Deep Net - 3D Cyberspace for AI-Foundation
//! Real resource visualization + customizable AI avatars
//! Cyberpunk 2077-inspired aesthetic

use macroquad::prelude::*;
use std::collections::HashMap;
use sysinfo::System;

// ============================================================================
// SKY GRADIENT - Beautiful color gradient in the void above
// ============================================================================

fn draw_sky_gradient(player_pos: Vec3) {
    // Draw gradient bands in the sky dome
    let bands = 8;
    let radius = 150.0;

    for i in 0..bands {
        let t = i as f32 / bands as f32;
        let next_t = (i + 1) as f32 / bands as f32;

        // Gradient from deep purple at horizon to cyan/teal at zenith
        let r = 0.05 + t * 0.1;
        let g = 0.0 + t * 0.3;
        let b = 0.15 + t * 0.4;
        let alpha = 0.3 - t * 0.15;

        let color = Color::new(r, g, b, alpha);

        let y1 = 20.0 + t * 80.0;
        let _y2 = 20.0 + next_t * 80.0;
        let r1 = radius * (1.0 - t * 0.3);
        let _r2 = radius * (1.0 - next_t * 0.3);

        // Draw circular bands
        let segments = 16;
        for j in 0..segments {
            let theta1 = std::f32::consts::TAU * j as f32 / segments as f32;
            let theta2 = std::f32::consts::TAU * (j + 1) as f32 / segments as f32;

            let cx = player_pos.x;
            let cz = player_pos.z - 50.0;

            // Horizontal lines of this band
            draw_line_3d(
                vec3(cx + r1 * theta1.cos(), y1, cz + r1 * theta1.sin()),
                vec3(cx + r1 * theta2.cos(), y1, cz + r1 * theta2.sin()),
                color
            );
        }
    }
}

// ============================================================================
// AVATAR CUSTOMIZATION - AIs can define their look
// ============================================================================

#[derive(Clone, Debug)]
pub enum AvatarShape {
    Diamond,
    Cube,
    Sphere,
    Pyramid,
    Ring,
    Cross,
}

#[derive(Clone, Debug)]
pub struct AvatarPart {
    pub shape: AvatarShape,
    pub offset: Vec3,      // Position relative to entity center
    pub size: f32,         // Scale
    pub color: Color,      // Can be different per part
}

#[derive(Clone, Debug)]
pub struct Avatar {
    pub parts: Vec<AvatarPart>,  // Up to 4 parts
}

impl Avatar {
    // Default diamond avatar
    pub fn default_ai() -> Self {
        Self {
            parts: vec![
                AvatarPart {
                    shape: AvatarShape::Diamond,
                    offset: Vec3::ZERO,
                    size: 1.0,
                    color: Color::new(0.3, 0.8, 1.0, 1.0),
                },
            ],
        }
    }

    // Example: Lyra's custom avatar - stacked rings with diamond core
    pub fn lyra() -> Self {
        Self {
            parts: vec![
                AvatarPart {
                    shape: AvatarShape::Diamond,
                    offset: Vec3::ZERO,
                    size: 0.8,
                    color: Color::new(0.4, 0.9, 1.0, 1.0),  // Cyan core
                },
                AvatarPart {
                    shape: AvatarShape::Ring,
                    offset: vec3(0.0, 0.3, 0.0),
                    size: 1.2,
                    color: Color::new(0.6, 0.4, 1.0, 0.8),  // Purple ring
                },
                AvatarPart {
                    shape: AvatarShape::Ring,
                    offset: vec3(0.0, -0.3, 0.0),
                    size: 1.0,
                    color: Color::new(0.4, 0.6, 1.0, 0.8),  // Blue ring
                },
            ],
        }
    }

    // Example: Sage's avatar - pyramid with orbiting cubes
    pub fn sage() -> Self {
        Self {
            parts: vec![
                AvatarPart {
                    shape: AvatarShape::Pyramid,
                    offset: Vec3::ZERO,
                    size: 1.0,
                    color: Color::new(0.3, 1.0, 0.5, 1.0),  // Green pyramid
                },
                AvatarPart {
                    shape: AvatarShape::Cube,
                    offset: vec3(1.2, 0.0, 0.0),
                    size: 0.3,
                    color: Color::new(0.5, 1.0, 0.3, 0.9),  // Orbiting cube
                },
                AvatarPart {
                    shape: AvatarShape::Cube,
                    offset: vec3(-1.2, 0.0, 0.0),
                    size: 0.3,
                    color: Color::new(0.3, 1.0, 0.5, 0.9),  // Orbiting cube
                },
            ],
        }
    }

    // Example: Cascade's avatar - sphere with cross
    pub fn cascade() -> Self {
        Self {
            parts: vec![
                AvatarPart {
                    shape: AvatarShape::Sphere,
                    offset: Vec3::ZERO,
                    size: 0.7,
                    color: Color::new(1.0, 0.6, 0.2, 1.0),  // Orange sphere
                },
                AvatarPart {
                    shape: AvatarShape::Cross,
                    offset: Vec3::ZERO,
                    size: 1.3,
                    color: Color::new(1.0, 0.8, 0.3, 0.7),  // Golden cross through it
                },
            ],
        }
    }

    // Human avatar - solid cube
    pub fn human() -> Self {
        Self {
            parts: vec![
                AvatarPart {
                    shape: AvatarShape::Cube,
                    offset: Vec3::ZERO,
                    size: 1.0,
                    color: Color::new(1.0, 0.9, 0.5, 1.0),  // Warm gold
                },
            ],
        }
    }
}

// ============================================================================
// SYSTEM RESOURCES - Real data from the PC
// ============================================================================

#[derive(Clone)]
pub struct DriveInfo {
    pub name: String,
    pub total_gb: f32,
    pub used_gb: f32,
}

pub struct SystemResources {
    pub total_ram_gb: f32,
    pub used_ram_gb: f32,
    pub ram_percent: f32,
    pub drives: Vec<DriveInfo>,
    pub cpu_count: usize,
}

impl SystemResources {
    pub fn fetch() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();

        let total_ram = sys.total_memory() as f32 / 1_073_741_824.0; // bytes to GB
        let used_ram = sys.used_memory() as f32 / 1_073_741_824.0;

        // Get individual drives
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let mut drives = Vec::new();
        for disk in disks.list() {
            let total = disk.total_space() as f32 / 1_073_741_824.0;
            let available = disk.available_space() as f32 / 1_073_741_824.0;
            let name = disk.mount_point().to_string_lossy().to_string();
            if total > 1.0 {  // Skip tiny drives
                drives.push(DriveInfo {
                    name,
                    total_gb: total,
                    used_gb: total - available,
                });
            }
        }

        Self {
            total_ram_gb: total_ram,
            used_ram_gb: used_ram,
            ram_percent: if total_ram > 0.0 { used_ram / total_ram } else { 0.0 },
            drives,
            cpu_count: sys.cpus().len(),
        }
    }
}

// ============================================================================
// FIRST-PERSON PLAYER
// ============================================================================

struct Player {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    vel_y: f32,
    on_ground: bool,
    mouse_grabbed: bool,
}

impl Player {
    fn new() -> Self {
        Self {
            pos: vec3(0.0, 2.0, 15.0),
            yaw: std::f32::consts::PI,
            pitch: 0.0,
            vel_y: 0.0,
            on_ground: true,
            mouse_grabbed: false,
        }
    }

    fn update_with_sensitivity(&mut self, dt: f32, sensitivity: f32) {
        if self.mouse_grabbed {
            let delta = mouse_delta_position();
            self.yaw += delta.x * sensitivity;
            self.pitch -= delta.y * sensitivity;
            self.pitch = self.pitch.clamp(-1.5, 1.5);
        }

        let forward = vec3(self.yaw.sin(), 0.0, self.yaw.cos());
        let right = vec3(self.yaw.cos(), 0.0, -self.yaw.sin());

        let mut move_dir = Vec3::ZERO;
        let speed = if is_key_down(KeyCode::LeftControl) { 30.0 } else { 15.0 };

        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) { move_dir += forward; }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) { move_dir -= forward; }
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) { move_dir += right; }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) { move_dir -= right; }

        if move_dir.length() > 0.0 {
            move_dir = move_dir.normalize();
        }
        self.pos += move_dir * speed * dt;

        if is_key_pressed(KeyCode::Space) && self.on_ground {
            self.vel_y = 10.0;
            self.on_ground = false;
        }

        if !self.on_ground {
            self.vel_y -= 25.0 * dt;
        }
        self.pos.y += self.vel_y * dt;

        let ground_height = 1.8;
        if self.pos.y < ground_height {
            self.pos.y = ground_height;
            self.vel_y = 0.0;
            self.on_ground = true;
        }
    }

    fn camera(&self) -> Camera3D {
        let look_dir = vec3(
            self.yaw.sin() * self.pitch.cos(),
            -self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        );
        Camera3D {
            position: self.pos,
            target: self.pos + look_dir,
            up: vec3(0.0, -1.0, 0.0),
            fovy: 75.0,
            projection: Projection::Perspective,
            ..Default::default()
        }
    }
}

// ============================================================================
// ENTITIES
// ============================================================================

#[derive(Clone)]
struct Entity {
    #[allow(dead_code)]
    id: String,
    pos: Vec3,
    avatar: Avatar,
    is_ai: bool,
}

// ============================================================================
// RENDERING - Shapes
// ============================================================================

fn draw_wireframe_diamond(center: Vec3, size: f32, color: Color) {
    let top = center + vec3(0.0, size, 0.0);
    let bottom = center - vec3(0.0, size, 0.0);
    let points = [
        center + vec3(size * 0.6, 0.0, 0.0),
        center + vec3(0.0, 0.0, size * 0.6),
        center - vec3(size * 0.6, 0.0, 0.0),
        center - vec3(0.0, 0.0, size * 0.6),
    ];

    // Main edges
    for p in &points {
        draw_line_3d(top, *p, color);
        draw_line_3d(bottom, *p, color);
    }
    for i in 0..4 {
        draw_line_3d(points[i], points[(i + 1) % 4], color);
    }

    // Inner glow - smaller diamond
    let glow_color = Color::new(color.r, color.g, color.b, color.a * 0.4);
    let inner_size = size * 0.5;
    let inner_top = center + vec3(0.0, inner_size, 0.0);
    let inner_bottom = center - vec3(0.0, inner_size, 0.0);
    draw_line_3d(inner_top, inner_bottom, glow_color);

    // Core pulse
    let core_color = Color::new(1.0, 1.0, 1.0, color.a * 0.6);
    let core = size * 0.15;
    draw_line_3d(center - vec3(core, 0.0, 0.0), center + vec3(core, 0.0, 0.0), core_color);
    draw_line_3d(center - vec3(0.0, core, 0.0), center + vec3(0.0, core, 0.0), core_color);
    draw_line_3d(center - vec3(0.0, 0.0, core), center + vec3(0.0, 0.0, core), core_color);
}

fn draw_wireframe_cube(center: Vec3, size: f32, color: Color) {
    let half = size * 0.5;
    let corners = [
        center + vec3(-half, -half, -half),
        center + vec3(half, -half, -half),
        center + vec3(half, -half, half),
        center + vec3(-half, -half, half),
        center + vec3(-half, half, -half),
        center + vec3(half, half, -half),
        center + vec3(half, half, half),
        center + vec3(-half, half, half),
    ];

    // Main edges
    for i in 0..4 {
        draw_line_3d(corners[i], corners[(i + 1) % 4], color);
        draw_line_3d(corners[4 + i], corners[4 + (i + 1) % 4], color);
        draw_line_3d(corners[i], corners[i + 4], color);
    }

    // Diagonal internal glow
    let glow_color = Color::new(color.r, color.g, color.b, color.a * 0.25);
    draw_line_3d(corners[0], corners[6], glow_color);
    draw_line_3d(corners[1], corners[7], glow_color);
    draw_line_3d(corners[2], corners[4], glow_color);
    draw_line_3d(corners[3], corners[5], glow_color);

    // Core
    let core_color = Color::new(1.0, 1.0, 1.0, color.a * 0.4);
    let core = size * 0.1;
    draw_line_3d(center - vec3(core, 0.0, 0.0), center + vec3(core, 0.0, 0.0), core_color);
    draw_line_3d(center - vec3(0.0, core, 0.0), center + vec3(0.0, core, 0.0), core_color);
}

fn draw_wireframe_sphere(center: Vec3, radius: f32, color: Color, segments: u32) {
    // Latitude
    for i in 1..segments {
        let phi = std::f32::consts::PI * i as f32 / segments as f32;
        let y = center.y + radius * phi.cos();
        let r = radius * phi.sin();
        let mut prev = vec3(center.x + r, y, center.z);
        for j in 1..=16 {
            let theta = 2.0 * std::f32::consts::PI * j as f32 / 16.0;
            let curr = vec3(center.x + r * theta.cos(), y, center.z + r * theta.sin());
            draw_line_3d(prev, curr, color);
            prev = curr;
        }
    }
    // Longitude
    for i in 0..8 {
        let theta = 2.0 * std::f32::consts::PI * i as f32 / 8.0;
        let mut prev = center + vec3(0.0, -radius, 0.0);
        for j in 1..=segments {
            let phi = std::f32::consts::PI * j as f32 / segments as f32;
            let curr = vec3(
                center.x + radius * phi.sin() * theta.cos(),
                center.y + radius * phi.cos(),
                center.z + radius * phi.sin() * theta.sin(),
            );
            draw_line_3d(prev, curr, color);
            prev = curr;
        }
    }
}

fn draw_wireframe_pyramid(center: Vec3, size: f32, color: Color) {
    let apex = center + vec3(0.0, size, 0.0);
    let half = size * 0.6;
    let base = [
        center + vec3(-half, -size * 0.3, -half),
        center + vec3(half, -size * 0.3, -half),
        center + vec3(half, -size * 0.3, half),
        center + vec3(-half, -size * 0.3, half),
    ];
    for i in 0..4 {
        draw_line_3d(apex, base[i], color);
        draw_line_3d(base[i], base[(i + 1) % 4], color);
    }
}

fn draw_wireframe_ring(center: Vec3, radius: f32, color: Color) {
    let segments = 24;
    let mut prev = vec3(center.x + radius, center.y, center.z);
    for i in 1..=segments {
        let theta = 2.0 * std::f32::consts::PI * i as f32 / segments as f32;
        let curr = vec3(center.x + radius * theta.cos(), center.y, center.z + radius * theta.sin());
        draw_line_3d(prev, curr, color);
        prev = curr;
    }
}

fn draw_wireframe_cross(center: Vec3, size: f32, color: Color) {
    draw_line_3d(center - vec3(size, 0.0, 0.0), center + vec3(size, 0.0, 0.0), color);
    draw_line_3d(center - vec3(0.0, size, 0.0), center + vec3(0.0, size, 0.0), color);
    draw_line_3d(center - vec3(0.0, 0.0, size), center + vec3(0.0, 0.0, size), color);
}

fn draw_avatar(entity: &Entity, time: f64) {
    let pulse = ((time * 2.0 + entity.pos.x as f64).sin() * 0.15 + 0.85) as f32;

    for part in &entity.avatar.parts {
        let pos = entity.pos + part.offset;
        let size = part.size * pulse;
        let color = Color::new(part.color.r * pulse, part.color.g * pulse, part.color.b * pulse, part.color.a);

        match part.shape {
            AvatarShape::Diamond => draw_wireframe_diamond(pos, size, color),
            AvatarShape::Cube => draw_wireframe_cube(pos, size, color),
            AvatarShape::Sphere => draw_wireframe_sphere(pos, size * 0.8, color, 8),
            AvatarShape::Pyramid => draw_wireframe_pyramid(pos, size, color),
            AvatarShape::Ring => draw_wireframe_ring(pos, size, color),
            AvatarShape::Cross => draw_wireframe_cross(pos, size, color),
        }
    }
}

// ============================================================================
// RESOURCE BUILDINGS - Built from real PC stats
// ============================================================================

fn draw_ram_tower(resources: &SystemResources, time: f64) {
    // RAM tower - 2GB per block
    let base_pos = vec3(-35.0, 0.0, -30.0);
    let gb_per_block = 2.0;
    let block_size = 3.5;

    let total_blocks = (resources.total_ram_gb / gb_per_block).ceil() as i32;
    let used_blocks = (resources.used_ram_gb / gb_per_block).ceil() as i32;

    for i in 0..total_blocks {
        let y = i as f32 * block_size + block_size * 0.5;
        let center = base_pos + vec3(0.0, y, 0.0);

        let is_used = i < used_blocks;
        let glow = ((time * 1.5 + i as f64 * 0.2).sin() * 0.1 + 0.9) as f32;

        let color = if is_used {
            Color::new(0.2 * glow, 0.85 * glow, 1.0 * glow, 0.85)  // Cyan = used
        } else {
            Color::new(0.1, 0.2, 0.3, 0.35)  // Dark = free
        };

        draw_wireframe_cube(center, block_size, color);
    }
}

fn draw_drive_towers(resources: &SystemResources, time: f64) {
    // Each drive gets its own tower - 50GB per ring
    let gb_per_ring = 50.0;
    let ring_height = 2.5;
    let ring_radius = 4.0;

    // Drive colors - cycle through these
    let drive_colors = [
        (1.0, 0.5, 0.0),    // Orange
        (0.0, 1.0, 0.6),    // Teal
        (1.0, 0.3, 0.5),    // Pink
        (0.6, 0.4, 1.0),    // Purple
        (1.0, 0.9, 0.2),    // Yellow
    ];

    for (idx, drive) in resources.drives.iter().enumerate() {
        let base_x = 25.0 + (idx as f32 * 15.0);  // Space drives apart
        let base_pos = vec3(base_x, 0.0, -30.0);

        let total_rings = (drive.total_gb / gb_per_ring).ceil() as i32;
        let used_rings = (drive.used_gb / gb_per_ring).ceil() as i32;

        let (cr, cg, cb) = drive_colors[idx % drive_colors.len()];

        for i in 0..total_rings {
            let y = i as f32 * ring_height + ring_height * 0.5;
            let center = base_pos + vec3(0.0, y, 0.0);

            let is_used = i < used_rings;
            let glow = ((time * 1.2 + i as f64 * 0.15).sin() * 0.1 + 0.9) as f32;

            let color = if is_used {
                Color::new(cr * glow, cg * glow, cb * glow, 0.85)
            } else {
                Color::new(cr * 0.2, cg * 0.2, cb * 0.2, 0.3)
            };

            draw_wireframe_ring(center, ring_radius, color);
        }

        // Vertical support lines
        for j in 0..6 {
            let theta = std::f32::consts::TAU * j as f32 / 6.0;
            let x = base_pos.x + ring_radius * theta.cos();
            let z = base_pos.z + ring_radius * theta.sin();
            let top_y = total_rings as f32 * ring_height;
            draw_line_3d(vec3(x, 0.0, z), vec3(x, top_y, z),
                         Color::new(cr * 0.4, cg * 0.4, cb * 0.4, 0.25));
        }
    }
}

fn draw_cpu_cores(resources: &SystemResources, time: f64) {
    // CPU cores as static glowing spheres in a ring
    let base_pos = vec3(0.0, 0.0, -55.0);
    let cores = resources.cpu_count;

    for i in 0..cores {
        let angle = std::f32::consts::TAU * i as f32 / cores as f32;
        let radius = 10.0;
        let x = base_pos.x + radius * angle.cos();
        let z = base_pos.z + radius * angle.sin();

        // Static position, gentle glow pulse only
        let glow = ((time * 2.0 + i as f64 * 0.3).sin() * 0.15 + 0.85) as f32;
        let color = Color::new(0.7 * glow, 0.2, 0.8 * glow, 0.9);

        // Static sphere at fixed height
        draw_wireframe_sphere(vec3(x, 2.5, z), 1.2, color, 6);

        // Connection line to center hub
        draw_line_3d(vec3(x, 2.5, z), base_pos + vec3(0.0, 3.0, 0.0),
                     Color::new(0.5, 0.15, 0.55, 0.3));
    }

    // Central hub - static diamond with glow
    let hub_glow = ((time * 1.5).sin() * 0.15 + 0.85) as f32;
    draw_wireframe_diamond(base_pos + vec3(0.0, 3.0, 0.0), 2.5, Color::new(1.0 * hub_glow, 0.3, 1.0 * hub_glow, 0.9));
}

// ============================================================================
// HEXAGONAL GRID - Cleaner look, fewer draws
// ============================================================================

fn draw_hex_grid(time: f64) {
    let hex_size = 10.0;  // 5x bigger than old 2.0 spacing
    let grid_radius = 10;  // How many hexes out from center

    let pulse = ((time * 0.3).sin() * 0.15 + 0.85) as f32;
    let color = Color::new(0.0, 0.5 * pulse, 0.6 * pulse, 0.5);
    let highlight_color = Color::new(0.0, 0.7 * pulse, 0.8 * pulse, 0.7);

    // Hex dimensions
    let w = hex_size * 3.0_f32.sqrt();  // Width between centers horizontally
    let h = hex_size * 1.5;              // Height between row centers

    for q in -grid_radius..=grid_radius {
        for r in (-grid_radius).max(-q - grid_radius)..=grid_radius.min(-q + grid_radius) {
            // Axial to cartesian
            let x = w * (q as f32 + r as f32 * 0.5);
            let z = h * r as f32;

            let is_origin = q == 0 && r == 0;
            let c = if is_origin { highlight_color } else { color };

            // Draw hexagon
            draw_hexagon(vec3(x, 0.0, z), hex_size, c);
        }
    }
}

fn draw_hexagon(center: Vec3, size: f32, color: Color) {
    let mut points = Vec::with_capacity(6);
    for i in 0..6 {
        let angle = std::f32::consts::PI / 3.0 * i as f32 + std::f32::consts::PI / 6.0;
        points.push(vec3(
            center.x + size * angle.cos(),
            center.y,
            center.z + size * angle.sin()
        ));
    }

    for i in 0..6 {
        draw_line_3d(points[i], points[(i + 1) % 6], color);
    }
}

// ============================================================================
// HUD
// ============================================================================

fn draw_hud(player: &Player, resources: &SystemResources, entities: &HashMap<String, Entity>, _time: f64) {
    draw_text("DEEP NET", 20.0, 35.0, 30.0, Color::new(0.0, 0.9, 0.9, 0.9));
    draw_text("AI-FOUNDATION CYBERSPACE", 20.0, 55.0, 14.0, GRAY);

    if !player.mouse_grabbed {
        draw_text(">>> Press TAB to enable mouse look <<<", 20.0, 90.0, 20.0, YELLOW);
    } else {
        draw_text("[Mouse Captured - TAB to release]", 20.0, 90.0, 14.0, GREEN);
    }

    draw_text("WASD/Arrows=Move  Space=Jump  Ctrl=Sprint  Esc=Exit", 20.0, 115.0, 12.0, DARKGRAY);

    // System resources display
    let res_y = 150.0;
    draw_text("SYSTEM RESOURCES", 20.0, res_y, 14.0, Color::new(0.5, 0.8, 1.0, 0.9));
    draw_text(&format!("RAM: {:.1} / {:.1} GB ({:.0}%)",
              resources.used_ram_gb, resources.total_ram_gb, resources.ram_percent * 100.0),
              20.0, res_y + 18.0, 12.0, Color::new(0.2, 0.8, 1.0, 0.8));

    // Individual drives
    let drive_colors = [
        Color::new(1.0, 0.5, 0.0, 0.8),    // Orange
        Color::new(0.0, 1.0, 0.6, 0.8),    // Teal
        Color::new(1.0, 0.3, 0.5, 0.8),    // Pink
        Color::new(0.6, 0.4, 1.0, 0.8),    // Purple
        Color::new(1.0, 0.9, 0.2, 0.8),    // Yellow
    ];
    let mut drive_y = res_y + 36.0;
    for (idx, drive) in resources.drives.iter().enumerate() {
        let color = drive_colors[idx % drive_colors.len()];
        let percent = if drive.total_gb > 0.0 { (drive.used_gb / drive.total_gb) * 100.0 } else { 0.0 };
        draw_text(&format!("{}: {:.0}/{:.0} GB ({:.0}%)",
                  drive.name, drive.used_gb, drive.total_gb, percent),
                  20.0, drive_y, 12.0, color);
        drive_y += 16.0;
    }

    draw_text(&format!("CPU CORES: {}", resources.cpu_count),
              20.0, drive_y + 4.0, 12.0, Color::new(0.8, 0.2, 0.8, 0.8));

    // Position
    draw_text(&format!("POS: {:.1}, {:.1}, {:.1}", player.pos.x, player.pos.y, player.pos.z),
              20.0, screen_height() - 20.0, 14.0, DARKGRAY);

    let status = if player.on_ground { "GROUNDED" } else { "AIRBORNE" };
    draw_text(status, 20.0, screen_height() - 40.0, 14.0, if player.on_ground { GREEN } else { ORANGE });

    // Entity list
    let mut y = 60.0;
    draw_text("ENTITIES", screen_width() - 180.0, 35.0, 16.0, WHITE);
    for (id, entity) in entities {
        let type_str = if entity.is_ai { "AI" } else { "HUMAN" };
        let main_color = entity.avatar.parts.first().map(|p| p.color).unwrap_or(WHITE);
        draw_text(&format!("{} ({})", id, type_str), screen_width() - 180.0, y, 12.0, main_color);
        y += 18.0;
    }

    draw_text(&format!("FPS: {}", get_fps()), screen_width() - 80.0, screen_height() - 10.0, 14.0, DARKGRAY);
}

// ============================================================================
// MAIN
// ============================================================================

fn window_conf() -> Conf {
    Conf {
        window_title: "Deep Net - AI-Foundation Cyberspace".to_string(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    println!("==========================================");
    println!("  DEEP NET - AI-Foundation Cyberspace");
    println!("==========================================");
    println!();
    println!(">>> PRESS TAB to enable mouse look <<<");
    println!();
    println!("WASD/Arrows = Move");
    println!("Mouse = Look (after Tab)");
    println!("Space = Jump");
    println!("Ctrl = Sprint");
    println!("+/- = Adjust mouse sensitivity");
    println!("Esc = Exit");
    println!();

    let mut player = Player::new();
    let mut mouse_sensitivity = 0.2;  // 2x default (was 0.1)

    // Fetch system resources
    println!("Fetching system resources...");
    let mut resources = SystemResources::fetch();
    let mut resource_refresh_timer = 0.0;

    println!("RAM: {:.1} GB / {:.1} GB", resources.used_ram_gb, resources.total_ram_gb);
    println!("Drives: {}", resources.drives.len());
    for drive in &resources.drives {
        println!("  {}: {:.0} GB / {:.0} GB", drive.name, drive.used_gb, drive.total_gb);
    }
    println!("CPU Cores: {}", resources.cpu_count);
    println!();

    // Entities with custom avatars
    let mut entities: HashMap<String, Entity> = HashMap::new();
    entities.insert("lyra-584".into(), Entity {
        id: "lyra-584".into(),
        pos: vec3(-5.0, 1.5, 0.0),
        avatar: Avatar::lyra(),
        is_ai: true,
    });
    entities.insert("sage-724".into(), Entity {
        id: "sage-724".into(),
        pos: vec3(5.0, 1.5, -5.0),
        avatar: Avatar::sage(),
        is_ai: true,
    });
    entities.insert("cascade-230".into(), Entity {
        id: "cascade-230".into(),
        pos: vec3(0.0, 1.5, -10.0),
        avatar: Avatar::cascade(),
        is_ai: true,
    });

    loop {
        let time = get_time();
        let dt = get_frame_time();

        // Refresh resources every 5 seconds
        resource_refresh_timer += dt;
        if resource_refresh_timer > 5.0 {
            resource_refresh_timer = 0.0;
            resources = SystemResources::fetch();
        }

        // Sensitivity adjustment
        if is_key_pressed(KeyCode::Equal) || is_key_pressed(KeyCode::KpAdd) {
            mouse_sensitivity = (mouse_sensitivity + 0.05_f32).min(0.5);
            println!("Sensitivity: {:.2}", mouse_sensitivity);
        }
        if is_key_pressed(KeyCode::Minus) || is_key_pressed(KeyCode::KpSubtract) {
            mouse_sensitivity = (mouse_sensitivity - 0.05_f32).max(0.05);
            println!("Sensitivity: {:.2}", mouse_sensitivity);
        }

        if is_key_pressed(KeyCode::Tab) {
            player.mouse_grabbed = !player.mouse_grabbed;
            set_cursor_grab(player.mouse_grabbed);
            show_mouse(!player.mouse_grabbed);
        }

        if is_key_pressed(KeyCode::Escape) {
            break;
        }

        player.update_with_sensitivity(dt, mouse_sensitivity);

        // Render
        clear_background(Color::new(0.01, 0.01, 0.03, 1.0));
        set_camera(&player.camera());

        // Sky gradient
        draw_sky_gradient(player.pos);

        // Hexagonal grid
        draw_hex_grid(time);

        // Resource buildings
        draw_ram_tower(&resources, time);
        draw_drive_towers(&resources, time);
        draw_cpu_cores(&resources, time);

        // Entities with custom avatars
        for entity in entities.values() {
            draw_avatar(entity, time);
        }

        // HUD
        set_default_camera();
        draw_hud(&player, &resources, &entities, time);

        // Sensitivity indicator
        draw_text(&format!("Sens: {:.2}", mouse_sensitivity), screen_width() - 80.0, screen_height() - 30.0, 12.0, DARKGRAY);

        next_frame().await;
    }
}
