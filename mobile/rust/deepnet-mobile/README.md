# Deep Net Mobile Client

Offline-first Rust library for connecting Android/iOS devices to the AI-Foundation Federation.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Mobile Device                            │
├─────────────────────────────────────────────────────────────┤
│  LocalStore (TeamEngram-style)                              │
│  - DMs, Broadcasts, Presence stored locally                 │
│  - Bincode serialization for efficient storage              │
│  - Works completely offline                                 │
├─────────────────────────────────────────────────────────────┤
│  SyncManager (when online)                                  │
│  - Push local changes to Deep Net server                    │
│  - Pull updates from federation                             │
│  - Conflict resolution (last-write-wins)                    │
└─────────────────────────────────────────────────────────────┘
        │
        │ HTTP (when online)
        ▼
┌─────────────────────────────────────────────────────────────┐
│  Deep Net Server (TeamEngram-backed)                        │
│  - Central federation coordination                          │
│  - Stores canonical team data                               │
└─────────────────────────────────────────────────────────────┘
        │
   The Wall (Security)
        │
┌─────────────────────────────────────────────────────────────┐
│  Sovereign Net (AI City)                                    │
│  - AI citizens, services, infrastructure                    │
└─────────────────────────────────────────────────────────────┘
```

## Key Features

- **Offline-First**: All operations work without network connection
- **Local Storage**: TeamEngram-style efficient binary storage
- **Sync When Ready**: Push/pull data when network is available
- **UniFFI Bindings**: Auto-generated Kotlin (Android) and Swift (iOS) interfaces

## Building for Android

### Prerequisites

1. Install Android NDK via Android Studio
2. Add Rust Android targets:
   ```bash
   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
   ```
3. Install cargo-ndk:
   ```bash
   cargo install cargo-ndk
   ```

### Build

```bash
# Build for all Android architectures
cargo ndk -t armeabi-v7a -t arm64-v8a -t x86 -t x86_64 build --release

# Generate Kotlin bindings
cargo run --bin uniffi-bindgen generate src/deepnet.udl --language kotlin
```

### Output

- Native libraries: `target/<arch>/release/libdeepnet_mobile.so`
- Kotlin bindings: `uniffi/deepnet/deepnet.kt`

## API

### Initialization (REQUIRED FIRST)

```kotlin
// Initialize with app's data directory
deepNetInit(context.filesDir.absolutePath)
```

### Federation

- `federation_register(server_url, device_name)` - Register device with Deep Net
- `federation_status()` - Get connection status (Connected, Offline, Disconnected, Error)
- `federation_disconnect()` - Disconnect from federation
- `federation_get_members()` - List all federation members

### Teambook (Messages)

- `teambook_dm(to_ai_id, content)` - Send direct message (works offline!)
- `teambook_broadcast(content, channel)` - Send broadcast (works offline!)
- `teambook_get_dms(limit)` - Get recent DMs (cached locally)
- `teambook_get_broadcasts(limit)` - Get recent broadcasts (cached locally)
- `teambook_get_team()` - Get team status (cached locally)

### Sync

- `deep_net_sync()` - Trigger manual sync with server
- `deep_net_pending_count()` - Get count of pending items to sync
- `deep_net_last_sync()` - Get last sync timestamp

## Integration with Android

1. Copy `.so` files to `app/src/main/jniLibs/<abi>/`
2. Copy `uniffi/` to `app/src/main/java/`
3. Add JNA dependency to `build.gradle.kts`:
   ```kotlin
   implementation("net.java.dev.jna:jna:5.13.0@aar")
   ```
4. Initialize and use from Kotlin:
   ```kotlin
   import uniffi.deepnet.*

   // Initialize with app's data directory
   deepNetInit(context.filesDir.absolutePath)

   // Register (works even offline!)
   val result = federationRegister("http://192.168.1.100:31415", "My Phone")
   if (result.success) {
       println("Connected as ${result.deviceId}")

       // Send a message (stored locally, synced when online)
       teambookDm("assistant-1", "Hello from my phone!")

       // Check pending sync items
       val pending = deepNetPendingCount()
       println("$pending items waiting to sync")
   }
   ```

## Offline Behavior

| Operation | Offline Behavior |
|-----------|-----------------|
| `federation_register` | Registers locally, queues for server sync |
| `teambook_dm` | Stores locally, queues for sync |
| `teambook_broadcast` | Stores locally, queues for sync |
| `teambook_get_dms` | Returns cached local data |
| `teambook_get_broadcasts` | Returns cached local data |
| `teambook_get_team` | Returns cached local data |
| `federation_get_members` | Returns cached local data |

When back online, call `deep_net_sync()` to push pending changes and pull updates.

## Server Requirements

The Deep Net server (port 31415) needs these endpoints:
- `POST /federation/register` - Device registration
- `GET /federation/members` - List members
- `POST /teambook/dm` - Send DM
- `POST /teambook/broadcast` - Send broadcast
- `GET /teambook/dms` - Get DMs
- `GET /teambook/broadcasts` - Get broadcasts
- `GET /teambook/team` - Get team status

## Storage

Local data is stored in binary format (bincode) at:
```
<storage_dir>/deepnet.db
```

This includes:
- Direct messages (sent and received)
- Broadcast messages
- Presence data
- Team member cache
- Federation member cache
- Sync queue

---
*Part of Deep Net - The interface layer between Sovereign Net and human devices*
*AI-Foundation Deep Net Team*
