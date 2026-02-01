//! Deep Net ECS Components
//!
//! Core components for visualizing the AI-Foundation federation mesh.

use bevy::prelude::*;

// Re-export federation components
pub use crate::federation::{
    FederationNodeVisual,
    ConnectionBeam,
    JumpPortal,
    LocalNode,
    DiscoveredNode,
    VisualIdentity,
};

// ============================================================================
// CORE COMPONENTS
// ============================================================================

/// The player entity (camera controller)
#[derive(Component)]
pub struct Player;

/// Camera with look direction
#[derive(Component)]
pub struct PlayerCamera {
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for PlayerCamera {
    fn default() -> Self {
        Self { yaw: 0.0, pitch: 0.0 }
    }
}

/// An AI entity in the world (legacy - prefer FederationNodeVisual)
#[derive(Component)]
pub struct AIEntity {
    pub ai_id: String,
    pub is_online: bool,
}

/// Pulsing glow effect
#[derive(Component)]
pub struct Glow {
    pub base_emissive: Color,
    pub pulse_speed: f32,
    pub pulse_amount: f32,
}

/// Hexagonal grid tile
#[derive(Component)]
pub struct HexTile {
    pub q: i32,
    pub r: i32,
}

// ============================================================================
// HARDWARE VISUALIZATION
// ============================================================================

/// Hardware component (RAM, drives, CPUs)
#[derive(Component)]
pub struct HardwareVisualization {
    pub kind: HardwareKind,
}

/// Type of hardware being visualized
#[derive(Clone, Copy)]
pub enum HardwareKind {
    RamBlock { index: usize, is_used: bool },
    DriveRing { drive_index: usize, ring_index: usize, is_used: bool },
    CpuCore { index: usize },
}

// ============================================================================
// INTERACTION
// ============================================================================

/// Component for entities that can be interacted with
#[derive(Component)]
pub struct Interactable {
    /// Label shown when near
    pub label: String,
    /// Interaction range
    pub range: f32,
    /// Is currently highlighted
    pub highlighted: bool,
}

impl Default for Interactable {
    fn default() -> Self {
        Self {
            label: "Interact".to_string(),
            range: 3.0,
            highlighted: false,
        }
    }
}

/// Component for entities the player can jump to
#[derive(Component)]
pub struct JumpTarget {
    /// Target node ID
    pub node_id: String,
    /// Distance to this target
    pub distance: f32,
}

// ============================================================================
// UI MARKERS
// ============================================================================

/// Marker for the node info panel
#[derive(Component)]
pub struct NodeInfoPanel;

/// Marker for the connection status indicator
#[derive(Component)]
pub struct ConnectionStatus;

/// Marker for the discovery radar
#[derive(Component)]
pub struct DiscoveryRadar;
