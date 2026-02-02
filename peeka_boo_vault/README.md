# PeekabooVault

A Rust-based Network Video Recorder with ONVIF camera discovery.

## Phase 1 - ONVIF Discovery & Settings UI

This is Phase 1 of PeekabooVault, implementing ONVIF camera discovery and a web-based settings UI.

## Quick Start

### Prerequisites

- Rust (edition 2024)
- Node.js 18+ (for UI development)

### Running the Backend

```bash
# From the repository root
cargo run -p peekaboovault

# With custom port
cargo run -p peekaboovault -- --port 8080

# With verbose logging
cargo run -p peekaboovault -- --verbose
```

The server will start on `http://localhost:8080` by default.

### Building the UI

```bash
cd peekaboovault/ui

# Install dependencies
npm install

# Development mode (with hot reload, proxies to backend)
npm run dev

# Build for production
npm run build
```

The production build outputs to `peekaboovault/ui/build` which is served by the Rust backend.

### Configuration

Create a `config.toml` file (optional):

```toml
[server]
bind_address = "0.0.0.0:8080"
static_dir = "./ui/build"

[discovery]
default_duration_secs = 5
auto_discover = true
scan_interval_secs = 300

[storage]
db_path = "./data/peekaboovault.db"
hot_storage_path = "./data/recordings/hot"
```

## API Endpoints

### Status
- `GET /api/status` - Get server status

### Discovery
- `POST /api/discovery/scan` - Start ONVIF discovery scan
- `GET /api/discovery/devices` - Get discovered devices

### Cameras
- `GET /api/cameras` - List all cameras
- `POST /api/cameras` - Add a camera
- `GET /api/cameras/{id}` - Get camera details
- `DELETE /api/cameras/{id}` - Delete a camera
- `POST /api/cameras/{id}/credentials` - Set camera credentials
- `POST /api/cameras/{id}/probe` - Probe camera for device info and streams

## Architecture

PeekabooVault follows the patterns established in `blue-onyx`:
- Axum server with `CancellationToken` for graceful shutdown
- `Arc<ServerState>` with `RwLock`/`Mutex`-guarded mutable state
- Background tasks for discovery and health monitoring
- TOML configuration as the source of truth

## Project Structure

```
peekaboovault/
├── Cargo.toml          # Rust dependencies
├── src/
│   ├── lib.rs          # Library root
│   ├── api.rs          # Request/response DTOs
│   ├── config.rs       # TOML configuration
│   ├── discovery.rs    # ONVIF discovery service
│   ├── server.rs       # Axum HTTP server
│   ├── state.rs        # Shared application state
│   └── bin/
│       └── peekaboovault.rs  # CLI entry point
└── ui/
    ├── package.json    # Node.js dependencies
    ├── svelte.config.js
    ├── src/
    │   ├── routes/     # SvelteKit pages
    │   └── lib/        # Shared components & API client
    └── static/         # Static assets
```

## License

MIT
