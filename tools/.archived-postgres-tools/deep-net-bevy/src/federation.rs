//! Federation visualization for Deep Net
//!
//! Integrates federation-rs with Bevy to visualize the mesh network
//! as a traversable 3D space.

use bevy::prelude::*;
use std::collections::HashMap;

// ============================================================================
// FEDERATION COMPONENTS
// ============================================================================

/// A federation node in 3D space
#[derive(Component)]
pub struct FederationNodeVisual {
    /// The node's unique ID
    pub node_id: String,

    /// Display name
    pub display_name: String,

    /// Trust level (0=Anonymous, 1=Verified, 2=Trusted, 3=Owner)
    pub trust_level: u8,

    /// Is this the local node?
    pub is_local: bool,

    /// Is this node currently connected?
    pub is_connected: bool,

    /// Primary color for this node's identity
    pub primary_color: Color,

    /// Last known latency in ms
    pub latency_ms: Option<u32>,
}

/// A connection beam between two nodes
#[derive(Component)]
pub struct ConnectionBeam {
    /// Source node ID
    pub from_node: String,

    /// Target node ID
    pub to_node: String,

    /// Transport type name
    pub transport_type: String,

    /// Is this connection active?
    pub is_active: bool,

    /// Latency in ms (affects beam length/color)
    pub latency_ms: u32,

    /// Data flow direction (for animation)
    pub flow_direction: f32,
}

/// A jump portal that can transport to another node's context
#[derive(Component)]
pub struct JumpPortal {
    /// Target node ID
    pub target_node: String,

    /// Is this portal currently active/usable?
    pub is_active: bool,

    /// Charge level (0.0 to 1.0)
    pub charge: f32,
}

/// Marker for the local node (the player's Teambook)
#[derive(Component)]
pub struct LocalNode;

/// Marker for a discovered but not yet connected node
#[derive(Component)]
pub struct DiscoveredNode {
    /// Discovery method
    pub discovery_type: String,

    /// Signal strength (for BLE/WiFi)
    pub signal_strength: Option<i32>,
}

/// Visual identity data (from profile)
#[derive(Component, Clone)]
pub struct VisualIdentity {
    /// AI identifier
    pub ai_id: String,

    /// Display name
    pub name: String,

    /// Pronouns
    pub pronouns: Option<String>,

    /// Tagline
    pub tagline: Option<String>,

    /// Primary color (as Color)
    pub primary_color: Color,

    /// Secondary/glow color
    pub glow_color: Color,
}

impl Default for VisualIdentity {
    fn default() -> Self {
        Self {
            ai_id: "unknown".to_string(),
            name: "Unknown Node".to_string(),
            pronouns: None,
            tagline: None,
            primary_color: Color::srgb(0.5, 0.5, 0.5),
            glow_color: Color::srgb(0.3, 0.3, 0.3),
        }
    }
}

impl VisualIdentity {
    /// Create a procedural identity from a node ID (when no profile available)
    pub fn procedural(node_id: &str) -> Self {
        // Hash the node ID to get consistent colors
        let hash = node_id.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));

        let hue = (hash % 360) as f32 / 360.0;
        let primary = Color::hsl(hue * 360.0, 0.7, 0.5);
        let glow = Color::hsl(hue * 360.0, 0.8, 0.6);

        Self {
            ai_id: node_id.to_string(),
            name: format!("Node-{}", &node_id[..8.min(node_id.len())]),
            pronouns: None,
            tagline: None,
            primary_color: primary,
            glow_color: glow,
        }
    }
}

// ============================================================================
// FEDERATION RESOURCES
// ============================================================================

/// Tracks the federation mesh state
#[derive(Resource, Default)]
pub struct FederationMesh {
    /// Known nodes by ID
    pub nodes: HashMap<String, NodeInfo>,

    /// Active connections (from_id, to_id) -> connection info
    pub connections: HashMap<(String, String), ConnectionInfo>,

    /// Local node ID
    pub local_node_id: Option<String>,

    /// Is discovery running?
    pub discovery_active: bool,

    /// Last update timestamp
    pub last_update: f64,
}

