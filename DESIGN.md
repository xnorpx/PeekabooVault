# PeekabooVault — Design Doc (Phased Implementation)

## 0) Summary
PeekabooVault is a Rust NVR that discovers ONVIF cameras on the local network, onboards them (credentials + configuration), records their RTSP streams to disk, indexes recordings in SQLite (short-term + long-term), exposes an HTTP API via Axum (including camera health checks + ONVIF event ingest), serves a SvelteKit UI, supports replay from the database, and finally adds low-latency live view via WebRTC (str0m) by forwarding camera streams.

Primary video target is **H.265/HEVC from day one** (with H.264 supported when cameras provide it).

This document describes the target architecture, storage model, and a phased implementation plan.

## 1) Goals / Non-goals

### Goals
- **Camera discovery (ONVIF WS-Discovery):** find cameras on LAN reliably.
- **Onboarding:** store camera identity + endpoints; configure username/password; validate connectivity.
- **Streaming:** fetch RTSP URLs (main/sub streams) via ONVIF and connect with Retina.
- **Recording:** continuous segmented recording to disk with crash-safe behavior.
- **Indexing:** SQLite metadata for efficient time-based search and playback.
- **Events + health:** ingest ONVIF events; periodically poll cameras (every 30s) to maintain liveness.
- **UI:** manage cameras + storage; browse timeline; replay.
- **Live view (WebRTC):** browser ↔ server WebRTC with server forwarding camera video.
- **Extensibility:** modular architecture allowing future addition of detection providers, storage backends, and integrations (inspired by Scrypted's plugin model).
- **Rebroadcast/Prebuffer:** maintain low-latency access to camera streams with intelligent prebuffering for instant playback.

### Non-goals (initially)
- Full transcoding pipeline (CPU-heavy) or analytics.
- Cross-network discovery (beyond local subnets) in v1.
- Complex user management/roles until the NVR basics are stable.
- Full plugin runtime (unlike Scrypted's JS/Python plugins, we focus on compile-time extensibility via Rust traits).

## 2) Key Technical Choices
- **Runtime:** Tokio.
- **HTTP:** Axum.
- **ONVIF:** existing ONVIF crates in this workspace (WS-Discovery + WS-Security).
- **RTSP client:** Retina (subscribe to both main and sub streams).
- **Databases:** SQLite, split into **short-term (hot)** and **long-term (cold)** DBs.
- **Live streaming:** str0m (WebRTC PeerConnection), forward camera video.
- **Stream rebroadcast:** Internal RTSP server for stream distribution (inspired by Scrypted's prebuffer-mixin).
- **Provider traits:** Rust trait-based abstraction for cameras, detectors, and storage backends (Scrypted-inspired interfaces).

## 3) Architecture Overview

PeekabooVault is built as a single Rust server process with multiple background tasks. The architecture draws heavily from four reference implementations in this workspace, each providing proven patterns for different concerns.

## 3.1) Reference Implementation: `blue-onyx`

This repository includes a checked-out copy of **blue-onyx** under [blue-onyx/](blue-onyx/). PeekabooVault should intentionally **mirror its Rust + Tokio + Axum coding style and project structure**, especially around:

- **Axum server structure:** a `run_server(...)` entrypoint that builds a `Router`, binds a `TcpListener`, and uses `axum::serve(...).with_graceful_shutdown(...)`.
- **Cancellation/shutdown:** use `tokio_util::sync::CancellationToken` and `tokio::select!` for graceful shutdown, matching the pattern in [blue-onyx/src/server.rs](blue-onyx/src/server.rs).
- **Shared state:** `Arc<ServerState>` with `tokio::sync::Mutex`-guarded mutable state, passed via `Router::with_state(...)`.
- **Module boundaries:** keep request/response DTOs in an `api` module (see [blue-onyx/src/api.rs](blue-onyx/src/api.rs)), and background initialization / orchestration in a coordinator module (see [blue-onyx/src/startup_coordinator.rs](blue-onyx/src/startup_coordinator.rs)).
- **Logging:** consistent use of `tracing::{info, warn, error, debug}` with structured fields.

PeekabooVault will differ in domain, but the *shape* (server entrypoint + state + coordinator tasks) should look familiar to anyone reading blue-onyx.

## 3.2) Reference Implementation: `frigate` (settings + metadata patterns)

This repository also includes a checked-out copy of **Frigate NVR** under [frigate/](frigate/). PeekabooVault should take inspiration from Frigate’s approach to:

- **Configuration as the source of truth:** a single config file that can be edited by users and validated, plus runtime APIs/UI that reflect and edit that config.
- **Camera-centric config model:** cameras as top-level entities with stream roles and per-camera overrides.
- **Metadata-first indexing:** a relational DB schema that supports time-based queries efficiently (see Frigate’s `recordings` and `timeline` concepts under [frigate/migrations/](frigate/migrations/)).

Important difference:
- Frigate uses **YAML** configuration; PeekabooVault will use **TOML** (Rust-first ergonomics), but we can keep the same *shape* of configuration and validation philosophy.

## 3.3) Reference Implementation: `moonfire-nvr` (recordings + SQLite schema)

This repository includes a checked-out copy of **Moonfire NVR** under [moonfire-nvr/](moonfire-nvr/). PeekabooVault should treat Moonfire as the primary reference for **recording storage layout and database indexing**:

- **Hybrid storage design:** video samples on disk + metadata/index in SQLite (Moonfire describes this in [moonfire-nvr/README.md](moonfire-nvr/README.md) and implements the schema in [moonfire-nvr/server/db/schema.sql](moonfire-nvr/server/db/schema.sql)).
- **Segmented recordings:** store continuous streams as short, fixed-duration segments (Moonfire targets ~1 minute segments) and stitch them at playback/export time.
- **Efficient time indexing:** store time as integer tick units (Moonfire uses 90 kHz ticks) and build covering indexes for common queries.
- **On-demand MP4 construction:** support generating `.mp4` for arbitrary time ranges without re-encoding by combining stored samples with DB-derived indexing metadata.

Important note:
- Moonfire NVR is licensed GPL-3.0-or-later (with an OpenSSL linking exception). PeekabooVault can use it as an **architectural reference**, but we should not copy Moonfire source code into PeekabooVault.

## 3.4) Reference Implementation: `scrypted` (device abstraction + stream management)

This repository includes a checked-out copy of **Scrypted** under [scrypted/](scrypted/). PeekabooVault should take inspiration from Scrypted's approach to:

### Device Abstraction & Interfaces
- **Interface-driven device model:** Scrypted defines devices through composable interfaces (`VideoCamera`, `MotionSensor`, `ObjectDetector`, `Intercom`, etc.). Each device implements only the interfaces it supports. PeekabooVault should adopt a similar Rust trait-based approach:
  ```rust
  pub trait VideoCamera: Send + Sync {
      async fn get_video_stream(&self, options: StreamOptions) -> Result<MediaStream>;
      async fn get_video_stream_options(&self) -> Result<Vec<MediaStreamOptions>>;
  }
  
  pub trait MotionSensor: Send + Sync {
      fn motion_detected(&self) -> bool;
      fn subscribe_motion(&self) -> broadcast::Receiver<MotionEvent>;
  }
  
  pub trait ObjectDetector: Send + Sync {
      async fn get_object_types(&self) -> Result<ObjectDetectionTypes>;
      async fn detect_objects(&self, input: MediaObject) -> Result<ObjectsDetected>;
  }
  ```

### Mixin / Provider Pattern
- **Mixins for cross-cutting concerns:** Scrypted uses "mixins" to layer functionality onto devices (e.g., `prebuffer-mixin` adds stream prebuffering to any `VideoCamera`, `objectdetector` adds AI detection). PeekabooVault can achieve similar composition through Rust's newtype pattern or wrapper structs:
  ```rust
  pub struct PrebufferedCamera<C: VideoCamera> {
      inner: C,
      prebuffer: Arc<Prebuffer>,
  }
  
  pub struct DetectionEnabledCamera<C: VideoCamera, D: ObjectDetector> {
      camera: C,
      detector: D,
      zones: Vec<DetectionZone>,
  }
  ```

### Stream Destinations & Quality Selection
- **MediaStreamDestination concept:** Scrypted differentiates stream consumers (`local`, `remote`, `low-resolution`, `local-recorder`). This helps select the right stream quality:
  - `local-recorder`: highest quality for recording
  - `low-resolution`: substream for detection/analysis
  - `remote`: optimized for network delivery
  
  PeekabooVault should adopt similar stream destination hints to automatically select main vs. sub streams.

### Prebuffer / Rebroadcast Architecture (from `prebuffer-mixin`)
- **Always-on stream rebroadcast:** Scrypted maintains persistent RTSP connections and rebroadcasts to multiple consumers. Key insights:
  - Keep ~10 seconds of prebuffer for instant playback on new viewer connections
  - Detect IDR frames to ensure clean stream joins
  - Support both TCP and UDP RTSP transport
  - Handle codec detection (H.264/H.265) and SPS/PPS parsing
  - Expose internal RTSP server paths for each camera stream
  
  PeekabooVault should implement a similar `StreamManager` that:
  1. Maintains a single RTSP connection per camera stream
  2. Buffers recent IDR-aligned data
  3. Multiplexes to multiple consumers (recorder, WebRTC, HLS)
  4. Tracks keyframe intervals for adaptive prebuffer sizing

### ONVIF Integration Patterns (from `plugins/onvif`)
- **Event topic normalization:** Scrypted's ONVIF plugin strips namespaces from event topics for consistent handling across vendors:
  ```
  // Scrypted strips "tns1:" etc from topic names
  // "tns1:RuleEngine/CellMotionDetector/Motion" → "RuleEngine/CellMotionDetector/Motion"
  ```
- **Vendor-specific event handling:** Different cameras report events differently (Reolink "Visitor", Mobotix "Ring", generic "MotionAlarm"). PeekabooVault should normalize these to a common event model.
- **Codec configuration via ONVIF:** Use `getVideoEncoderConfigurationOptions` to discover supported codecs, resolutions, frame rates, GOP lengths, and bitrates—then allow users to configure optimal settings.

### Object Detection Pipeline (from `plugins/objectdetector`)
- **Zone-based detection:** Define polygons for detection zones with options:
  - `filterMode`: include/exclude/observe
  - `type`: Intersect vs. Contain (does object touch zone or is fully inside?)
  - Per-zone class filters and score thresholds
- **Motion sensor integration:** Object detection can supplement or replace built-in motion sensors:
  - `Assist`: Only run detection when camera reports motion
  - `Replace`: Ignore camera motion, rely solely on detection
- **Performance throttling:** Track FPS of detection pipeline and gracefully degrade when system is overloaded (kill low-FPS sessions to restore performance).

### State Management
- **Centralized state with event notifications:** Scrypted's `ScryptedStateManager` provides:
  - Throttled DB writes (batch upserts every 30s)
  - Event emission on state changes
  - Refresh polling for devices that support it
  
  PeekabooVault should implement similar patterns for camera state (online/offline, current settings, last event times).

### Key Scrypted Concepts to Adopt

| Scrypted Concept | PeekabooVault Equivalent |
|------------------|--------------------------|
| `ScryptedInterface` (e.g., `VideoCamera`) | Rust traits (`VideoCamera`, `MotionSensor`) |
| `MixinProvider` | Wrapper structs with trait delegation |
| `MediaStreamDestination` | `StreamPurpose` enum (Record, Live, Analysis) |
| `prebuffer-mixin` | `StreamManager` with ring buffer |
| `ObjectDetector` plugin | `DetectionProvider` trait + blue-onyx integration |
| Storage Settings per device | Per-camera config in TOML + runtime overrides |
| Event normalization | Unified `CameraEvent` enum |

### High-level components
1. **Supervisor / App Orchestrator**
   - Loads config, opens DBs, initializes storage roots.
   - Spawns and monitors background tasks.

2. **Discovery Service (ONVIF WS-Discovery)**
   - Periodic discovery scans.
   - Dedupes devices by stable identifiers (e.g., XAddr + endpoint reference, serial/mac if available).

3. **Camera Manager**
   - Stores camera inventory and credentials.
   - Fetches ONVIF capabilities and RTSP stream URIs.
   - Owns per-camera task lifecycle: ingest/record + health + event sessions.

4. **Stream Manager (Scrypted-inspired)**
   - Maintains persistent RTSP connections per camera stream.
   - Provides ~10s prebuffer for instant playback.
   - Multiplexes stream data to multiple consumers.
   - Tracks keyframe timing and codec parameters.

5. **Ingest/Recorder (Retina RTSP)**
   - Consumes from Stream Manager (not directly from camera).
   - Writes continuous segmented recording to disk.
   - Emits metadata events (segment created, keyframe timestamps, errors).

6. **Event Ingest (ONVIF Events)**
   - Supports PullPoint or subscription-based notifications (depending on camera support).
   - Normalizes events (motion, digital input, analytics) into a common model using Scrypted-style topic stripping.

7. **Indexer (SQLite hot/cold)**
   - Writes segment metadata, camera metadata, event metadata.
   - Retention/migration job: moves segments and metadata from hot → cold.

8. **HTTP API (Axum)**
   - CRUD cameras, storage paths, retention settings.
   - Query recordings by time range.
   - Playback endpoints (initially “download/stream a segment”, later timeline/HLS).
   - Receives ONVIF event callbacks if you choose push subscriptions.

9. **UI (SvelteKit)**
   - Admin UI + viewer UI.
   - Served by Axum (static assets or reverse proxy pattern).

10. **WebRTC Gateway (str0m)**
    - Signaling over HTTP/WebSocket.
    - Per-view session tasks that forward camera video into WebRTC.
    - Consumes from Stream Manager for instant stream access.

11. **Detection Provider (Optional, blue-onyx integration)**
    - Trait-based abstraction for object detection backends.
    - Zone-based filtering (Scrypted-inspired).
    - Motion sensor assist/replace modes.

### Process model
- Single server process initially.
- Many background tasks:
  - Discovery loop
  - Stream Manager sessions (one per stream)
  - Per-camera health loop
  - Per-camera event loop
  - Per-camera ingest/record loops (main + sub)
  - Retention/migration loop
  - Detection pipeline (when enabled)

## 4) Data Model (SQLite)

PeekabooVault’s database design should be **Moonfire-first for recordings**, with a small amount of additional schema to support ONVIF identity/onboarding and Frigate-style timeline/event metadata.

### Why two SQLite DBs
- **Hot DB:** fast writes for “last N days” indexing, optimized for frequent reads during day-to-day browsing.
- **Cold DB:** larger historical index with fewer writes; optionally moved to slower storage.

> Note: SQLite can handle a lot, but splitting hot/cold simplifies retention and reduces index bloat for the most common queries. Both DBs should share the *same* schema for the recording/index tables so that query code can be reused.

### Proposed schema (Moonfire-inspired core + PeekabooVault additions)

#### Recording/index core (Moonfire-inspired)
These are the tables that define the **recording truth**. The names and relationships should be strongly aligned with Moonfire concepts:

- `open`
  - Tracks each read/write open of the DB (used for crash-safety, integrity, and etag/version disambiguation).
- `sample_file_dir`
  - Storage roots (disks/paths) used for recorded sample files; allows multiple disks.
- `camera`
  - A stable camera row for the recorder (human name + serialized config snapshot).
- `stream`
  - Per-camera streams: `main` and `sub` (and optionally `ext` later), with stream config and counters.
- `recording`
  - One row per completed segment (~60s) with start time, duration, bytes, flags, and enough fields to serve as a covering index for timeline queries.
- `recording_playback`
  - Per-recording playback index data needed to assemble on-demand MP4 (e.g., sample tables / indexes).
- `garbage`
  - Tracks files scheduled for deletion, to make retention robust across crashes.

> Implementation note: Moonfire uses integer tick time (90 kHz) for `start_time` and durations and designs covering indexes carefully. PeekabooVault should adopt the same approach (tick time internally; convert to RFC3339/Unix ms at API boundaries).

#### ONVIF/onboarding additions
- `onvif_device`
  - WS-Discovery identity: XAddr(s), scopes, endpoint reference/URN, last_seen, and any stable serial/MAC if available.
- `credential`
  - Encrypted/OS-protected secret material for camera auth (store references/ids in DB, not plaintext).
- `camera_link`
  - Links `onvif_device` → `camera` (the recorder entity) and tracks onboarding state.
  - Rationale for separate table: a camera may be added manually (no ONVIF device), or re-discovered after replacement (new ONVIF device, same logical camera). This indirection allows flexible identity management.

#### Timeline/events (Frigate-inspired)
- `event`
  - Normalized ONVIF events (and later analytics) with timestamp, type, and JSON payload.
- `timeline`
  - A unified timeline table that can be efficiently queried for UI review (recordings, events, health transitions) similar in spirit to Frigate’s timeline concept.
- `health_check`
  - Periodic health pings with latency/error for debugging and “camera offline” UX.

#### Settings/config
- Prefer **TOML file config** as the source of truth (Frigate-inspired). The DB should store only:
  - a `settings`/`meta` row for versioning, last-applied config hash, and small runtime state.

### Migration hot → cold
- A retention job periodically enforces quota-based budgets and migrates hot → cold:
  1. Computes current usage by `stream` (bytes and estimated bitrate), and compares to configured budgets.
  2. Selects eviction candidates oldest-first (with policy knobs to keep `main` longer than `sub`, or vice versa).
  3. Moves sample files from hot storage roots to cold storage roots.
  4. Copies the corresponding rows (`recording`, `recording_playback`, and any needed linkage) into the cold DB.
  5. Deletes hot DB rows and schedules hot files for deletion via `garbage`.

Crash-safety principle:
- Moves should be modeled as a small state machine (or transaction boundaries) so that on startup, PeekabooVault can resume/repair partial migrations.

## 5) Storage Layout on Disk (Moonfire-inspired approach)

The key principles to mimic (conceptually), using Moonfire as the reference:
- Store media as **immutable segments**.
- Keep a **DB index** mapping time → file segments.
- Use deterministic directory layout for easy ops/backup.

### Primary layout (Moonfire-style sample file directories)

Use one or more **sample file directories** (Moonfire concept) that store the raw compressed samples for each recording segment. The SQLite DB stores the indexing metadata required to construct a valid `.mp4` for playback/export.

Example (illustrative):
- `recordings/hot/<dir_uuid_or_id>/...` (sample files)
- `recordings/cold/<dir_uuid_or_id>/...` (sample files)

The important invariant is not the exact folder names; it’s that:
- sample files are immutable once finalized,
- their identity is tracked in the DB,
- and retention can safely delete them by consulting DB + `garbage`.

### Recording format

Use the **Moonfire-inspired sample file** approach:
- Store each segment as a **sample file** containing concatenated compressed samples (akin to the `mdat` payload), while storing the sample tables / indexing info in SQLite for on-demand `.mp4` construction.
- This approach is more flexible than pre-muxed containers: arbitrary time-range exports without re-muxing, efficient storage (no per-segment container overhead), and the DB becomes the single source of truth for segment boundaries.

Codec requirements:
- **Record:** support **H.265** and **H.264**.
- **Replay:** start by replaying the recorded codec (no transcode) where the client supports it.
- **Live (WebRTC):** browsers commonly support H.264; HEVC support is inconsistent. Plan to:
  - forward **H.264** when available,
  - and add a **transcode path** (HEVC→AVC) later if needed for “HEVC-only cameras”.

## 6) Recording Pipeline

### RTSP ingest
- Use Retina to:
  - DESCRIBE/SETUP/PLAY
  - Receive RTP packets
  - Track timestamps / RTCP / clock drift

### Writing to disk
- Segment length: align with Moonfire’s proven approach (≈60s segments) unless we have a concrete reason to go shorter.
- Ensure each segment starts at (or quickly reaches) a keyframe.
- Write pattern:
  - write to temp file → fsync → rename to final name
  - then commit metadata to DB

### Handling disconnects
- Recorder tasks retry with backoff.
- On reconnect, start a new segment run (avoid corrupting existing files).
## 6.5) Stream Manager (Scrypted-inspired Prebuffer)

The Stream Manager is a critical component inspired by Scrypted's `prebuffer-mixin`. It sits between cameras and consumers (recorder, WebRTC, detection), providing:

### Core Responsibilities
1. **Single connection per stream:** Maintains exactly one RTSP connection to each camera stream, avoiding connection storms.
2. **Prebuffer ring:** Keeps ~10 seconds of IDR-aligned video data for instant playback.
3. **Consumer multiplexing:** Distributes stream data to multiple consumers without re-fetching.
4. **Codec detection:** Parses SPS/PPS for H.264, VPS/SPS/PPS for H.265 to detect resolution and profile.
5. **Keyframe tracking:** Monitors IDR frame intervals to optimize prebuffer size and detect stream issues.

### Architecture
```
┌──────────────┐
│   Camera     │
│ (RTSP/ONVIF) │
└──────┬───────┘
       │ Single RTSP connection
       ▼
┌──────────────────────────────────────────┐
│           Stream Manager                  │
│  ┌─────────────┐  ┌──────────────────┐   │
│  │  Prebuffer  │  │  Codec Metadata  │   │
│  │  Ring (~10s)│  │  (SPS/PPS/VPS)   │   │
│  └─────────────┘  └──────────────────┘   │
│  ┌──────────────────────────────────┐    │
│  │     Consumer Subscriptions       │    │
│  │  - Recorder                      │    │
│  │  - WebRTC sessions               │    │
│  │  - Detection pipeline            │    │
│  │  - HLS generator                 │    │
│  └──────────────────────────────────┘    │
└──────────────────────────────────────────┘
       │ Broadcast to consumers
       ▼
┌──────────┬──────────┬──────────┐
│ Recorder │  WebRTC  │Detection │
└──────────┴──────────┴──────────┘
```

### Stream Session Lifecycle
```rust
pub struct StreamSession {
    camera_id: CameraId,
    stream_id: StreamId,  // "main" or "sub"
    rtsp_url: Url,
    
    // Prebuffer state
    prebuffer: RingBuffer<PrebufferChunk>,
    last_idr_time: Option<Instant>,
    detected_idr_interval: Duration,
    
    // Codec metadata
    video_codec: Option<VideoCodec>,
    audio_codec: Option<AudioCodec>,
    sps_pps: Option<Vec<u8>>,  // H.264/H.265 parameter sets
    
    // Consumer management
    consumers: Vec<broadcast::Sender<StreamChunk>>,
    
    // Connection state
    state: SessionState,
    reconnect_backoff: ExponentialBackoff,
}

pub enum SessionState {
    Connecting,
    Active { since: Instant },
    Reconnecting { attempts: u32 },
    Offline { since: Instant },
}
```

### Prebuffer Behavior (from Scrypted patterns)
- **IDR alignment:** Prebuffer always starts from an IDR frame so new consumers get a clean decode start.
- **Dynamic sizing:** Track actual IDR intervals (varies by camera); adjust prebuffer to hold 2-3 GOP lengths.
- **Chunk metadata:** Each chunk includes timestamp, whether it's a keyframe, and size.
- **Battery-aware:** For battery-powered cameras, disable prebuffer when camera is on battery (Scrypted pattern).

### Consumer Subscription API
```rust
impl StreamManager {
    /// Subscribe to a stream, receiving prebuffered data first then live
    pub async fn subscribe(
        &self,
        camera_id: CameraId,
        stream: StreamType,
        purpose: StreamPurpose,
    ) -> Result<StreamSubscription> {
        // 1. Get or create session
        // 2. Wait for active state (or timeout)
        // 3. Send prebuffer contents
        // 4. Return live broadcast receiver
    }
    
    /// Get stream metadata without subscribing
    pub async fn get_stream_info(
        &self,
        camera_id: CameraId,
        stream: StreamType,
    ) -> Result<StreamInfo> {
        // Returns codec, resolution, bitrate estimate, IDR interval
    }
}

pub enum StreamPurpose {
    Recording,      // Wants main stream, high quality
    LiveView,       // Wants low latency, may prefer substream
    Detection,      // Wants substream for analysis
    Export,         // Wants main stream for file export
}
```

### Integration Points
- **Recorder:** Subscribes to main stream with `StreamPurpose::Recording`
- **WebRTC:** Subscribes with prebuffer for instant playback, prefers H.264 for browser compatibility
- **Detection:** Subscribes to substream with `StreamPurpose::Detection`, lower resolution for faster inference
- **Health monitor:** Uses Stream Manager's session state to track camera online/offline status
## 7) Health Checks + Events

### Health checks (every 30s)
- Minimal ONVIF request (cheap and reliable), e.g.:
  - `GetSystemDateAndTime` or `GetDeviceInformation`
- Record latency + failures into `health_checks`.
- If camera stops responding:
  - mark offline
  - signal recorder to reconnect

### ONVIF Events
Two common approaches:
- **PullPoint:** server periodically pulls events from each camera.
- **Push subscription:** camera sends notifications to your Axum endpoint (requires reachable callback URL).

Preferred approach:
- **Push subscription first** when a camera supports it.
- **Fallback to PullPoint** if push isn’t supported or isn’t reliable in the user’s network topology.

## 8) HTTP API + UI

### API surface (v1)
- `GET /api/cameras`
- `POST /api/cameras` (manual add)
- `POST /api/discovery/scan`
- `POST /api/cameras/{id}/credentials`
- `POST /api/cameras/{id}/refresh` (capabilities + RTSP URLs)
- `GET /api/recordings?camera_id=...&start=...&end=...`
- `GET /api/segments/{id}` (download/stream a segment)
- `GET /api/events?camera_id=...&start=...&end=...`

### SvelteKit integration
- Serve built assets from Axum.
- Consider a dev mode proxy during development.

## 9) WebRTC Live View (str0m)

### Signaling
- WebSocket preferred:
  - exchange SDP offer/answer
  - exchange ICE candidates

### Media forwarding strategy
- Ideal: forward camera’s H.264 Annex B / RTP into WebRTC with minimal re-encode.
- Tasks:
  - depayload RTSP RTP
  - packetize for WebRTC RTP
  - handle keyframes, SPS/PPS insertion for new viewers

Start with **video-only** for first live view.

Important compatibility note: most browsers reliably support **H.264 over WebRTC**; **H.265 over WebRTC** is not consistently available. That means the “first working live view” should target H.264 forwarding, even if the recorder supports H.265 from day one.

## 10) Phased Implementation Plan

### Phase 1 — ONVIF + Settings UI (first vertical slice)

The goal of Phase 1 is: **get ONVIF working end-to-end with a web settings page** so you can discover cameras, enter credentials, subscribe to events, and fetch RTSP stream URIs.

Deliverables:
- Server skeleton (Axum + Tokio) following the `blue-onyx` structure:
  - `src/server.rs` with `run_server(...)` + graceful shutdown via `CancellationToken`.
  - `src/api.rs` for request/response DTOs.
  - `src/startup_coordinator.rs` for background loops.
- TOML-based configuration (Frigate-inspired config-as-truth):
  - storage roots, server bind, discovery settings, per-camera settings.
- Minimal persistence:
  - store discovered ONVIF devices, camera records, and credential references.
- ONVIF discovery + onboarding endpoints:
  - scan WS-Discovery, list candidates, “adopt/add camera”.
- ONVIF capabilities + stream URI retrieval:
  - fetch media profiles; fetch RTSP URIs for **main + sub**.
- ONVIF events subscription management:
  - prefer push subscriptions when supported; fallback to PullPoint.
  - show last event time + subscription status per camera.
- SvelteKit “Settings” UI served by Axum:
  - discover cameras, add/edit camera, set credentials, toggle event mode, view subscription health.
  - show RTSP main/sub URIs and a “probe” status.

Acceptance criteria:
- From the browser settings UI: discover a camera, enter credentials, confirm ONVIF connectivity, fetch RTSP URIs, and observe ONVIF events arriving.

Notes:
- Browsers can’t play RTSP directly. In Phase 1, the UI should **display RTSP URIs** and a server-side **probe** (Retina connects briefly and reports codec/resolution/online). Low-latency in-browser playback comes later in the WebRTC phase.

### Phase 2 — Stream Manager + Recorder Foundation (RTSP ingest + DB schema)
Deliverables:
- **Stream Manager implementation (Scrypted-inspired):**
  - Single RTSP connection per stream with automatic reconnection
  - ~10 second prebuffer with IDR alignment
  - Consumer subscription API for recorder and future WebRTC
  - Codec detection (H.264/H.265) with SPS/PPS parsing
  - Keyframe interval tracking for adaptive prebuffer sizing
- Establish the Moonfire-inspired recording tables (`stream`, `recording`, `recording_playback`, `open`, `sample_file_dir`, `garbage`).
- Wire per-camera stream tasks that consume from Stream Manager and rotate ~60s segments.
- Write sample files + insert recording rows (crash-safe finalize).

Acceptance criteria:
- A configured camera records continuously and produces indexed segments without manual intervention.
- Stream Manager maintains persistent connection and can serve multiple consumers.

### Phase 3 — Playback (Frame-based WebRTC Playback)
Deliverables:
- **Frame storage with keyframe markers:** Each recorded frame stored with timestamp, keyframe flag, and sequence number.
- **Time-based frame queries:** Query frames by time range from SQLite with efficient keyframe seeking.
- **Keyframe seeking logic:** When client requests playback at time T, find nearest keyframe (before T for forward play, closest for seeking).
- **Frame serving API:** HTTP/WebSocket endpoint to stream frames for a time range, starting from keyframe.
- Basic timeline UI for selecting playback position.

Design notes:
- No MP4 assembly needed - frames sent directly via WebRTC (prepared here, actual WebRTC in Phase 6).
- Unified pipeline: same frame data serves both recording storage and playback.
- Seeking always snaps to nearest keyframe for clean decoder state.

Acceptance criteria:
- User can query frames by time range and receive keyframe-aligned frame data.
- Seeking forward/backward finds the appropriate keyframe boundary.

### Phase 4 — SQLite Indexing (Hot/Cold) + Retention [COMPLETE]
Deliverables:
- [x] Hot/Cold dual database architecture (HotColdDb)
- [x] Hot DB queries for timeline browsing
- [x] Retention policy configuration (quota-based + age limits)
- [x] Migration job to move recordings from hot → cold storage
- [x] Startup recovery for partial segments and interrupted migrations
- [x] Storage/retention HTTP API endpoints
- [x] Timeline query API with hot/cold awareness

Acceptance criteria:
- [x] Old recordings automatically move to cold storage + cold DB without gaps
- [x] All 26 tests passing

### Phase 5 — Health Monitoring + Timeline UX [COMPLETE]
Deliverables:
- [x] Health monitor with state machine (online/offline/degraded/unknown)
- [x] Health polling loop (configurable interval, default 30s)
- [x] State transitions with thresholds (consecutive failures → offline)
- [x] Event normalization using Scrypted-style topic stripping
- [x] Health/Events HTTP API endpoints
- [x] Timeline UI page with recordings and events visualization

Acceptance criteria:
- [x] Motion events show up; cameras marked offline within bounded time
- [x] All 30 tests passing

### Phase 6 — WebRTC Live View (str0m) [COMPLETE]
Deliverables:
- [x] str0m dependency with pure Rust crypto (str0m-rust-crypto)
- [x] WebRTC signaling types (api.rs)
- [x] WebRTC session manager (webrtc.rs)
- [x] HTTP signaling endpoints (POST /offer, POST /ice-candidate, etc.)
- [x] Forwarding pipeline connecting Stream Manager to WebRTC
- [x] UDP transport for RTP media with shared socket
- [x] Source address cache with halfbrown for fast ufrag lookup
- [x] Viewer UI for live playback with auto-reconnection
- [x] Stream selector (main/sub) and codec negotiation
- [x] Stats display (codec, resolution, fps, bitrate)

Acceptance criteria:
- [x] Browser can view live video with low latency; reconnect works
- [x] New viewer connections start playing instantly (within ~100ms)

### Phase 7 — Detection Integration (Optional, blue-onyx)
Deliverables:
- `DetectionProvider` trait abstraction.
- Integration with blue-onyx for object detection.
- Zone-based detection filtering (Scrypted-inspired):
  - Polygon zones with include/exclude modes
  - Per-zone class filters and score thresholds
- Motion sensor assist/replace modes.
- Detection events stored in timeline.

Acceptance criteria:
- Object detection runs on substream without impacting recording.
- Detection results trigger events and are visible in timeline.

### Phase 8 — Hardening, Testing, and Ops
Deliverables:
- Integration tests for DB + segment lifecycle.
- Soak testing (multi-camera).
- Backups and restore strategy.
- Packaging (Windows service, systemd, Docker—pick target).
- Performance throttling for detection pipeline (Scrypted-inspired).

Acceptance criteria:
- Stable 24/7 operation with clear logs, minimal manual intervention.

## 11) Open Questions (to confirm before Phase 3/4)
Confirmed:
- **Codec:** H.265 from day one (plus H.264 where available).
- **Events:** prefer push subscriptions; fallback to PullPoint.
- **Scale:** 1–32 cameras.
- **Stream Manager:** Scrypted-style prebuffer with IDR alignment (~10s).

Still to decide:
1. **Hot/cold boundary:** What's "short-term" vs "long-term" (e.g., 7 days hot, 90 days cold)?
2. **Auth model:** Should the web UI/API require login from day one?
3. **Multiple subnets/VLANs:** Only same subnet broadcast discovery, or do you need routed discovery?
4. **Detection integration:** blue-onyx integration priority vs. other features?

Retention modeling detail (quota-based):
- Do you want a **global storage budget** (e.g., “use at most 2TB”) or **per-camera/per-profile budgets**?
- Should retention prioritize keeping **main stream** longer than **sub stream**, or vice versa?

## 12) Scrypted Integration Opportunities (Future)

While PeekabooVault is a standalone Rust NVR, there are potential integration points with Scrypted:

### As a Scrypted Plugin Consumer
- PeekabooVault could optionally connect to a running Scrypted instance to:
  - Discover cameras that Scrypted manages (via its API)
  - Receive detection events from Scrypted's object detection plugins
  - Share camera credentials/configuration

### As a Recording Backend for Scrypted
- Scrypted lacks built-in continuous recording; PeekabooVault could fill that gap:
  - Scrypted handles camera onboarding + live view + detection
  - PeekabooVault handles recording storage + retention + playback

### Key Learnings from Scrypted to Apply
1. **Interface-first design:** Define capabilities as traits, compose through wrappers
2. **Prebuffer everything:** Instant playback is expected by users
3. **Normalize vendor differences:** Strip namespaces, handle quirks per-vendor
4. **Performance awareness:** Track FPS, throttle gracefully under load
5. **Battery-aware behavior:** Disable continuous features for battery cameras
6. **Stream destination hints:** Let consumers declare intent (recording vs. live vs. analysis)
7. **Mixin composition:** Layer functionality without tight coupling