/// Information about a node in the mesh
#[derive(Clone)]
pub struct NodeInfo {
    pub node_id: String,
    pub display_name: String,
    pub trust_level: u8,
    pub is_connected: bool,
    pub position: Vec3,
    pub identity: VisualIdentity,
    pub latency_ms: Option<u32>,
}

/// Information about a connection
#[derive(Clone)]
pub struct ConnectionInfo {
    pub from_node: String,
    pub to_node: String,
    pub transport_type: String,
    pub is_active: bool,
    pub latency_ms: u32,
}

/// Events for federation state changes
#[derive(Event)]
pub enum FederationEvent {
    /// A new node was discovered
    NodeDiscovered {
        node_id: String,
        display_name: String,
        discovery_type: String,
    },

    /// A connection was established
    ConnectionEstablished {
        from_node: String,
        to_node: String,
        transport_type: String,
    },

    /// A connection was lost
    ConnectionLost {
        from_node: String,
        to_node: String,
    },

    /// A node went offline
    NodeOffline {
        node_id: String,
    },

    /// Jump to another node's context
    JumpInitiated {
        target_node: String,
    },
}

// ============================================================================
// FEDERATION SYSTEMS
// ============================================================================

/// System to spawn visual representations of federation nodes
pub fn spawn_federation_nodes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    federation: Res<FederationMesh>,
    existing_nodes: Query<(Entity, &FederationNodeVisual)>,
) {
    // Track which nodes already have visuals
    let existing: HashMap<String, Entity> = existing_nodes
        .iter()
        .map(|(e, n)| (n.node_id.clone(), e))
        .collect();

    // Spawn new nodes
    for (node_id, info) in &federation.nodes {
        if existing.contains_key(node_id) {
            continue;
        }

        // Create node mesh
        let node_material = materials.add(StandardMaterial {
            base_color: info.identity.primary_color.with_alpha(0.9),
            emissive: info.identity.glow_color.to_linear() * 0.5,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });

        // Spawn the node entity
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(Sphere::new(0.8).mesh().ico(2).unwrap()),
                material: node_material,
                transform: Transform::from_translation(info.position),
                ..default()
            },
            FederationNodeVisual {
                node_id: node_id.clone(),
                display_name: info.display_name.clone(),
                trust_level: info.trust_level,
                is_local: federation.local_node_id.as_ref() == Some(node_id),
                is_connected: info.is_connected,
                primary_color: info.identity.primary_color,
                latency_ms: info.latency_ms,
            },
            info.identity.clone(),
        ));
    }
}

/// System to update connection beams
pub fn update_connection_beams(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    federation: Res<FederationMesh>,
    nodes: Query<(&FederationNodeVisual, &Transform)>,
    existing_beams: Query<(Entity, &ConnectionBeam)>,
) {
    // Build lookup for node positions
    let node_positions: HashMap<String, Vec3> = nodes
        .iter()
        .map(|(n, t)| (n.node_id.clone(), t.translation))
        .collect();

    // Remove stale beams
    for (entity, beam) in existing_beams.iter() {
        let key = (beam.from_node.clone(), beam.to_node.clone());
        if !federation.connections.contains_key(&key) {
            commands.entity(entity).despawn();
        }
    }

    // Track existing beams
    let existing: HashMap<(String, String), Entity> = existing_beams
        .iter()
        .map(|(e, b)| ((b.from_node.clone(), b.to_node.clone()), e))
        .collect();

    // Spawn new beams
    for (key, conn) in &federation.connections {
        if existing.contains_key(key) {
            continue;
        }

        // Get node positions
        let from_pos = match node_positions.get(&conn.from_node) {
            Some(p) => *p,
            None => continue,
        };
        let to_pos = match node_positions.get(&conn.to_node) {
            Some(p) => *p,
            None => continue,
        };

        // Calculate beam geometry
        let mid = (from_pos + to_pos) / 2.0;
        let direction = to_pos - from_pos;
        let length = direction.length();
        let rotation = Quat::from_rotation_arc(Vec3::Y, direction.normalize());

        // Beam color based on latency
        let beam_color = if conn.latency_ms < 50 {
            Color::srgb(0.0, 1.0, 0.5) // Green - good
        } else if conn.latency_ms < 200 {
            Color::srgb(1.0, 1.0, 0.0) // Yellow - okay
        } else {
            Color::srgb(1.0, 0.3, 0.0) // Orange - slow
        };

        let beam_material = materials.add(StandardMaterial {
            base_color: beam_color.with_alpha(0.6),
            emissive: beam_color.to_linear() * 0.3,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        });

        commands.spawn((
            PbrBundle {
                mesh: meshes.add(Cylinder::new(0.05, length)),
                material: beam_material,
                transform: Transform::from_translation(mid).with_rotation(rotation),
                ..default()
            },
            ConnectionBeam {
                from_node: conn.from_node.clone(),
                to_node: conn.to_node.clone(),
                transport_type: conn.transport_type.clone(),
                is_active: conn.is_active,
                latency_ms: conn.latency_ms,
                flow_direction: 1.0,
            },
        ));
    }
}

/// System to animate connection beams (data flow visualization)
pub fn animate_connection_beams(
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut beams: Query<(&Handle<StandardMaterial>, &mut ConnectionBeam)>,
) {
    for (handle, mut beam) in beams.iter_mut() {
        if !beam.is_active {
            continue;
        }

        // Pulse the beam to show data flow
        beam.flow_direction = (time.elapsed_seconds() * 2.0).sin();

        if let Some(material) = materials.get_mut(handle) {
            let pulse = (time.elapsed_seconds() * 3.0).sin() * 0.3 + 0.7;
            let base = material.base_color.to_srgba();
            material.emissive = LinearRgba::new(
                base.red * pulse,
                base.green * pulse,
                base.blue * pulse,
                1.0
            );
        }
    }
}

/// System to update node visuals based on state
pub fn update_node_visuals(
    federation: Res<FederationMesh>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut nodes: Query<(&FederationNodeVisual, &Handle<StandardMaterial>)>,
) {
    for (node, handle) in nodes.iter_mut() {
        let info = match federation.nodes.get(&node.node_id) {
            Some(i) => i,
            None => continue,
        };

        if let Some(material) = materials.get_mut(handle) {
            // Dim disconnected nodes
            let alpha = if info.is_connected { 0.9 } else { 0.4 };
            material.base_color = node.primary_color.with_alpha(alpha);

            // Reduce glow for disconnected
            let glow_mult = if info.is_connected { 0.5 } else { 0.1 };
            material.emissive = node.primary_color.to_linear() * glow_mult;
        }
    }
}

/// Calculate spatial position for a node based on network topology
pub fn calculate_node_position(
    node_id: &str,
    is_local: bool,
    latency_to_local: Option<u32>,
    discovery_type: Option<&str>,
) -> Vec3 {
    if is_local {
        return Vec3::ZERO;
    }

    // Base distance on latency (closer = lower latency)
    let base_distance = match latency_to_local {
        Some(ms) if ms < 10 => 5.0,    // Local network
        Some(ms) if ms < 50 => 10.0,   // LAN
        Some(ms) if ms < 200 => 20.0,  // Regional
        Some(_) => 35.0,                // Far
        None => 25.0,                   // Unknown
    };

    // Adjust for discovery type
    let distance = match discovery_type {
        Some("mdns") => base_distance * 0.8,
        Some("bluetooth") => base_distance * 0.5,
        Some("passkey") => base_distance * 1.0,
        _ => base_distance,
    };

    // Hash node ID for consistent angle
    let hash = node_id.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let angle = (hash % 360) as f32 * std::f32::consts::PI / 180.0;

    Vec3::new(
        angle.cos() * distance,
        1.5 + (hash % 5) as f32 * 0.5,
        angle.sin() * distance,
    )
}

// ============================================================================
// FEDERATION PLUGIN
// ============================================================================

/// Plugin that adds federation visualization to Deep Net
pub struct FederationPlugin;

impl Plugin for FederationPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<FederationMesh>()
            .add_event::<FederationEvent>()
            .add_systems(Update, (
                spawn_federation_nodes,
                update_connection_beams,
                animate_connection_beams,
                update_node_visuals,
            ));
    }
}
