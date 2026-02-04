//! UI HTML Generation for PeekabooVault
//!
//! This module contains the HTML/CSS/JS generation for:
//! - Live Viewer (WebRTC)
//! - Replay/Review Page (Timeline + Playback)
//! - Dashboard (Multi-camera grid)
//!
//! Design inspired by Scrypted and Frigate with modern dark theme.

use uuid::Uuid;

/// Color palette for consistent theming
pub mod colors {
    pub const BG_DARK: &str = "#0d1117";
    pub const BG_CARD: &str = "#161b22";
    pub const BG_SURFACE: &str = "#21262d";
    pub const BG_HOVER: &str = "#30363d";
    pub const ACCENT: &str = "#238636";
    pub const ACCENT_HOVER: &str = "#2ea043";
    pub const HIGHLIGHT: &str = "#58a6ff";
    pub const DANGER: &str = "#f85149";
    pub const WARNING: &str = "#d29922";
    pub const TEXT_PRIMARY: &str = "#e6edf3";
    pub const TEXT_SECONDARY: &str = "#8b949e";
    pub const TEXT_MUTED: &str = "#6e7681";
    pub const BORDER: &str = "#30363d";
    pub const RECORDING: &str = "#238636";
    pub const MOTION: &str = "#58a6ff";
    pub const PERSON: &str = "#f85149";
    pub const VEHICLE: &str = "#d29922";
}

/// Generate the live viewer page HTML
pub fn generate_live_viewer(camera_id: &Uuid, camera_name: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
    <title>{camera_name} - Live</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap" rel="stylesheet">
    {common_styles}
    <style>
        .viewer-container {{
            display: flex;
            flex-direction: column;
            height: 100vh;
            overflow: hidden;
        }}
        
        .viewer-header {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 0.75rem 1.25rem;
            background: var(--bg-card);
            border-bottom: 1px solid var(--border);
            flex-shrink: 0;
        }}
        
        .header-left {{
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
        
        .back-button {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 36px;
            border-radius: 8px;
            background: transparent;
            border: none;
            color: var(--text-secondary);
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .back-button:hover {{
            background: var(--bg-hover);
            color: var(--text-primary);
        }}
        
        .camera-info {{
            display: flex;
            flex-direction: column;
        }}
        
        .camera-name {{
            font-size: 1rem;
            font-weight: 600;
            color: var(--text-primary);
        }}
        
        .camera-status {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            font-size: 0.75rem;
            color: var(--text-secondary);
        }}
        
        .status-indicator {{
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: var(--text-muted);
        }}
        
        .status-indicator.live {{
            background: var(--danger);
            box-shadow: 0 0 0 2px rgba(248, 81, 73, 0.3);
            animation: pulse-live 2s infinite;
        }}
        
        @keyframes pulse-live {{
            0%, 100% {{ box-shadow: 0 0 0 2px rgba(248, 81, 73, 0.3); }}
            50% {{ box-shadow: 0 0 0 4px rgba(248, 81, 73, 0.1); }}
        }}
        
        .status-indicator.connecting {{
            background: var(--warning);
            animation: pulse 1s infinite;
        }}
        
        .status-indicator.offline {{
            background: var(--text-muted);
        }}
        
        .header-right {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
        }}
        
        .layer-mode {{
            display: flex;
            background: var(--bg-surface);
            border-radius: 8px;
            padding: 3px;
            border: 1px solid var(--border);
        }}
        
        .layer-mode button {{
            padding: 0.35rem 0.6rem;
            font-size: 0.7rem;
            font-weight: 500;
            border: none;
            background: transparent;
            color: var(--text-muted);
            border-radius: 5px;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .layer-mode button.active {{
            background: var(--highlight);
            color: white;
        }}
        
        .layer-mode button:not(.active):hover {{
            color: var(--text-secondary);
        }}
        
        .stream-toggle {{
            display: flex;
            background: var(--bg-surface);
            border-radius: 8px;
            padding: 3px;
        }}
        
        .focus-btn {{
            padding: 0.4rem 0.75rem;
            font-size: 0.75rem;
            font-weight: 500;
            border: 2px solid var(--border);
            background: transparent;
            color: var(--text-secondary);
            border-radius: 8px;
            cursor: pointer;
            transition: all 0.2s;
            margin-right: 0.5rem;
        }}
        
        .focus-btn:hover {{
            border-color: var(--accent);
            color: var(--accent);
        }}
        
        .focus-btn.active {{
            background: linear-gradient(135deg, #f59e0b, #f97316);
            border-color: transparent;
            color: white;
            box-shadow: 0 0 12px rgba(249, 115, 22, 0.5);
        }}
        
        .stream-toggle button {{
            padding: 0.4rem 0.75rem;
            font-size: 0.75rem;
            font-weight: 500;
            border: none;
            background: transparent;
            color: var(--text-secondary);
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .stream-toggle button.active {{
            background: var(--accent);
            color: white;
        }}
        
        .stream-toggle button:not(.active):hover {{
            color: var(--text-primary);
        }}
        
        .stream-toggle.disabled {{
            opacity: 0.5;
            pointer-events: none;
        }}
        
        .video-wrapper {{
            flex: 1;
            display: flex;
            align-items: center;
            justify-content: center;
            background: #000;
            position: relative;
            overflow: hidden;
        }}
        
        .video-wrapper video {{
            max-width: 100%;
            max-height: 100%;
            object-fit: contain;
        }}
        
        .video-overlay {{
            position: absolute;
            inset: 0;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            background: rgba(0, 0, 0, 0.7);
            opacity: 1;
            transition: opacity 0.3s;
            pointer-events: none;
        }}
        
        .video-overlay.hidden {{
            opacity: 0;
        }}
        
        .loading-spinner {{
            width: 48px;
            height: 48px;
            border: 3px solid var(--bg-surface);
            border-top-color: var(--highlight);
            border-radius: 50%;
            animation: spin 1s linear infinite;
        }}
        
        @keyframes spin {{
            to {{ transform: rotate(360deg); }}
        }}
        
        .overlay-text {{
            margin-top: 1rem;
            font-size: 0.875rem;
            color: var(--text-secondary);
        }}
        
        .video-controls {{
            position: absolute;
            bottom: 0;
            left: 0;
            right: 0;
            padding: 1.5rem;
            background: linear-gradient(transparent, rgba(0, 0, 0, 0.9));
            display: flex;
            justify-content: space-between;
            align-items: flex-end;
            opacity: 0;
            transition: opacity 0.3s;
        }}
        
        .video-wrapper:hover .video-controls {{
            opacity: 1;
        }}
        
        .controls-left {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
        }}
        
        .control-btn {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 40px;
            height: 40px;
            border-radius: 10px;
            background: rgba(255, 255, 255, 0.1);
            backdrop-filter: blur(8px);
            border: none;
            color: white;
            cursor: pointer;
            transition: all 0.2s;
            font-size: 1.25rem;
        }}
        
        .control-btn:hover {{
            background: rgba(255, 255, 255, 0.2);
            transform: scale(1.05);
        }}
        
        .control-btn.active {{
            background: var(--accent);
        }}
        
        .controls-right {{
            display: flex;
            flex-direction: column;
            align-items: flex-end;
            gap: 0.5rem;
        }}
        
        .stats-grid {{
            display: grid;
            grid-template-columns: repeat(2, auto);
            gap: 0.25rem 1rem;
            font-size: 0.7rem;
            color: rgba(255, 255, 255, 0.7);
            background: rgba(0, 0, 0, 0.4);
            padding: 0.5rem 0.75rem;
            border-radius: 6px;
            backdrop-filter: blur(8px);
        }}
        
        .stat-label {{
            color: rgba(255, 255, 255, 0.5);
        }}
        
        .stat-value {{
            font-weight: 500;
            font-variant-numeric: tabular-nums;
        }}
        
        .codec-badge {{
            display: inline-block;
            padding: 0.15rem 0.4rem;
            background: var(--highlight);
            color: white;
            border-radius: 4px;
            font-size: 0.65rem;
            font-weight: 600;
            text-transform: uppercase;
        }}
        
        .viewer-footer {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 0.5rem 1.25rem;
            background: var(--bg-card);
            border-top: 1px solid var(--border);
            flex-shrink: 0;
        }}
        
        .footer-left {{
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
        
        .quick-link {{
            display: flex;
            align-items: center;
            gap: 0.4rem;
            padding: 0.4rem 0.75rem;
            font-size: 0.75rem;
            color: var(--text-secondary);
            text-decoration: none;
            border-radius: 6px;
            transition: all 0.2s;
        }}
        
        .quick-link:hover {{
            background: var(--bg-hover);
            color: var(--text-primary);
        }}
        
        .latency-indicator {{
            display: flex;
            align-items: center;
            gap: 0.4rem;
            font-size: 0.75rem;
            color: var(--text-secondary);
        }}
        
        .latency-dot {{
            width: 6px;
            height: 6px;
            border-radius: 50%;
            background: var(--accent);
        }}
        
        .latency-dot.warning {{
            background: var(--warning);
        }}
        
        .latency-dot.danger {{
            background: var(--danger);
        }}
    </style>
</head>
<body>
    <div class="viewer-container">
        <header class="viewer-header">
            <div class="header-left">
                <a href="/" class="back-button" title="Back to Dashboard">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M19 12H5M12 19l-7-7 7-7"/>
                    </svg>
                </a>
                <div class="camera-info">
                    <span class="camera-name">{camera_name}</span>
                    <div class="camera-status">
                        <span class="status-indicator connecting" id="statusDot"></span>
                        <span id="statusText">Connecting...</span>
                    </div>
                </div>
            </div>
            <div class="header-right">
                <button id="btnFocus" class="focus-btn" onclick="toggleFocus()" title="Request HD focus">🔍 Focus</button>
                <div class="layer-mode" id="layerModeToggle">
                    <button id="btnLayerAuto" class="active" onclick="setLayerMode('auto')" title="Auto quality based on bandwidth">Auto</button>
                    <button id="btnLayerManual" onclick="setLayerMode('manual')" title="Manual quality selection">Manual</button>
                </div>
                <div class="stream-toggle">
                    <button id="btnHigh" class="active" onclick="setLayer('high')" title="High quality (Main stream)">HD</button>
                    <button id="btnMedium" onclick="setLayer('medium')" title="Medium quality">MQ</button>
                    <button id="btnLow" onclick="setLayer('low')" title="Low quality (Sub stream)">SD</button>
                </div>
            </div>
        </header>
        
        <div class="video-wrapper" id="videoWrapper">
            <video id="video" autoplay playsinline muted></video>
            <div class="video-overlay" id="overlay">
                <div class="loading-spinner"></div>
                <div class="overlay-text" id="overlayText">Connecting to camera...</div>
            </div>
            <div class="video-controls">
                <div class="controls-left">
                    <button class="control-btn" id="btnMute" onclick="toggleMute()" title="Toggle Audio">
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M11 5L6 9H2v6h4l5 4V5z"/>
                            <line x1="23" y1="9" x2="17" y2="15"/>
                            <line x1="17" y1="9" x2="23" y2="15"/>
                        </svg>
                    </button>
                    <button class="control-btn" id="btnFullscreen" onclick="toggleFullscreen()" title="Fullscreen">
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M8 3H5a2 2 0 00-2 2v3m18 0V5a2 2 0 00-2-2h-3m0 18h3a2 2 0 002-2v-3M3 16v3a2 2 0 002 2h3"/>
                        </svg>
                    </button>
                    <button class="control-btn" onclick="takeSnapshot()" title="Snapshot">
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M23 19a2 2 0 01-2 2H3a2 2 0 01-2-2V8a2 2 0 012-2h4l2-3h6l2 3h4a2 2 0 012 2z"/>
                            <circle cx="12" cy="13" r="4"/>
                        </svg>
                    </button>
                </div>
                <div class="controls-right">
                    <div class="stats-grid" id="statsGrid">
                        <span class="stat-label">Codec</span>
                        <span class="stat-value" id="codecInfo">--</span>
                        <span class="stat-label">Resolution</span>
                        <span class="stat-value" id="resInfo">--</span>
                        <span class="stat-label">FPS</span>
                        <span class="stat-value" id="fpsInfo">--</span>
                        <span class="stat-label">Bitrate</span>
                        <span class="stat-value" id="bitrateInfo">--</span>
                    </div>
                </div>
            </div>
        </div>
        
        <footer class="viewer-footer">
            <div class="footer-left">
                <a href="/api/timeline-ui/{camera_id}" class="quick-link">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <circle cx="12" cy="12" r="10"/>
                        <polyline points="12 6 12 12 16 14"/>
                    </svg>
                    Timeline
                </a>
                <a href="#" class="quick-link">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/>
                        <line x1="16" y1="2" x2="16" y2="6"/>
                        <line x1="8" y1="2" x2="8" y2="6"/>
                        <line x1="3" y1="10" x2="21" y2="10"/>
                    </svg>
                    Events
                </a>
            </div>
            <div class="latency-indicator">
                <span class="latency-dot" id="latencyDot"></span>
                <span id="latencyText">-- ms</span>
            </div>
        </footer>
    </div>
    
    <script>
        // ==========================================================================
        // WebRTC Data Channel Live Viewer Controller
        // ==========================================================================
        // All video controls happen via WebRTC data channel after connection.
        
        const CAMERA_ID = '{camera_id}';
        const CONFIG = {{
            reconnectDelay: 2000,
            maxReconnectDelay: 30000,
            statsInterval: 1000,
            positionUpdateInterval: 250,
        }};
        
        // State
        let pc = null;
        let dataChannel = null;
        let sessionId = null;
        let isConnected = false;
        let reconnectAttempts = 0;
        let reconnectTimer = null;
        let currentStream = 'main';
        let currentLayer = 'high';
        let layerMode = 'auto';  // 'auto' or 'manual'
        let isFocused = false;   // HD focus state
        let bandwidthInfo = null; // Latest bandwidth budget info
        let lastBytesReceived = 0;
        let lastStatsTime = 0;
        let playbackState = {{
            mode: 'live',
            isPlaying: true,
            speed: 1.0,
            currentTimestamp: null
        }};
        
        const video = document.getElementById('video');
        const overlay = document.getElementById('overlay');
        const overlayText = document.getElementById('overlayText');
        const statusDot = document.getElementById('statusDot');
        const statusText = document.getElementById('statusText');
        
        // ==========================================================================
        // Data Channel Commands
        // ==========================================================================
        
        function sendCommand(cmd) {{
            if (!dataChannel || dataChannel.readyState !== 'open') {{
                console.warn('Data channel not open, cannot send:', cmd);
                return false;
            }}
            console.log('Sending command:', cmd);
            dataChannel.send(JSON.stringify(cmd));
            return true;
        }}
        
        function handleMessage(event) {{
            try {{
                const msg = JSON.parse(event.data);
                console.log('Received message:', msg);
                
                switch (msg.type) {{
                    case 'position':
                        playbackState = {{ ...playbackState, ...msg }};
                        break;
                    case 'stateChange':
                        playbackState = {{ ...playbackState, ...msg.current }};
                        updateLayerButtons();
                        break;
                    case 'status':
                        handleStatus(msg);
                        break;
                    case 'streamChanged':
                        console.log('Stream changed to:', msg.streamType);
                        currentStream = msg.streamType;
                        updateLayerButtons();
                        break;
                    case 'layerChanged':
                        console.log('Layer changed:', msg);
                        currentLayer = msg.activeLayer;
                        currentStream = msg.streamType;
                        updateLayerButtons();
                        // Show brief notification
                        showOverlay(`Quality: ${{msg.activeLayer.toUpperCase()}}`);
                        setTimeout(hideOverlay, 1000);
                        break;
                    case 'layerInfo':
                        console.log('Layer info:', msg);
                        currentLayer = msg.activeLayer;
                        layerMode = msg.mode;
                        currentStream = msg.streamType;
                        updateLayerButtons();
                        break;
                    case 'focusGranted':
                        console.log('Focus granted:', msg);
                        isFocused = true;
                        currentLayer = msg.layer;
                        updateFocusUI();
                        showOverlay(`🎯 HD Focus Active`);
                        setTimeout(hideOverlay, 1500);
                        break;
                    case 'focusDenied':
                        console.warn('Focus denied:', msg.reason);
                        showOverlay(`❌ Focus denied: ${{msg.reason}}`);
                        setTimeout(hideOverlay, 2000);
                        break;
                    case 'focusRevoked':
                        console.log('Focus revoked:', msg.reason);
                        isFocused = false;
                        currentLayer = msg.newLayer;
                        updateFocusUI();
                        showOverlay(`Focus released: ${{msg.reason}}`);
                        setTimeout(hideOverlay, 1500);
                        break;
                    case 'bandwidthBudget':
                        console.log('Bandwidth budget:', msg);
                        bandwidthInfo = msg;
                        updateBandwidthUI();
                        break;
                    case 'buffering':
                        if (msg.isBuffering) {{
                            showOverlay('Buffering...');
                        }} else {{
                            hideOverlay();
                        }}
                        break;
                    case 'error':
                        console.error('Server error:', msg.code, msg.message);
                        showOverlay('Error: ' + msg.message);
                        break;
                    case 'pong':
                        const rtt = Date.now() - new Date(msg.clientTime).getTime();
                        updateLatencyFromRTT(rtt);
                        break;
                }}
            }} catch (e) {{
                console.error('Failed to parse message:', e);
            }}
        }}
        
        function handleStatus(msg) {{
            playbackState = {{
                mode: msg.mode,
                isPlaying: msg.isPlaying,
                speed: msg.speed,
                currentTimestamp: msg.currentTimestamp ? new Date(msg.currentTimestamp) : null
            }};
            if (msg.streamType) {{
                currentStream = msg.streamType;
            }}
            if (msg.activeLayer) {{
                currentLayer = msg.activeLayer;
            }}
            if (msg.layerMode) {{
                layerMode = msg.layerMode;
            }}
            updateLayerButtons();
            if (msg.codec) {{
                document.getElementById('codecInfo').innerHTML = `<span class="codec-badge">${{msg.codec.toUpperCase()}}</span>`;
            }}
            if (msg.width && msg.height) {{
                document.getElementById('resInfo').textContent = `${{msg.width}}×${{msg.height}}`;
            }}
        }}
        
        // ==========================================================================
        // Layer Control (via data channel)
        // ==========================================================================
        
        function setLayer(layer) {{
            if (layer === currentLayer && layerMode === 'manual') return;
            
            // Switching to a specific layer implies manual mode
            layerMode = 'manual';
            
            // Send command via data channel
            if (sendCommand({{ type: 'setLayer', layer: layer }})) {{
                currentLayer = layer;
                updateLayerButtons();
                showOverlay('Switching quality...');
            }}
        }}
        
        function setLayerMode(mode) {{
            if (mode === layerMode) return;
            layerMode = mode;
            
            if (mode === 'auto') {{
                // Send auto preference to server
                sendCommand({{ 
                    type: 'setLayerPreference',
                    preferredLayer: 'auto',
                    mode: 'adaptive'
                }});
            }}
            
            updateLayerButtons();
        }}
        
        function updateLayerButtons() {{
            // Update layer mode buttons
            document.getElementById('btnLayerAuto').classList.toggle('active', layerMode === 'auto' || layerMode === 'adaptive');
            document.getElementById('btnLayerManual').classList.toggle('active', layerMode === 'manual');
            
            // Update quality buttons
            document.getElementById('btnHigh').classList.toggle('active', currentLayer === 'high');
            document.getElementById('btnMedium').classList.toggle('active', currentLayer === 'medium');
            document.getElementById('btnLow').classList.toggle('active', currentLayer === 'low');
            
            // Disable manual layer buttons in auto mode
            const streamToggle = document.querySelector('.stream-toggle');
            if (layerMode === 'auto' || layerMode === 'adaptive') {{
                streamToggle.classList.add('disabled');
            }} else {{
                streamToggle.classList.remove('disabled');
            }}
        }}
        
        // Notify server of UI context changes for adaptive mode
        function notifyUiContext(context) {{
            sendCommand({{ type: 'setUiContext', context: context }});
        }}
        
        // ==========================================================================
        // Focus Control (Multi-Camera HD Switching)
        // ==========================================================================
        
        function requestFocus() {{
            if (isFocused) return;
            if (sendCommand({{ type: 'requestFocus' }})) {{
                showOverlay('Requesting HD focus...');
            }}
        }}
        
        function releaseFocus() {{
            if (!isFocused) return;
            if (sendCommand({{ type: 'releaseFocus' }})) {{
                showOverlay('Releasing focus...');
            }}
        }}
        
        function toggleFocus() {{
            if (isFocused) {{
                releaseFocus();
            }} else {{
                requestFocus();
            }}
        }}
        
        function updateFocusUI() {{
            const focusBtn = document.getElementById('btnFocus');
            if (focusBtn) {{
                focusBtn.classList.toggle('active', isFocused);
                focusBtn.textContent = isFocused ? '🎯 HD' : '🔍 Focus';
                focusBtn.title = isFocused ? 'Release HD focus' : 'Request HD focus';
            }}
        }}
        
        function updateBandwidthUI() {{
            const bwIndicator = document.getElementById('bandwidthIndicator');
            if (bwIndicator && bandwidthInfo) {{
                const pct = Math.round((bandwidthInfo.usedKbps / bandwidthInfo.totalKbps) * 100);
                bwIndicator.textContent = `${{Math.round(bandwidthInfo.usedKbps/1000)}}/${{Math.round(bandwidthInfo.totalKbps/1000)}} Mbps`;
                bwIndicator.classList.toggle('warning', pct > 80);
                bwIndicator.classList.toggle('critical', pct > 95);
            }}
        }}
        
        function getBandwidthBudget() {{
            sendCommand({{ type: 'getBandwidthBudget' }});
        }}
        
        // ==========================================================================
        // WebRTC Connection
        // ==========================================================================
        
        function setStatus(state, text) {{
            statusDot.className = 'status-indicator ' + state;
            statusText.textContent = text;
        }}
        
        function showOverlay(text) {{
            overlayText.textContent = text;
            overlay.classList.remove('hidden');
        }}
        
        function hideOverlay() {{
            overlay.classList.add('hidden');
        }}
        
        async function connect() {{
            if (pc) disconnect();
            
            setStatus('connecting', 'Connecting...');
            showOverlay('Connecting to camera...');
            
            try {{
                pc = new RTCPeerConnection({{
                    iceServers: [{{ urls: 'stun:stun.l.google.com:19302' }}]
                }});
                
                // Create data channel BEFORE creating offer
                dataChannel = pc.createDataChannel('control', {{
                    ordered: true
                }});
                
                dataChannel.onopen = () => {{
                    console.log('Data channel open');
                    // Request initial status
                    sendCommand({{ type: 'getStatus' }});
                    // Set mode to live
                    sendCommand({{ type: 'setMode', mode: 'live' }});
                    // Start ping interval for latency measurement
                    startPingInterval();
                }};
                
                dataChannel.onmessage = handleMessage;
                
                dataChannel.onclose = () => {{
                    console.log('Data channel closed');
                    stopPingInterval();
                }};
                
                dataChannel.onerror = (e) => {{
                    console.error('Data channel error:', e);
                }};
                
                // Add transceivers for receiving media
                pc.addTransceiver('video', {{ direction: 'recvonly' }});
                pc.addTransceiver('audio', {{ direction: 'recvonly' }});
                
                pc.ontrack = (event) => {{
                    console.log('Received track:', event.track.kind);
                    if (event.track.kind === 'video') {{
                        video.srcObject = event.streams[0];
                        video.play().catch(e => console.log('Autoplay blocked:', e));
                    }}
                }};
                
                pc.oniceconnectionstatechange = () => {{
                    console.log('ICE state:', pc.iceConnectionState);
                    if (pc.iceConnectionState === 'connected') {{
                        isConnected = true;
                        reconnectAttempts = 0;
                        setStatus('live', 'Live');
                        hideOverlay();
                    }} else if (pc.iceConnectionState === 'disconnected' || pc.iceConnectionState === 'failed') {{
                        handleDisconnect();
                    }}
                }};
                
                const offer = await pc.createOffer();
                await pc.setLocalDescription(offer);
                
                const resp = await fetch('/api/webrtc/offer', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{
                        cameraId: CAMERA_ID,
                        streamType: currentStream,
                        sdp: offer.sdp
                    }})
                }});
                
                const data = await resp.json();
                if (!data.success) throw new Error(data.error || 'Failed to get answer');
                
                sessionId = data.data.sessionId;
                await pc.setRemoteDescription({{ type: 'answer', sdp: data.data.sdp }});
                
            }} catch (e) {{
                console.error('Connection error:', e);
                handleDisconnect();
            }}
        }}
        
        function disconnect() {{
            stopPingInterval();
            if (dataChannel) {{
                dataChannel.close();
                dataChannel = null;
            }}
            if (pc) {{
                pc.close();
                pc = null;
            }}
            sessionId = null;
            isConnected = false;
        }}
        
        function handleDisconnect() {{
            isConnected = false;
            reconnectAttempts++;
            const delay = Math.min(CONFIG.reconnectDelay * Math.pow(2, reconnectAttempts - 1), CONFIG.maxReconnectDelay);
            
            setStatus('connecting', `Reconnecting in ${{Math.ceil(delay/1000)}}s...`);
            showOverlay(`Connection lost. Reconnecting...`);
            
            if (reconnectTimer) clearTimeout(reconnectTimer);
            reconnectTimer = setTimeout(() => {{
                reconnectTimer = null;
                connect();
            }}, delay);
        }}
        
        // ==========================================================================
        // Ping/Latency Measurement
        // ==========================================================================
        
        let pingInterval = null;
        
        function startPingInterval() {{
            stopPingInterval();
            pingInterval = setInterval(() => {{
                sendCommand({{ type: 'ping', clientTime: new Date().toISOString() }});
            }}, 2000);
        }}
        
        function stopPingInterval() {{
            if (pingInterval) {{
                clearInterval(pingInterval);
                pingInterval = null;
            }}
        }}
        
        function updateLatencyFromRTT(rtt) {{
            const dot = document.getElementById('latencyDot');
            const text = document.getElementById('latencyText');
            
            dot.className = 'latency-dot' + (rtt > 200 ? ' warning' : rtt > 400 ? ' danger' : '');
            text.textContent = `${{Math.round(rtt)}} ms`;
        }}
        
        // ==========================================================================
        // Video Controls
        // ==========================================================================
        
        function toggleMute() {{
            video.muted = !video.muted;
            const btn = document.getElementById('btnMute');
            btn.classList.toggle('active', !video.muted);
        }}
        
        function toggleFullscreen() {{
            const wrapper = document.getElementById('videoWrapper');
            if (document.fullscreenElement) {{
                document.exitFullscreen();
            }} else {{
                wrapper.requestFullscreen();
            }}
        }}
        
        function takeSnapshot() {{
            const canvas = document.createElement('canvas');
            canvas.width = video.videoWidth;
            canvas.height = video.videoHeight;
            canvas.getContext('2d').drawImage(video, 0, 0);
            
            const link = document.createElement('a');
            link.download = `snapshot_${{Date.now()}}.jpg`;
            link.href = canvas.toDataURL('image/jpeg', 0.95);
            link.click();
        }}
        
        // ==========================================================================
        // Stats Collection
        // ==========================================================================
        
        async function updateStats() {{
            if (!pc || pc.iceConnectionState !== 'connected') return;
            
            try {{
                const stats = await pc.getStats();
                const now = Date.now();
                
                stats.forEach(report => {{
                    if (report.type === 'inbound-rtp' && report.kind === 'video') {{
                        // Codec
                        if (report.codecId) {{
                            stats.forEach(s => {{
                                if (s.id === report.codecId && s.mimeType) {{
                                    const codec = s.mimeType.split('/')[1]?.toUpperCase() || 'Unknown';
                                    document.getElementById('codecInfo').innerHTML = `<span class="codec-badge">${{codec}}</span>`;
                                }}
                            }});
                        }}
                        
                        // Resolution
                        if (report.frameWidth && report.frameHeight) {{
                            document.getElementById('resInfo').textContent = `${{report.frameWidth}}×${{report.frameHeight}}`;
                        }}
                        
                        // FPS
                        if (report.framesPerSecond !== undefined) {{
                            document.getElementById('fpsInfo').textContent = `${{report.framesPerSecond.toFixed(0)}}`;
                        }}
                        
                        // Bitrate
                        if (report.bytesReceived !== undefined && lastStatsTime > 0) {{
                            const deltaBytes = report.bytesReceived - lastBytesReceived;
                            const deltaTime = (now - lastStatsTime) / 1000;
                            if (deltaTime > 0) {{
                                const bitrate = (deltaBytes * 8) / deltaTime / 1000;
                                if (bitrate >= 1000) {{
                                    document.getElementById('bitrateInfo').textContent = `${{(bitrate/1000).toFixed(1)}} Mbps`;
                                }} else {{
                                    document.getElementById('bitrateInfo').textContent = `${{bitrate.toFixed(0)}} kbps`;
                                }}
                            }}
                            lastBytesReceived = report.bytesReceived;
                        }}
                        lastStatsTime = now;
                    }}
                }});
            }} catch (e) {{}}
        }}
        
        function updateLatency(bitrate) {{
            // Deprecated - now using RTT from ping/pong
        }}
        
        // ==========================================================================
        // Keyboard Shortcuts
        // ==========================================================================
        
        document.addEventListener('keydown', (e) => {{
            if (e.key === 'f') toggleFullscreen();
            if (e.key === 'm') toggleMute();
            if (e.key === 's') takeSnapshot();
            if (e.key === '1') setLayer('high');   // HD
            if (e.key === '2') setLayer('medium'); // Medium quality
            if (e.key === '3') setLayer('low');    // SD
            if (e.key === 'a') setLayerMode(layerMode === 'auto' ? 'manual' : 'auto'); // Toggle auto
        }});
        
        // Detect fullscreen changes for UI context
        document.addEventListener('fullscreenchange', () => {{
            const context = document.fullscreenElement ? 'fullscreen' : 'grid2x2';
            notifyUiContext(context);
        }});
        
        // ==========================================================================
        // Initialization
        // ==========================================================================
        
        window.addEventListener('load', () => {{
            setTimeout(connect, 100);
            setInterval(updateStats, CONFIG.statsInterval);
            updateLayerButtons();
        }});
        
        window.addEventListener('beforeunload', disconnect);
    </script>
</body>
</html>"##,
        camera_name = camera_name,
        camera_id = camera_id,
        common_styles = get_common_styles()
    )
}

/// Generate the replay/review page HTML
pub fn generate_replay_page(camera_id: &Uuid, camera_name: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{camera_name} - Review</title>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap" rel="stylesheet">
    {common_styles}
    <style>
        .review-container {{
            display: grid;
            grid-template-columns: 1fr 320px;
            grid-template-rows: auto 1fr auto;
            height: 100vh;
            overflow: hidden;
        }}
        
        @media (max-width: 1024px) {{
            .review-container {{
                grid-template-columns: 1fr;
                grid-template-rows: auto 1fr auto auto;
            }}
        }}
        
        .review-header {{
            grid-column: 1 / -1;
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 0.75rem 1.25rem;
            background: var(--bg-card);
            border-bottom: 1px solid var(--border);
        }}
        
        .header-center {{
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
        
        .date-nav {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        
        .date-nav button {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 32px;
            height: 32px;
            border: none;
            background: var(--bg-surface);
            color: var(--text-secondary);
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .date-nav button:hover {{
            background: var(--bg-hover);
            color: var(--text-primary);
        }}
        
        .current-date {{
            font-size: 0.875rem;
            font-weight: 500;
            color: var(--text-primary);
            min-width: 140px;
            text-align: center;
        }}
        
        .playback-area {{
            display: flex;
            flex-direction: column;
            background: #000;
            position: relative;
        }}
        
        .video-player {{
            flex: 1;
            display: flex;
            align-items: center;
            justify-content: center;
            position: relative;
        }}
        
        .video-player video {{
            max-width: 100%;
            max-height: 100%;
            object-fit: contain;
        }}
        
        .playback-controls {{
            display: flex;
            align-items: center;
            gap: 1rem;
            padding: 1rem 1.5rem;
            background: linear-gradient(transparent, rgba(0, 0, 0, 0.95));
        }}
        
        .play-btn {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 48px;
            height: 48px;
            border-radius: 50%;
            background: var(--highlight);
            border: none;
            color: white;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .play-btn:hover {{
            transform: scale(1.05);
            background: #79b8ff;
        }}
        
        .seek-buttons {{
            display: flex;
            gap: 0.5rem;
        }}
        
        .seek-btn {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 36px;
            border-radius: 8px;
            background: rgba(255, 255, 255, 0.1);
            border: none;
            color: white;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .seek-btn:hover {{
            background: rgba(255, 255, 255, 0.2);
        }}
        
        .time-display {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            font-size: 0.875rem;
            color: var(--text-primary);
            font-variant-numeric: tabular-nums;
        }}
        
        .time-display .separator {{
            color: var(--text-muted);
        }}
        
        .speed-selector {{
            margin-left: auto;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        
        .speed-selector select {{
            background: rgba(255, 255, 255, 0.1);
            border: none;
            color: white;
            padding: 0.4rem 0.6rem;
            border-radius: 6px;
            font-size: 0.75rem;
            cursor: pointer;
        }}
        
        .events-sidebar {{
            background: var(--bg-card);
            border-left: 1px solid var(--border);
            display: flex;
            flex-direction: column;
            overflow: hidden;
        }}
        
        @media (max-width: 1024px) {{
            .events-sidebar {{
                border-left: none;
                border-top: 1px solid var(--border);
                max-height: 250px;
            }}
        }}
        
        .sidebar-header {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 1rem;
            border-bottom: 1px solid var(--border);
        }}
        
        .sidebar-title {{
            font-size: 0.875rem;
            font-weight: 600;
            color: var(--text-primary);
        }}
        
        .event-filter {{
            display: flex;
            gap: 0.25rem;
        }}
        
        .filter-btn {{
            padding: 0.25rem 0.5rem;
            font-size: 0.7rem;
            border: none;
            background: var(--bg-surface);
            color: var(--text-secondary);
            border-radius: 4px;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .filter-btn.active {{
            background: var(--highlight);
            color: white;
        }}
        
        .events-list {{
            flex: 1;
            overflow-y: auto;
            padding: 0.5rem;
        }}
        
        .event-card {{
            display: flex;
            gap: 0.75rem;
            padding: 0.75rem;
            background: var(--bg-surface);
            border-radius: 8px;
            margin-bottom: 0.5rem;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .event-card:hover {{
            background: var(--bg-hover);
        }}
        
        .event-card.active {{
            border: 1px solid var(--highlight);
        }}
        
        .event-thumbnail {{
            width: 80px;
            height: 45px;
            border-radius: 4px;
            background: var(--bg-dark);
            object-fit: cover;
        }}
        
        .event-info {{
            flex: 1;
            min-width: 0;
        }}
        
        .event-type {{
            display: flex;
            align-items: center;
            gap: 0.4rem;
            font-size: 0.8rem;
            font-weight: 500;
            color: var(--text-primary);
        }}
        
        .event-badge {{
            display: inline-block;
            padding: 0.1rem 0.35rem;
            font-size: 0.6rem;
            font-weight: 600;
            border-radius: 3px;
            text-transform: uppercase;
        }}
        
        .event-badge.motion {{ background: var(--motion); color: white; }}
        .event-badge.person {{ background: var(--person); color: white; }}
        .event-badge.vehicle {{ background: var(--vehicle); color: white; }}
        
        .event-time {{
            font-size: 0.7rem;
            color: var(--text-secondary);
            margin-top: 0.25rem;
        }}
        
        .timeline-container {{
            grid-column: 1 / -1;
            background: var(--bg-card);
            border-top: 1px solid var(--border);
            padding: 0.75rem 1rem;
        }}
        
        .timeline {{
            position: relative;
            height: 48px;
            background: var(--bg-surface);
            border-radius: 6px;
            overflow: hidden;
        }}
        
        .timeline-ruler {{
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            height: 18px;
            display: flex;
            border-bottom: 1px solid var(--border);
        }}
        
        .timeline-tick {{
            flex: 1;
            border-right: 1px solid var(--border);
            padding: 2px 4px;
            font-size: 0.6rem;
            color: var(--text-muted);
        }}
        
        .timeline-track {{
            position: absolute;
            top: 20px;
            left: 0;
            right: 0;
            height: 12px;
        }}
        
        .timeline-segment {{
            position: absolute;
            height: 100%;
            background: var(--recording);
            border-radius: 2px;
            opacity: 0.6;
        }}
        
        .timeline-segment.cold {{
            background: var(--highlight);
        }}
        
        .timeline-events {{
            position: absolute;
            top: 34px;
            left: 0;
            right: 0;
            height: 12px;
        }}
        
        .timeline-event {{
            position: absolute;
            width: 4px;
            height: 100%;
            border-radius: 2px;
            cursor: pointer;
        }}
        
        .timeline-event.motion {{ background: var(--motion); }}
        .timeline-event.person {{ background: var(--person); }}
        .timeline-event.vehicle {{ background: var(--vehicle); }}
        
        .timeline-playhead {{
            position: absolute;
            top: 0;
            bottom: 0;
            width: 2px;
            background: var(--danger);
            cursor: ew-resize;
            z-index: 10;
        }}
        
        .timeline-playhead::before {{
            content: '';
            position: absolute;
            top: 0;
            left: -5px;
            border-left: 6px solid transparent;
            border-right: 6px solid transparent;
            border-top: 8px solid var(--danger);
        }}
        
        .no-events {{
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            padding: 2rem;
            color: var(--text-muted);
        }}
        
        .no-events svg {{
            width: 48px;
            height: 48px;
            margin-bottom: 1rem;
            opacity: 0.3;
        }}
    </style>
</head>
<body>
    <div class="review-container">
        <header class="review-header">
            <div class="header-left">
                <a href="/viewer/{camera_id}" class="back-button" title="Back to Live View">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M19 12H5M12 19l-7-7 7-7"/>
                    </svg>
                </a>
                <div class="camera-info">
                    <span class="camera-name">{camera_name}</span>
                    <span style="font-size: 0.75rem; color: var(--text-secondary)">Review Mode</span>
                </div>
            </div>
            <div class="header-center">
                <div class="date-nav">
                    <button onclick="prevDay()" title="Previous Day">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M15 18l-6-6 6-6"/>
                        </svg>
                    </button>
                    <span class="current-date" id="currentDate">Loading...</span>
                    <button onclick="nextDay()" title="Next Day">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M9 18l6-6-6-6"/>
                        </svg>
                    </button>
                </div>
            </div>
            <div class="header-right">
                <div class="quality-selector">
                    <span style="font-size: 0.7rem; color: var(--text-muted); margin-right: 0.5rem;">Quality:</span>
                    <div class="stream-toggle" id="qualityToggle">
                        <button id="btnHigh" class="active" onclick="setReplayQuality('high')" title="Original quality">HD</button>
                        <button id="btnLow" onclick="setReplayQuality('low')" title="Fast scrubbing">SD</button>
                    </div>
                </div>
                <a href="/viewer/{camera_id}" class="btn btn-primary" style="font-size: 0.8rem; padding: 0.4rem 0.75rem;">
                    <span style="display: inline-block; width: 6px; height: 6px; background: var(--danger); border-radius: 50%; margin-right: 0.4rem;"></span>
                    Live
                </a>
            </div>
        </header>
        
        <div class="playback-area">
            <div class="video-player">
                <video id="video" playsinline></video>
                <div class="video-overlay" id="overlay">
                    <div class="loading-spinner"></div>
                    <div class="overlay-text">Select a time to begin playback</div>
                </div>
            </div>
            <div class="playback-controls">
                <button class="play-btn" id="playBtn" onclick="togglePlay()">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
                        <path d="M8 5v14l11-7z"/>
                    </svg>
                </button>
                <div class="seek-buttons">
                    <button class="seek-btn" onclick="seek(-10)" title="-10s">
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M1 4v6h6M3.51 15a9 9 0 102.13-9.36L1 10"/>
                            <text x="12" y="16" font-size="8" fill="currentColor" text-anchor="middle">10</text>
                        </svg>
                    </button>
                    <button class="seek-btn" onclick="seek(10)" title="+10s">
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <path d="M23 4v6h-6M20.49 15a9 9 0 11-2.12-9.36L23 10"/>
                            <text x="12" y="16" font-size="8" fill="currentColor" text-anchor="middle">10</text>
                        </svg>
                    </button>
                </div>
                <div class="time-display">
                    <span id="currentTime">00:00:00</span>
                    <span class="separator">/</span>
                    <span id="duration">00:00:00</span>
                </div>
                <div class="speed-selector">
                    <span style="font-size: 0.75rem; color: var(--text-secondary);">Speed:</span>
                    <select id="playbackSpeed" onchange="setSpeed(this.value)">
                        <option value="0.5">0.5×</option>
                        <option value="1" selected>1×</option>
                        <option value="2">2×</option>
                        <option value="4">4×</option>
                        <option value="8">8×</option>
                    </select>
                </div>
            </div>
        </div>
        
        <aside class="events-sidebar">
            <div class="sidebar-header">
                <span class="sidebar-title">Events</span>
                <div class="event-filter">
                    <button class="filter-btn active" data-filter="all">All</button>
                    <button class="filter-btn" data-filter="motion">Motion</button>
                    <button class="filter-btn" data-filter="person">Person</button>
                </div>
            </div>
            <div class="events-list" id="eventsList">
                <div class="no-events">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                        <circle cx="12" cy="12" r="10"/>
                        <path d="M12 6v6l4 2"/>
                    </svg>
                    <span>No events in selected range</span>
                </div>
            </div>
        </aside>
        
        <div class="timeline-container">
            <div class="timeline" id="timeline">
                <div class="timeline-ruler" id="ruler"></div>
                <div class="timeline-track" id="trackContainer"></div>
                <div class="timeline-events" id="eventsContainer"></div>
                <div class="timeline-playhead" id="playhead" style="left: 10%;"></div>
            </div>
        </div>
    </div>
    
    <script>
        // ==========================================================================
        // WebRTC Data Channel Replay Controller
        // ==========================================================================
        // All playback control happens via WebRTC data channel after connection.
        // Protocol: JSON messages over data channel named "control"
        
        const CAMERA_ID = '{camera_id}';
        
        // State
        let pc = null;           // RTCPeerConnection
        let dataChannel = null;  // Data channel for control
        let currentDate = new Date();
        let currentQuality = 'high';  // 'high' or 'low'
        let isScrubbing = false;      // Track scrubbing state
        let playbackState = {{
            mode: 'live',
            isPlaying: false,
            speed: 1.0,
            currentTimestamp: null,
            bufferedRanges: []
        }};
        let timeline = {{
            segments: [],
            events: [],
            earliestTime: null,
            latestTime: null
        }};
        
        // ==========================================================================
        // WebRTC Connection
        // ==========================================================================
        
        async function connect() {{
            showOverlay('Connecting...');
            
            try {{
                pc = new RTCPeerConnection({{
                    iceServers: [{{ urls: 'stun:stun.l.google.com:19302' }}]
                }});
                
                // Create data channel BEFORE creating offer
                dataChannel = pc.createDataChannel('control', {{
                    ordered: true
                }});
                
                dataChannel.onopen = () => {{
                    console.log('Data channel open');
                    hideOverlay();
                    // Request initial timeline data
                    sendCommand({{ type: 'getTimeline', startTime: getDayStart(), endTime: getDayEnd() }});
                    sendCommand({{ type: 'getStatus' }});
                }};
                
                dataChannel.onmessage = handleMessage;
                
                dataChannel.onclose = () => {{
                    console.log('Data channel closed');
                    showOverlay('Disconnected');
                }};
                
                dataChannel.onerror = (e) => {{
                    console.error('Data channel error:', e);
                }};
                
                // Handle video track
                pc.ontrack = (event) => {{
                    console.log('Received track:', event.track.kind);
                    if (event.track.kind === 'video') {{
                        const video = document.getElementById('video');
                        video.srcObject = event.streams[0];
                        video.play().catch(e => console.log('Autoplay blocked:', e));
                    }}
                }};
                
                // ICE handling
                pc.onicecandidate = (event) => {{
                    // In ICE-lite mode, server doesn't need our candidates
                    // but we could send them if needed
                }};
                
                pc.oniceconnectionstatechange = () => {{
                    console.log('ICE state:', pc.iceConnectionState);
                    if (pc.iceConnectionState === 'disconnected' || 
                        pc.iceConnectionState === 'failed') {{
                        showOverlay('Connection lost');
                    }}
                }};
                
                // Create and send offer
                const offer = await pc.createOffer({{
                    offerToReceiveVideo: true,
                    offerToReceiveAudio: false
                }});
                await pc.setLocalDescription(offer);
                
                // Send offer to server
                const response = await fetch('/api/webrtc/offer', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{
                        cameraId: CAMERA_ID,
                        streamType: 'main',
                        sdp: offer.sdp
                    }})
                }});
                
                const data = await response.json();
                if (!data.success) {{
                    throw new Error(data.error || 'Failed to connect');
                }}
                
                // Set remote description
                await pc.setRemoteDescription({{
                    type: 'answer',
                    sdp: data.data.sdp
                }});
                
                console.log('WebRTC connected, session:', data.data.sessionId);
                
            }} catch (e) {{
                console.error('Connection failed:', e);
                showOverlay('Connection failed: ' + e.message);
            }}
        }}
        
        function disconnect() {{
            if (dataChannel) {{
                dataChannel.close();
                dataChannel = null;
            }}
            if (pc) {{
                pc.close();
                pc = null;
            }}
        }}
        
        // ==========================================================================
        // Data Channel Protocol
        // ==========================================================================
        
        function sendCommand(cmd) {{
            if (!dataChannel || dataChannel.readyState !== 'open') {{
                console.warn('Data channel not open, cannot send:', cmd);
                return;
            }}
            console.log('Sending command:', cmd);
            dataChannel.send(JSON.stringify(cmd));
        }}
        
        function handleMessage(event) {{
            try {{
                const msg = JSON.parse(event.data);
                console.log('Received message:', msg);
                
                switch (msg.type) {{
                    case 'position':
                        handlePosition(msg);
                        break;
                    case 'stateChange':
                        handleStateChange(msg);
                        break;
                    case 'seekComplete':
                        handleSeekComplete(msg);
                        break;
                    case 'timeline':
                        handleTimeline(msg);
                        break;
                    case 'keyframes':
                        handleKeyframes(msg);
                        break;
                    case 'events':
                        handleEvents(msg);
                        break;
                    case 'buffering':
                        handleBuffering(msg);
                        break;
                    case 'status':
                        handleStatus(msg);
                        break;
                    case 'pong':
                        handlePong(msg);
                        break;
                    case 'endOfStream':
                        handleEndOfStream();
                        break;
                    case 'gap':
                        handleGap(msg);
                        break;
                    case 'error':
                        handleError(msg);
                        break;
                }}
            }} catch (e) {{
                console.error('Failed to parse message:', e);
            }}
        }}
        
        // ==========================================================================
        // Message Handlers
        // ==========================================================================
        
        function handlePosition(msg) {{
            playbackState.currentTimestamp = new Date(msg.timestamp);
            playbackState.isPlaying = msg.isPlaying;
            playbackState.mode = msg.mode;
            playbackState.speed = msg.speed;
            
            updateTimeDisplay();
            updatePlayhead();
            updatePlayButton();
        }}
        
        function handleStateChange(msg) {{
            console.log('State changed:', msg.previous, '->', msg.current);
            playbackState = {{ ...playbackState, ...msg.current }};
            updateUI();
        }}
        
        function handleSeekComplete(msg) {{
            console.log('Seek complete:', msg.requestedTimestamp, '->', msg.actualTimestamp);
            playbackState.currentTimestamp = new Date(msg.actualTimestamp);
            updateTimeDisplay();
            updatePlayhead();
            hideOverlay();
        }}
        
        function handleTimeline(msg) {{
            timeline.segments = msg.segments || [];
            timeline.earliestTime = msg.earliestAvailable ? new Date(msg.earliestAvailable) : null;
            timeline.latestTime = msg.latestAvailable ? new Date(msg.latestAvailable) : null;
            
            renderTimelineSegments();
        }}
        
        function handleKeyframes(msg) {{
            // Store keyframes for seek hints
            console.log('Received keyframes:', msg.keyframes.length);
        }}
        
        function handleEvents(msg) {{
            timeline.events = msg.events || [];
            renderEvents(timeline.events);
        }}
        
        function handleBuffering(msg) {{
            playbackState.bufferedRanges = msg.bufferedRanges || [];
            if (msg.isBuffering) {{
                showOverlay('Buffering...');
            }} else {{
                hideOverlay();
            }}
        }}
        
        function handleStatus(msg) {{
            playbackState = {{
                mode: msg.mode,
                isPlaying: msg.isPlaying,
                speed: msg.speed,
                currentTimestamp: msg.currentTimestamp ? new Date(msg.currentTimestamp) : null,
                bufferedRanges: msg.bufferedRanges || []
            }};
            updateUI();
        }}
        
        function handlePong(msg) {{
            const rtt = Date.now() - new Date(msg.clientTime).getTime();
            console.log('RTT:', rtt, 'ms');
        }}
        
        function handleEndOfStream() {{
            console.log('End of stream');
            playbackState.isPlaying = false;
            updatePlayButton();
            showOverlay('End of recordings');
        }}
        
        function handleGap(msg) {{
            console.log('Gap in recordings:', msg.gapStart, '->', msg.gapEnd);
            // Optionally show gap indicator
        }}
        
        function handleError(msg) {{
            console.error('Server error:', msg.code, msg.message);
            showOverlay('Error: ' + msg.message);
        }}
        
        // ==========================================================================
        // Playback Controls (via data channel)
        // ==========================================================================
        
        function togglePlay() {{
            if (playbackState.isPlaying) {{
                sendCommand({{ type: 'pause' }});
            }} else {{
                sendCommand({{ type: 'play', speed: playbackState.speed }});
            }}
        }}
        
        function seek(seconds) {{
            const ts = playbackState.currentTimestamp || new Date();
            const newTime = new Date(ts.getTime() + seconds * 1000);
            sendCommand({{ 
                type: 'seek', 
                timestamp: newTime.toISOString(),
                direction: seconds < 0 ? 'backward' : 'forward'
            }});
            showOverlay('Seeking...');
        }}
        
        function seekToTimestamp(timestamp) {{
            sendCommand({{ 
                type: 'seek', 
                timestamp: timestamp.toISOString(),
                direction: 'nearest'
            }});
            showOverlay('Seeking...');
        }}
        
        function setSpeed(speed) {{
            playbackState.speed = parseFloat(speed);
            sendCommand({{ type: 'setSpeed', speed: playbackState.speed }});
        }}
        
        function setMode(mode) {{
            sendCommand({{ type: 'setMode', mode: mode }});
        }}
        
        // ==========================================================================
        // Quality Control (for replay)
        // ==========================================================================
        
        function setReplayQuality(quality) {{
            if (quality === currentQuality) return;
            currentQuality = quality;
            
            // Map to layer
            const layer = quality === 'high' ? 'high' : 'low';
            sendCommand({{ type: 'setLayer', layer: layer }});
            
            // Update UI
            document.getElementById('btnHigh').classList.toggle('active', quality === 'high');
            document.getElementById('btnLow').classList.toggle('active', quality === 'low');
        }}
        
        // Auto-switch to low quality during scrubbing for responsiveness
        function startScrubbing() {{
            if (!isScrubbing && currentQuality === 'high') {{
                isScrubbing = true;
                // Temporarily switch to low quality for fast scrubbing
                sendCommand({{ type: 'setUiContext', context: 'scrubbing' }});
            }}
        }}
        
        function stopScrubbing() {{
            if (isScrubbing) {{
                isScrubbing = false;
                // Restore quality preference
                const layer = currentQuality === 'high' ? 'high' : 'low';
                sendCommand({{ type: 'setLayer', layer: layer }});
            }}
        }}
        
        // ==========================================================================
        // UI Updates
        // ==========================================================================
        
        function updateUI() {{
            updateTimeDisplay();
            updatePlayhead();
            updatePlayButton();
            document.getElementById('playbackSpeed').value = playbackState.speed.toString();
        }}
        
        function updateTimeDisplay() {{
            if (playbackState.currentTimestamp) {{
                document.getElementById('currentTime').textContent = 
                    playbackState.currentTimestamp.toLocaleTimeString();
            }}
        }}
        
        function updatePlayhead() {{
            if (!playbackState.currentTimestamp) return;
            const dayStart = getDayStart();
            const dayEnd = getDayEnd();
            const current = playbackState.currentTimestamp.getTime();
            const percent = ((current - dayStart.getTime()) / (dayEnd.getTime() - dayStart.getTime())) * 100;
            document.getElementById('playhead').style.left = Math.max(0, Math.min(100, percent)) + '%';
        }}
        
        function updatePlayButton() {{
            const btn = document.getElementById('playBtn');
            if (playbackState.isPlaying) {{
                btn.innerHTML = '<svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>';
            }} else {{
                btn.innerHTML = '<svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>';
            }}
        }}
        
        function showOverlay(text) {{
            const overlay = document.getElementById('overlay');
            const textEl = overlay.querySelector('.overlay-text');
            if (textEl) textEl.textContent = text;
            overlay.classList.remove('hidden');
        }}
        
        function hideOverlay() {{
            document.getElementById('overlay').classList.add('hidden');
        }}
        
        // ==========================================================================
        // Date Navigation
        // ==========================================================================
        
        function getDayStart() {{
            const d = new Date(currentDate);
            d.setHours(0, 0, 0, 0);
            return d;
        }}
        
        function getDayEnd() {{
            const d = new Date(currentDate);
            d.setHours(23, 59, 59, 999);
            return d;
        }}
        
        function formatDate(date) {{
            return date.toLocaleDateString('en-US', {{ 
                weekday: 'short', 
                month: 'short', 
                day: 'numeric' 
            }});
        }}
        
        function updateDateDisplay() {{
            document.getElementById('currentDate').textContent = formatDate(currentDate);
        }}
        
        function prevDay() {{
            currentDate.setDate(currentDate.getDate() - 1);
            updateDateDisplay();
            loadTimelineForDay();
        }}
        
        function nextDay() {{
            const tomorrow = new Date();
            tomorrow.setDate(tomorrow.getDate() + 1);
            if (currentDate < tomorrow) {{
                currentDate.setDate(currentDate.getDate() + 1);
                updateDateDisplay();
                loadTimelineForDay();
            }}
        }}
        
        function loadTimelineForDay() {{
            sendCommand({{ 
                type: 'getTimeline', 
                startTime: getDayStart().toISOString(), 
                endTime: getDayEnd().toISOString() 
            }});
            sendCommand({{ 
                type: 'getEvents', 
                startTime: getDayStart().toISOString(), 
                endTime: getDayEnd().toISOString() 
            }});
            renderRuler();
        }}
        
        // ==========================================================================
        // Timeline Rendering
        // ==========================================================================
        
        function renderRuler() {{
            const ruler = document.getElementById('ruler');
            ruler.innerHTML = '';
            for (let i = 0; i < 24; i++) {{
                const tick = document.createElement('div');
                tick.className = 'timeline-tick';
                tick.textContent = `${{i.toString().padStart(2, '0')}}:00`;
                ruler.appendChild(tick);
            }}
        }}
        
        function renderTimelineSegments() {{
            const track = document.getElementById('trackContainer');
            track.innerHTML = '';
            
            const dayStart = getDayStart().getTime();
            const dayDuration = 24 * 60 * 60 * 1000;
            
            for (const seg of timeline.segments) {{
                const start = new Date(seg.startTime).getTime();
                const end = new Date(seg.endTime).getTime();
                
                const left = ((start - dayStart) / dayDuration) * 100;
                const width = ((end - start) / dayDuration) * 100;
                
                const el = document.createElement('div');
                el.className = 'timeline-segment' + (seg.isCold ? ' cold' : '');
                el.style.left = Math.max(0, left) + '%';
                el.style.width = Math.min(100 - left, width) + '%';
                el.title = `${{new Date(seg.startTime).toLocaleTimeString()}} - ${{new Date(seg.endTime).toLocaleTimeString()}}`;
                el.onclick = () => seekToTimestamp(new Date(seg.startTime));
                track.appendChild(el);
            }}
        }}
        
        function renderEvents(evts) {{
            const list = document.getElementById('eventsList');
            
            if (!evts || evts.length === 0) {{
                list.innerHTML = `
                    <div class="no-events">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                            <circle cx="12" cy="12" r="10"/>
                            <path d="M12 6v6l4 2"/>
                        </svg>
                        <span>No events in selected range</span>
                    </div>
                `;
                return;
            }}
            
            list.innerHTML = evts.map(evt => `
                <div class="event-card" onclick="jumpToEvent('${{evt.id}}', '${{evt.timestamp}}')">
                    <div class="event-thumbnail" style="background: var(--bg-dark)"></div>
                    <div class="event-info">
                        <div class="event-type">
                            <span class="event-badge ${{getEventClass(evt.eventType)}}">${{evt.eventType}}</span>
                        </div>
                        <div class="event-time">${{new Date(evt.timestamp).toLocaleTimeString()}}</div>
                    </div>
                </div>
            `).join('');
            
            // Also render on timeline
            renderTimelineEvents(evts);
        }}
        
        function renderTimelineEvents(evts) {{
            const container = document.getElementById('eventsContainer');
            container.innerHTML = '';
            
            const dayStart = getDayStart().getTime();
            const dayDuration = 24 * 60 * 60 * 1000;
            
            for (const evt of evts) {{
                const ts = new Date(evt.timestamp).getTime();
                const left = ((ts - dayStart) / dayDuration) * 100;
                
                if (left < 0 || left > 100) continue;
                
                const el = document.createElement('div');
                el.className = 'timeline-event ' + getEventClass(evt.eventType);
                el.style.left = left + '%';
                el.title = `${{evt.eventType}} at ${{new Date(evt.timestamp).toLocaleTimeString()}}`;
                el.onclick = (e) => {{
                    e.stopPropagation();
                    jumpToEvent(evt.id, evt.timestamp);
                }};
                container.appendChild(el);
            }}
        }}
        
        function getEventClass(type) {{
            if (type && type.includes('person')) return 'person';
            if (type && type.includes('vehicle')) return 'vehicle';
            return 'motion';
        }}
        
        function jumpToEvent(eventId, timestamp) {{
            console.log('Jump to event:', eventId, timestamp);
            if (timestamp) {{
                seekToTimestamp(new Date(timestamp));
            }}
        }}
        
        // ==========================================================================
        // Event Handlers
        // ==========================================================================
        
        // Filter buttons
        document.querySelectorAll('.filter-btn').forEach(btn => {{
            btn.addEventListener('click', () => {{
                document.querySelectorAll('.filter-btn').forEach(b => b.classList.remove('active'));
                btn.classList.add('active');
                const filter = btn.dataset.filter;
                // Request filtered events
                sendCommand({{ 
                    type: 'getEvents', 
                    startTime: getDayStart().toISOString(), 
                    endTime: getDayEnd().toISOString(),
                    eventTypes: filter === 'all' ? null : [filter]
                }});
            }});
        }});
        
        // Playhead dragging with quality switching
        const timelineEl = document.getElementById('timeline');
        const playheadEl = document.getElementById('playhead');
        let dragging = false;
        
        playheadEl.addEventListener('mousedown', () => {{
            dragging = true;
            startScrubbing();  // Switch to low quality for fast scrubbing
        }});
        document.addEventListener('mouseup', () => {{
            if (dragging) {{
                dragging = false;
                stopScrubbing();  // Restore quality
                // Seek to dropped position
                const rect = timelineEl.getBoundingClientRect();
                const percent = parseFloat(playheadEl.style.left) / 100;
                const timestamp = new Date(getDayStart().getTime() + percent * 24 * 60 * 60 * 1000);
                seekToTimestamp(timestamp);
            }}
        }});
        document.addEventListener('mousemove', (e) => {{
            if (!dragging) return;
            const rect = timelineEl.getBoundingClientRect();
            const x = Math.max(0, Math.min(rect.width, e.clientX - rect.left));
            playheadEl.style.left = `${{(x / rect.width) * 100}}%`;
        }});
        
        timelineEl.addEventListener('click', (e) => {{
            if (e.target === playheadEl) return;
            const rect = timelineEl.getBoundingClientRect();
            const x = e.clientX - rect.left;
            const percent = x / rect.width;
            playheadEl.style.left = `${{percent * 100}}%`;
            const timestamp = new Date(getDayStart().getTime() + percent * 24 * 60 * 60 * 1000);
            seekToTimestamp(timestamp);
        }});
        
        // Keyboard shortcuts
        document.addEventListener('keydown', (e) => {{
            if (e.key === ' ') {{ togglePlay(); e.preventDefault(); }}
            if (e.key === 'ArrowLeft') seek(-10);
            if (e.key === 'ArrowRight') seek(10);
            if (e.key === 'j') seek(-10);
            if (e.key === 'l') seek(10);
            if (e.key === 'k') togglePlay();
            if (e.key === ',') seek(-1/30);  // Frame back
            if (e.key === '.') seek(1/30);   // Frame forward
            if (e.key === '1') setReplayQuality('high');  // HD
            if (e.key === '2') setReplayQuality('low');   // SD
        }});
        
        // Cleanup on page unload
        window.addEventListener('beforeunload', disconnect);
        
        // ==========================================================================
        // Initialization
        // ==========================================================================
        
        updateDateDisplay();
        renderRuler();
        connect();
    </script>
</body>
</html>"##,
        camera_name = camera_name,
        camera_id = camera_id,
        common_styles = get_common_styles()
    )
}

/// Generate multi-camera dashboard
pub fn generate_dashboard(cameras: &[(Uuid, String, bool)]) -> String {
    let camera_cards: String = cameras
        .iter()
        .map(|(id, name, online)| {
            let status_class = if *online { "online" } else { "offline" };
            let status_text = if *online { "Online" } else { "Offline" };
            format!(
                r##"<a href="/viewer/{id}" class="camera-card">
                    <div class="camera-preview">
                        <img src="/api/cameras/{id}/snapshot" alt="" loading="lazy" onerror="this.style.display='none'">
                        <div class="preview-overlay">
                            <span class="live-badge">LIVE</span>
                        </div>
                    </div>
                    <div class="camera-card-info">
                        <div class="camera-card-name">{name}</div>
                        <div class="camera-card-status {status_class}">
                            <span class="status-dot"></span>
                            {status_text}
                        </div>
                    </div>
                </a>"##,
                id = id,
                name = name,
                status_class = status_class,
                status_text = status_text
            )
        })
        .collect();

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>PeekabooVault - Dashboard</title>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
    {common_styles}
    <style>
        .dashboard {{
            min-height: 100vh;
        }}
        
        .dashboard-nav {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 1rem 1.5rem;
            background: var(--bg-card);
            border-bottom: 1px solid var(--border);
        }}
        
        .logo {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
        }}
        
        .logo-icon {{
            width: 36px;
            height: 36px;
            background: linear-gradient(135deg, var(--highlight) 0%, var(--accent) 100%);
            border-radius: 8px;
            display: flex;
            align-items: center;
            justify-content: center;
        }}
        
        .logo-text {{
            font-size: 1.25rem;
            font-weight: 700;
            color: var(--text-primary);
        }}
        
        .nav-links {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        
        .nav-link {{
            display: flex;
            align-items: center;
            gap: 0.4rem;
            padding: 0.5rem 0.75rem;
            font-size: 0.875rem;
            color: var(--text-secondary);
            text-decoration: none;
            border-radius: 6px;
            transition: all 0.2s;
        }}
        
        .nav-link:hover {{
            background: var(--bg-hover);
            color: var(--text-primary);
            text-decoration: none;
        }}
        
        .nav-link.active {{
            background: var(--bg-surface);
            color: var(--text-primary);
        }}
        
        .main-content {{
            padding: 1.5rem;
            max-width: 1600px;
            margin: 0 auto;
        }}
        
        .section-header {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 1.25rem;
        }}
        
        .section-title {{
            font-size: 1.125rem;
            font-weight: 600;
            color: var(--text-primary);
        }}
        
        .view-toggle {{
            display: flex;
            background: var(--bg-surface);
            border-radius: 8px;
            padding: 3px;
        }}
        
        .view-toggle button {{
            padding: 0.4rem 0.6rem;
            border: none;
            background: transparent;
            color: var(--text-secondary);
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .view-toggle button.active {{
            background: var(--bg-hover);
            color: var(--text-primary);
        }}
        
        .cameras-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
            gap: 1rem;
        }}
        
        .cameras-grid.compact {{
            grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
        }}
        
        .camera-card {{
            background: var(--bg-card);
            border-radius: 12px;
            overflow: hidden;
            border: 1px solid var(--border);
            transition: all 0.2s;
            text-decoration: none;
        }}
        
        .camera-card:hover {{
            border-color: var(--highlight);
            transform: translateY(-2px);
            box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
            text-decoration: none;
        }}
        
        .camera-preview {{
            position: relative;
            aspect-ratio: 16/9;
            background: var(--bg-dark);
            overflow: hidden;
        }}
        
        .camera-preview img {{
            width: 100%;
            height: 100%;
            object-fit: cover;
        }}
        
        .preview-overlay {{
            position: absolute;
            inset: 0;
            background: linear-gradient(transparent 50%, rgba(0, 0, 0, 0.7));
            opacity: 0;
            transition: opacity 0.2s;
            display: flex;
            align-items: flex-end;
            justify-content: space-between;
            padding: 0.75rem;
        }}
        
        .camera-card:hover .preview-overlay {{
            opacity: 1;
        }}
        
        .live-badge {{
            display: inline-flex;
            align-items: center;
            gap: 0.35rem;
            padding: 0.25rem 0.5rem;
            background: var(--danger);
            color: white;
            font-size: 0.65rem;
            font-weight: 600;
            border-radius: 4px;
            text-transform: uppercase;
            letter-spacing: 0.02em;
        }}
        
        .live-badge::before {{
            content: '';
            width: 6px;
            height: 6px;
            background: white;
            border-radius: 50%;
            animation: pulse 1.5s infinite;
        }}
        
        .camera-card-info {{
            padding: 0.875rem 1rem;
        }}
        
        .camera-card-name {{
            font-size: 0.9rem;
            font-weight: 600;
            color: var(--text-primary);
            margin-bottom: 0.25rem;
        }}
        
        .camera-card-status {{
            display: flex;
            align-items: center;
            gap: 0.4rem;
            font-size: 0.75rem;
            color: var(--text-secondary);
        }}
        
        .camera-card-status .status-dot {{
            width: 6px;
            height: 6px;
            border-radius: 50%;
            background: var(--text-muted);
        }}
        
        .camera-card-status.online .status-dot {{
            background: var(--accent);
        }}
        
        .camera-card-status.offline .status-dot {{
            background: var(--danger);
        }}
        
        .stats-cards {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1rem;
            margin-bottom: 2rem;
        }}
        
        .stat-card {{
            background: var(--bg-card);
            border-radius: 10px;
            padding: 1.25rem;
            border: 1px solid var(--border);
        }}
        
        .stat-card-label {{
            font-size: 0.75rem;
            color: var(--text-secondary);
            margin-bottom: 0.5rem;
        }}
        
        .stat-card-value {{
            font-size: 1.75rem;
            font-weight: 600;
            color: var(--text-primary);
        }}
        
        .stat-card-subtext {{
            font-size: 0.75rem;
            color: var(--text-muted);
            margin-top: 0.25rem;
        }}
        
        .empty-state {{
            grid-column: 1 / -1;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            padding: 4rem 2rem;
            text-align: center;
        }}
        
        .empty-state svg {{
            width: 64px;
            height: 64px;
            color: var(--text-muted);
            margin-bottom: 1.5rem;
            opacity: 0.5;
        }}
        
        .empty-state h3 {{
            font-size: 1.125rem;
            font-weight: 600;
            color: var(--text-primary);
            margin-bottom: 0.5rem;
        }}
        
        .empty-state p {{
            color: var(--text-secondary);
            max-width: 360px;
        }}
    </style>
</head>
<body>
    <div class="dashboard">
        <nav class="dashboard-nav">
            <div class="logo">
                <div class="logo-icon">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2">
                        <path d="M23 19a2 2 0 01-2 2H3a2 2 0 01-2-2V8a2 2 0 012-2h4l2-3h6l2 3h4a2 2 0 012 2z"/>
                        <circle cx="12" cy="13" r="4"/>
                    </svg>
                </div>
                <span class="logo-text">PeekabooVault</span>
            </div>
            <div class="nav-links">
                <a href="/" class="nav-link active">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <rect x="3" y="3" width="7" height="7"/>
                        <rect x="14" y="3" width="7" height="7"/>
                        <rect x="14" y="14" width="7" height="7"/>
                        <rect x="3" y="14" width="7" height="7"/>
                    </svg>
                    Cameras
                </a>
                <a href="/multi-replay" class="nav-link">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <polygon points="5 3 19 12 5 21 5 3"/>
                        <rect x="3" y="3" width="4" height="4"/>
                        <rect x="17" y="3" width="4" height="4"/>
                        <rect x="3" y="17" width="4" height="4"/>
                        <rect x="17" y="17" width="4" height="4"/>
                    </svg>
                    Multi-Replay
                </a>
                <a href="/events" class="nav-link">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M22 11.08V12a10 10 0 11-5.93-9.14"/>
                        <polyline points="22 4 12 14.01 9 11.01"/>
                    </svg>
                    Events
                </a>
                <a href="/storage" class="nav-link">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <ellipse cx="12" cy="5" rx="9" ry="3"/>
                        <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
                        <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
                    </svg>
                    Storage
                </a>
                <a href="/settings" class="nav-link">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <circle cx="12" cy="12" r="3"/>
                        <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 010-2.83 2 2 0 012.83 0l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 114 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 0 2 2 0 010 2.83l-.06.06a1.65 1.65 0 00-.33 1.82V9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/>
                    </svg>
                    Settings
                </a>
            </div>
        </nav>
        
        <main class="main-content">
            <div class="stats-cards">
                <div class="stat-card">
                    <div class="stat-card-label">Cameras Online</div>
                    <div class="stat-card-value" id="camerasOnline">{online_count}</div>
                    <div class="stat-card-subtext">of {total_count} total</div>
                </div>
                <div class="stat-card">
                    <div class="stat-card-label">Recording</div>
                    <div class="stat-card-value" id="recording">{online_count}</div>
                    <div class="stat-card-subtext">active streams</div>
                </div>
                <div class="stat-card">
                    <div class="stat-card-label">Storage Used</div>
                    <div class="stat-card-value" id="storageUsed">--</div>
                    <div class="stat-card-subtext">hot + cold</div>
                </div>
                <div class="stat-card">
                    <div class="stat-card-label">Events Today</div>
                    <div class="stat-card-value" id="eventsToday">--</div>
                    <div class="stat-card-subtext">motion, objects</div>
                </div>
            </div>
            
            <div class="section-header">
                <h2 class="section-title">Cameras</h2>
                <div class="section-actions">
                    <button class="btn btn-primary" onclick="showAddCameraModal()">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 0.5rem;">
                            <line x1="12" y1="5" x2="12" y2="19"/>
                            <line x1="5" y1="12" x2="19" y2="12"/>
                        </svg>
                        Add Camera
                    </button>
                    <button class="btn btn-secondary" onclick="scanNetwork()" id="scanBtn">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 0.5rem;">
                            <circle cx="11" cy="11" r="8"/>
                            <path d="m21 21-4.35-4.35"/>
                        </svg>
                        Scan Network
                    </button>
                    <div class="view-toggle">
                    <button class="active" onclick="setView('normal')" title="Normal">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <rect x="3" y="3" width="7" height="7"/>
                            <rect x="14" y="3" width="7" height="7"/>
                            <rect x="14" y="14" width="7" height="7"/>
                            <rect x="3" y="14" width="7" height="7"/>
                        </svg>
                    </button>
                    <button onclick="setView('compact')" title="Compact">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                            <rect x="3" y="3" width="5" height="5"/>
                            <rect x="10" y="3" width="5" height="5"/>
                            <rect x="17" y="3" width="5" height="5"/>
                            <rect x="3" y="10" width="5" height="5"/>
                            <rect x="10" y="10" width="5" height="5"/>
                            <rect x="17" y="10" width="5" height="5"/>
                        </svg>
                    </button>
                </div>
            </div>
            
            <div class="cameras-grid" id="camerasGrid">
                {camera_cards}
            </div>
        </main>
    </div>
    
    <script>
        function setView(type) {{
            const grid = document.getElementById('camerasGrid');
            const buttons = document.querySelectorAll('.view-toggle button');
            
            buttons.forEach((btn, i) => {{
                btn.classList.toggle('active', (type === 'normal' && i === 0) || (type === 'compact' && i === 1));
            }});
            
            grid.classList.toggle('compact', type === 'compact');
        }}
        
        // Load storage stats
        async function loadStats() {{
            try {{
                const resp = await fetch('/api/storage');
                const data = await resp.json();
                if (data.success && data.data) {{
                    const total = data.data.hot_bytes + data.data.cold_bytes;
                    document.getElementById('storageUsed').textContent = formatBytes(total);
                }}
            }} catch (e) {{}}
        }}
        
        function formatBytes(bytes) {{
            if (bytes === 0) return '0 B';
            const k = 1024;
            const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
            const i = Math.floor(Math.log(bytes) / Math.log(k));
            return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
        }}
        
        loadStats();
    </script>
</body>
</html>"##,
        common_styles = get_common_styles(),
        camera_cards = camera_cards,
        online_count = cameras.iter().filter(|(_, _, online)| *online).count(),
        total_count = cameras.len()
    )
}

/// Common CSS styles shared across all pages
fn get_common_styles() -> &'static str {
    r#"<style>
        :root {
            --bg-dark: #0d1117;
            --bg-card: #161b22;
            --bg-surface: #21262d;
            --bg-hover: #30363d;
            --accent: #238636;
            --accent-hover: #2ea043;
            --highlight: #58a6ff;
            --danger: #f85149;
            --warning: #d29922;
            --text-primary: #e6edf3;
            --text-secondary: #8b949e;
            --text-muted: #6e7681;
            --border: #30363d;
            --recording: #238636;
            --motion: #58a6ff;
            --person: #f85149;
            --vehicle: #d29922;
        }
        
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }
        
        body {
            font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: var(--bg-dark);
            color: var(--text-primary);
            line-height: 1.5;
        }
        
        a {
            color: var(--highlight);
            text-decoration: none;
        }
        
        a:hover {
            text-decoration: underline;
        }
        
        .btn {
            display: inline-flex;
            align-items: center;
            justify-content: center;
            padding: 0.5rem 1rem;
            font-size: 0.875rem;
            font-weight: 500;
            border: none;
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.2s;
        }
        
        .btn-primary {
            background: var(--accent);
            color: white;
        }
        
        .btn-primary:hover {
            background: var(--accent-hover);
        }
        
        .btn-secondary {
            background: var(--bg-surface);
            color: var(--text-primary);
        }
        
        .btn-secondary:hover {
            background: var(--bg-hover);
        }
        
        .loading-spinner {
            width: 48px;
            height: 48px;
            border: 3px solid var(--bg-surface);
            border-top-color: var(--highlight);
            border-radius: 50%;
            animation: spin 1s linear infinite;
        }
        
        @keyframes spin {
            to { transform: rotate(360deg); }
        }
        
        @keyframes pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.5; }
        }
        
        /* Scrollbar styling */
        ::-webkit-scrollbar {
            width: 8px;
            height: 8px;
        }
        
        ::-webkit-scrollbar-track {
            background: var(--bg-dark);
        }
        
        ::-webkit-scrollbar-thumb {
            background: var(--bg-hover);
            border-radius: 4px;
        }
        
        ::-webkit-scrollbar-thumb:hover {
            background: var(--text-muted);
        }
    </style>"#
}
/// Camera info for multi-replay page
pub struct ReplayCameraSlot {
    pub id: Uuid,
    pub name: String,
    pub has_recordings: bool,
}

/// Generate multi-camera replay page
/// 
/// Shows a grid of up to 9 cameras with synchronized playback.
/// Clicking a camera focuses it (switches to main stream).
pub fn generate_multi_replay_page(cameras: &[ReplayCameraSlot], start_time: &str, end_time: &str) -> String {
    // Generate camera grid slots
    let camera_slots: String = cameras
        .iter()
        .enumerate()
        .map(|(idx, cam)| {
            let has_rec_class = if cam.has_recordings { "" } else { "no-recording" };
            format!(
                r##"<div class="camera-slot {has_rec_class}" data-camera-id="{id}" data-slot="{idx}">
                    <div class="slot-video-container">
                        <video id="video-{idx}" autoplay muted playsinline></video>
                        <div class="slot-overlay">
                            <div class="slot-name">{name}</div>
                            <div class="slot-status">
                                <span class="slot-indicator"></span>
                                <span class="status-text">Loading...</span>
                            </div>
                        </div>
                        <div class="slot-gap-indicator" style="display:none;">
                            <span>⏸ No recording</span>
                        </div>
                        <button class="focus-btn" onclick="focusCamera('{id}')" title="Focus this camera">
                            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                                <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/>
                            </svg>
                        </button>
                    </div>
                </div>"##,
                id = cam.id,
                idx = idx,
                name = cam.name,
                has_rec_class = has_rec_class
            )
        })
        .collect();
    
    // Generate camera IDs array for JavaScript
    let camera_ids_js: String = cameras
        .iter()
        .map(|c| format!("\"{}\"", c.id))
        .collect::<Vec<_>>()
        .join(", ");
    
    // Determine grid layout based on camera count
    let grid_class = match cameras.len() {
        1 => "grid-1",
        2 => "grid-2",
        3..=4 => "grid-4",
        5..=6 => "grid-6",
        _ => "grid-9",
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
    <title>Multi-Camera Replay</title>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
    {common_styles}
    <style>
        .replay-container {{
            display: flex;
            flex-direction: column;
            height: 100vh;
            overflow: hidden;
            background: var(--bg-dark);
        }}
        
        /* Header */
        .replay-header {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 0.75rem 1.25rem;
            background: var(--bg-card);
            border-bottom: 1px solid var(--border);
            flex-shrink: 0;
            z-index: 100;
        }}
        
        .header-left {{
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
        
        .back-btn {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 36px;
            border-radius: 8px;
            background: transparent;
            border: none;
            color: var(--text-secondary);
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .back-btn:hover {{
            background: var(--bg-hover);
            color: var(--text-primary);
        }}
        
        .page-title {{
            font-size: 1rem;
            font-weight: 600;
            color: var(--text-primary);
        }}
        
        .header-center {{
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
        
        .time-range {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            font-size: 0.875rem;
            color: var(--text-secondary);
        }}
        
        .current-time {{
            font-size: 1rem;
            font-weight: 600;
            color: var(--highlight);
            font-variant-numeric: tabular-nums;
            min-width: 160px;
            text-align: center;
        }}
        
        .header-right {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
        }}
        
        /* Main content area */
        .main-area {{
            flex: 1;
            display: flex;
            flex-direction: column;
            overflow: hidden;
            position: relative;
        }}
        
        /* Camera grid */
        .camera-grid {{
            flex: 1;
            display: grid;
            gap: 4px;
            padding: 4px;
            background: #000;
        }}
        
        .camera-grid.grid-1 {{
            grid-template-columns: 1fr;
        }}
        
        .camera-grid.grid-2 {{
            grid-template-columns: repeat(2, 1fr);
        }}
        
        .camera-grid.grid-4 {{
            grid-template-columns: repeat(2, 1fr);
            grid-template-rows: repeat(2, 1fr);
        }}
        
        .camera-grid.grid-6 {{
            grid-template-columns: repeat(3, 1fr);
            grid-template-rows: repeat(2, 1fr);
        }}
        
        .camera-grid.grid-9 {{
            grid-template-columns: repeat(3, 1fr);
            grid-template-rows: repeat(3, 1fr);
        }}
        
        /* Focus mode - single camera view */
        .camera-grid.focus-mode {{
            grid-template-columns: 1fr !important;
            grid-template-rows: 1fr !important;
        }}
        
        .camera-grid.focus-mode .camera-slot {{
            display: none;
        }}
        
        .camera-grid.focus-mode .camera-slot.focused {{
            display: block;
        }}
        
        /* Camera slot */
        .camera-slot {{
            position: relative;
            background: #111;
            border-radius: 4px;
            overflow: hidden;
        }}
        
        .camera-slot.no-recording {{
            opacity: 0.5;
        }}
        
        .camera-slot:hover .focus-btn {{
            opacity: 1;
        }}
        
        .slot-video-container {{
            position: relative;
            width: 100%;
            height: 100%;
        }}
        
        .camera-slot video {{
            width: 100%;
            height: 100%;
            object-fit: contain;
            background: #000;
        }}
        
        .slot-overlay {{
            position: absolute;
            bottom: 0;
            left: 0;
            right: 0;
            padding: 0.5rem 0.75rem;
            background: linear-gradient(transparent, rgba(0,0,0,0.8));
            pointer-events: none;
        }}
        
        .slot-name {{
            font-size: 0.75rem;
            font-weight: 600;
            color: white;
            text-shadow: 0 1px 2px rgba(0,0,0,0.8);
        }}
        
        .slot-status {{
            display: flex;
            align-items: center;
            gap: 0.35rem;
            font-size: 0.65rem;
            color: var(--text-secondary);
            margin-top: 0.2rem;
        }}
        
        .slot-indicator {{
            width: 6px;
            height: 6px;
            border-radius: 50%;
            background: var(--text-muted);
        }}
        
        .slot-indicator.active {{
            background: var(--accent);
        }}
        
        .slot-indicator.gap {{
            background: var(--warning);
        }}
        
        .slot-gap-indicator {{
            position: absolute;
            top: 50%;
            left: 50%;
            transform: translate(-50%, -50%);
            padding: 0.5rem 1rem;
            background: rgba(0,0,0,0.8);
            border-radius: 8px;
            color: var(--warning);
            font-size: 0.875rem;
        }}
        
        .focus-btn {{
            position: absolute;
            top: 0.5rem;
            right: 0.5rem;
            width: 32px;
            height: 32px;
            border-radius: 6px;
            background: rgba(0,0,0,0.6);
            border: none;
            color: white;
            cursor: pointer;
            opacity: 0;
            transition: all 0.2s;
            display: flex;
            align-items: center;
            justify-content: center;
        }}
        
        .focus-btn:hover {{
            background: var(--highlight);
        }}
        
        /* Controls bar */
        .controls-bar {{
            display: flex;
            align-items: center;
            gap: 1rem;
            padding: 0.75rem 1.25rem;
            background: var(--bg-card);
            border-top: 1px solid var(--border);
            flex-shrink: 0;
        }}
        
        .playback-controls {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        
        .ctrl-btn {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 40px;
            height: 40px;
            border-radius: 50%;
            background: var(--bg-surface);
            border: none;
            color: var(--text-primary);
            cursor: pointer;
            transition: all 0.2s;
        }}
        
        .ctrl-btn:hover {{
            background: var(--bg-hover);
        }}
        
        .ctrl-btn.play-pause {{
            width: 48px;
            height: 48px;
            background: var(--highlight);
            color: white;
        }}
        
        .ctrl-btn.play-pause:hover {{
            background: #79b8ff;
        }}
        
        .skip-btns {{
            display: flex;
            gap: 0.25rem;
        }}
        
        .skip-btn {{
            display: flex;
            align-items: center;
            justify-content: center;
            width: 36px;
            height: 36px;
            border-radius: 6px;
            background: var(--bg-surface);
            border: none;
            color: var(--text-secondary);
            cursor: pointer;
            font-size: 0.7rem;
            font-weight: 600;
            transition: all 0.2s;
        }}
        
        .skip-btn:hover {{
            background: var(--bg-hover);
            color: var(--text-primary);
        }}
        
        .speed-control {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        
        .speed-control label {{
            font-size: 0.75rem;
            color: var(--text-secondary);
        }}
        
        .speed-select {{
            padding: 0.35rem 0.5rem;
            background: var(--bg-surface);
            border: 1px solid var(--border);
            border-radius: 6px;
            color: var(--text-primary);
            font-size: 0.75rem;
            cursor: pointer;
        }}
        
        /* Timeline */
        .timeline-container {{
            flex: 0 0 auto;
            padding: 0.75rem 1.25rem;
            background: var(--bg-card);
            border-top: 1px solid var(--border);
        }}
        
        .timeline {{
            position: relative;
            height: 60px;
            background: var(--bg-surface);
            border-radius: 6px;
            overflow: hidden;
            cursor: pointer;
        }}
        
        .timeline-ruler {{
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            height: 20px;
            display: flex;
            justify-content: space-between;
            padding: 0 8px;
            font-size: 0.6rem;
            color: var(--text-muted);
            border-bottom: 1px solid var(--border);
        }}
        
        .timeline-tracks {{
            position: absolute;
            top: 20px;
            left: 0;
            right: 0;
            bottom: 0;
            padding: 4px 0;
        }}
        
        .timeline-track {{
            height: 6px;
            margin: 2px 8px;
            background: var(--bg-hover);
            border-radius: 2px;
            position: relative;
        }}
        
        .track-segment {{
            position: absolute;
            height: 100%;
            background: var(--accent);
            border-radius: 2px;
        }}
        
        .timeline-playhead {{
            position: absolute;
            top: 0;
            bottom: 0;
            width: 2px;
            background: var(--highlight);
            transform: translateX(-50%);
            z-index: 10;
            pointer-events: none;
        }}
        
        .timeline-playhead::before {{
            content: '';
            position: absolute;
            top: 0;
            left: 50%;
            transform: translateX(-50%);
            width: 0;
            height: 0;
            border-left: 6px solid transparent;
            border-right: 6px solid transparent;
            border-top: 8px solid var(--highlight);
        }}
        
        /* Focus exit button (shown in focus mode) */
        .exit-focus-btn {{
            position: absolute;
            top: 1rem;
            left: 1rem;
            padding: 0.5rem 1rem;
            background: rgba(0,0,0,0.8);
            border: none;
            border-radius: 8px;
            color: white;
            font-size: 0.875rem;
            cursor: pointer;
            z-index: 50;
            display: none;
            align-items: center;
            gap: 0.5rem;
            transition: all 0.2s;
        }}
        
        .exit-focus-btn:hover {{
            background: rgba(0,0,0,0.9);
        }}
        
        .camera-grid.focus-mode + .exit-focus-btn,
        .main-area:has(.focus-mode) .exit-focus-btn {{
            display: flex;
        }}
        
        /* Connection status */
        .connection-status {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            padding: 0.35rem 0.75rem;
            background: var(--bg-surface);
            border-radius: 6px;
            font-size: 0.75rem;
            color: var(--text-secondary);
        }}
        
        .connection-status.connected {{
            color: var(--accent);
        }}
        
        .connection-status.connecting {{
            color: var(--warning);
        }}
        
        .connection-status.error {{
            color: var(--danger);
        }}
    </style>
</head>
<body>
    <div class="replay-container">
        <header class="replay-header">
            <div class="header-left">
                <button class="back-btn" onclick="window.location.href='/'">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M19 12H5M12 19l-7-7 7-7"/>
                    </svg>
                </button>
                <span class="page-title">Multi-Camera Replay</span>
            </div>
            <div class="header-center">
                <div class="current-time" id="currentTime">--:--:--</div>
            </div>
            <div class="header-right">
                <div class="connection-status" id="connectionStatus">
                    <span class="status-dot"></span>
                    <span>Connecting...</span>
                </div>
            </div>
        </header>
        
        <div class="main-area">
            <div class="camera-grid {grid_class}" id="cameraGrid">
                {camera_slots}
            </div>
            <button class="exit-focus-btn" id="exitFocusBtn" onclick="exitFocus()">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M8 3v3a2 2 0 0 1-2 2H3m18 0h-3a2 2 0 0 1-2-2V3m0 18v-3a2 2 0 0 1 2-2h3M3 16h3a2 2 0 0 1 2 2v3"/>
                </svg>
                Exit Focus
            </button>
        </div>
        
        <div class="controls-bar">
            <div class="playback-controls">
                <div class="skip-btns">
                    <button class="skip-btn" onclick="skip(-60)">-60s</button>
                    <button class="skip-btn" onclick="skip(-10)">-10s</button>
                </div>
                
                <button class="ctrl-btn play-pause" id="playPauseBtn" onclick="togglePlayPause()">
                    <svg id="playIcon" width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
                        <path d="M8 5v14l11-7z"/>
                    </svg>
                    <svg id="pauseIcon" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" style="display:none">
                        <path d="M6 4h4v16H6zM14 4h4v16h-4z"/>
                    </svg>
                </button>
                
                <div class="skip-btns">
                    <button class="skip-btn" onclick="skip(10)">+10s</button>
                    <button class="skip-btn" onclick="skip(60)">+60s</button>
                </div>
            </div>
            
            <div class="speed-control">
                <label>Speed:</label>
                <select class="speed-select" id="speedSelect" onchange="setSpeed(this.value)">
                    <option value="0.25">0.25x</option>
                    <option value="0.5">0.5x</option>
                    <option value="1" selected>1x</option>
                    <option value="2">2x</option>
                    <option value="4">4x</option>
                    <option value="8">8x</option>
                </select>
            </div>
        </div>
        
        <div class="timeline-container">
            <div class="timeline" id="timeline" onclick="onTimelineClick(event)">
                <div class="timeline-ruler" id="timelineRuler"></div>
                <div class="timeline-tracks" id="timelineTracks"></div>
                <div class="timeline-playhead" id="playhead" style="left: 0%"></div>
            </div>
        </div>
    </div>
    
    <script>
        // ==========================================================================
        // Multi-Camera Replay Controller - SINGLE SESSION, MULTIPLE TRACKS
        // ==========================================================================
        //
        // Architecture:
        // - ONE RTCPeerConnection with up to 9 video tracks (one per camera)
        // - Each track has 2 simulcast layers (high/low)
        // - ONE data channel for all control commands
        // - Server manages single playhead for synchronized playback
        //
        
        const CAMERA_IDS = [{camera_ids_js}];
        const START_TIME = new Date('{start_time}');
        const END_TIME = new Date('{end_time}');
        
        // State
        let isPlaying = false;
        let currentTime = new Date(START_TIME);
        let playbackSpeed = 1.0;
        let focusedTrack = null; // Track index, not camera ID
        let sessionId = null;
        
        // Single PeerConnection for ALL cameras
        let peerConnection = null;
        let dataChannel = null;
        
        // Track index -> MediaStream mapping
        let trackStreams = {{}};
        
        // ==========================================================================
        // Single WebRTC Connection with Multiple Tracks
        // ==========================================================================
        
        async function connect() {{
            console.log('Connecting multi-camera replay session...');
            updateConnectionStatus('connecting', 'Connecting...');
            
            try {{
                // Create single peer connection
                peerConnection = new RTCPeerConnection({{
                    iceServers: [{{ urls: 'stun:stun.l.google.com:19302' }}]
                }});
                
                // Add receive-only transceivers for each camera (video + audio)
                // Each camera gets its own track
                for (let i = 0; i < CAMERA_IDS.length; i++) {{
                    // Video track for this camera
                    const videoTransceiver = peerConnection.addTransceiver('video', {{
                        direction: 'recvonly'
                    }});
                    
                    // Audio track for this camera (optional, muted in grid)
                    const audioTransceiver = peerConnection.addTransceiver('audio', {{
                        direction: 'recvonly'
                    }});
                }}
                
                // Handle incoming tracks
                let trackIndex = 0;
                peerConnection.ontrack = (event) => {{
                    const track = event.track;
                    console.log(`Track received: kind=${{track.kind}}, id=${{track.id}}`);
                    
                    if (track.kind === 'video') {{
                        // Determine which camera slot this belongs to
                        // Server sends tracks in order: cam0, cam1, cam2...
                        const slotIndex = Math.floor(trackIndex / 2); // 2 tracks per camera (video+audio)
                        
                        const video = document.getElementById(`video-${{slotIndex}}`);
                        if (video) {{
                            // Create or update stream for this slot
                            if (!trackStreams[slotIndex]) {{
                                trackStreams[slotIndex] = new MediaStream();
                            }}
                            trackStreams[slotIndex].addTrack(track);
                            video.srcObject = trackStreams[slotIndex];
                            
                            const slotEl = document.querySelector(`[data-slot="${{slotIndex}}"]`);
                            updateSlotStatus(slotEl, 'active', 'Playing');
                        }}
                        trackIndex++;
                    }}
                }};
                
                // Handle data channel from server
                peerConnection.ondatachannel = (event) => {{
                    dataChannel = event.channel;
                    console.log('Data channel opened');
                    
                    dataChannel.onmessage = (e) => {{
                        handleMessage(JSON.parse(e.data));
                    }};
                    
                    dataChannel.onopen = () => {{
                        console.log('Data channel ready');
                        // Request initial state
                        sendCommand({{ type: 'getState' }});
                    }};
                }};
                
                // Handle ICE candidates
                peerConnection.onicecandidate = async (event) => {{
                    if (event.candidate && sessionId) {{
                        await fetch('/api/webrtc/ice-candidate', {{
                            method: 'POST',
                            headers: {{ 'Content-Type': 'application/json' }},
                            body: JSON.stringify({{
                                sessionId: sessionId,
                                candidate: event.candidate
                            }})
                        }});
                    }}
                }};
                
                // Handle connection state changes
                peerConnection.onconnectionstatechange = () => {{
                    console.log(`Connection state: ${{peerConnection.connectionState}}`);
                    switch (peerConnection.connectionState) {{
                        case 'connected':
                            updateConnectionStatus('connected', `${{CAMERA_IDS.length}} cameras`);
                            break;
                        case 'disconnected':
                        case 'failed':
                            updateConnectionStatus('error', 'Disconnected');
                            // Try to reconnect
                            setTimeout(connect, 3000);
                            break;
                    }}
                }};
                
                // Create offer
                const offer = await peerConnection.createOffer();
                await peerConnection.setLocalDescription(offer);
                
                // Send multi-replay offer to server
                const response = await fetch('/api/webrtc/multi-replay', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{
                        cameraIds: CAMERA_IDS,
                        sdpOffer: offer.sdp,
                        mode: 'replay',
                        startTime: currentTime.toISOString(),
                        endTime: END_TIME.toISOString()
                    }})
                }});
                
                const result = await response.json();
                if (result.success) {{
                    sessionId = result.data.sessionId;
                    await peerConnection.setRemoteDescription({{
                        type: 'answer',
                        sdp: result.data.sdpAnswer
                    }});
                    
                    updateConnectionStatus('connected', `${{CAMERA_IDS.length}} cameras`);
                    console.log(`Connected! Session: ${{sessionId}}`);
                }} else {{
                    throw new Error(result.error || 'Failed to connect');
                }}
                
            }} catch (err) {{
                console.error('Connection failed:', err);
                updateConnectionStatus('error', 'Connection failed');
            }}
        }}
        
        function updateConnectionStatus(state, text) {{
            const el = document.getElementById('connectionStatus');
            el.className = 'connection-status ' + state;
            el.innerHTML = `
                <span class="status-dot"></span>
                <span>${{text}}</span>
            `;
        }}
        
        function updateSlotStatus(slotEl, state, text) {{
            if (!slotEl) return;
            const indicator = slotEl.querySelector('.slot-indicator');
            const statusText = slotEl.querySelector('.status-text');
            
            if (indicator) {{
                indicator.className = 'slot-indicator';
                if (state === 'active') indicator.classList.add('active');
                if (state === 'gap') indicator.classList.add('gap');
            }}
            if (statusText) statusText.textContent = text;
        }}
        
        // ==========================================================================
        // Data Channel Communication
        // ==========================================================================
        
        function sendCommand(cmd) {{
            if (dataChannel && dataChannel.readyState === 'open') {{
                dataChannel.send(JSON.stringify(cmd));
            }} else {{
                console.warn('Data channel not ready, command dropped:', cmd);
            }}
        }}
        
        function handleMessage(msg) {{
            console.log('Message:', msg.type, msg);
            
            switch (msg.type) {{
                case 'position':
                    // Server position update - sync our local state
                    currentTime = new Date(msg.timestamp);
                    isPlaying = msg.isPlaying;
                    playbackSpeed = msg.speed;
                    updateTimeDisplay();
                    updatePlayheadPosition();
                    updatePlayPauseButton();
                    break;
                    
                case 'state':
                    // Full state update
                    currentTime = new Date(msg.currentTime);
                    isPlaying = msg.isPlaying;
                    playbackSpeed = msg.speed;
                    
                    // Update focused camera
                    if (msg.focusedCamera) {{
                        const trackIdx = CAMERA_IDS.indexOf(msg.focusedCamera);
                        if (trackIdx >= 0) focusTrack(trackIdx, false);
                    }} else {{
                        exitFocusInternal();
                    }}
                    
                    // Update per-camera states
                    msg.cameraStates.forEach((state, idx) => {{
                        const slotEl = document.querySelector(`[data-slot="${{idx}}"]`);
                        if (state.inGap) {{
                            updateSlotStatus(slotEl, 'gap', 'No recording');
                            showGapIndicator(idx, true);
                        }} else {{
                            updateSlotStatus(slotEl, 'active', 'Playing');
                            showGapIndicator(idx, false);
                        }}
                    }});
                    
                    updateUI();
                    break;
                    
                case 'focusChanged':
                    if (msg.cameraId) {{
                        const trackIdx = CAMERA_IDS.indexOf(msg.cameraId);
                        if (trackIdx >= 0) focusTrack(trackIdx, false);
                    }} else {{
                        exitFocusInternal();
                    }}
                    break;
                    
                case 'gapStatus':
                    const gapTrackIdx = CAMERA_IDS.indexOf(msg.cameraId);
                    if (gapTrackIdx >= 0) {{
                        const slotEl = document.querySelector(`[data-slot="${{gapTrackIdx}}"]`);
                        if (msg.inGap) {{
                            updateSlotStatus(slotEl, 'gap', 'No recording');
                            showGapIndicator(gapTrackIdx, true);
                        }} else {{
                            updateSlotStatus(slotEl, 'active', 'Playing');
                            showGapIndicator(gapTrackIdx, false);
                        }}
                    }}
                    break;
                    
                case 'seekComplete':
                    console.log(`Seek complete: requested=${{msg.requested}}, actual=${{msg.actual}}`);
                    currentTime = new Date(msg.actual);
                    updateTimeDisplay();
                    updatePlayheadPosition();
                    break;
                    
                case 'endOfRecordings':
                    isPlaying = false;
                    updatePlayPauseButton();
                    console.log('End of recordings reached');
                    break;
                    
                case 'error':
                    console.error('Server error:', msg.code, msg.message);
                    break;
            }}
        }}
        
        function showGapIndicator(trackIdx, show) {{
            const slotEl = document.querySelector(`[data-slot="${{trackIdx}}"]`);
            if (slotEl) {{
                const gapIndicator = slotEl.querySelector('.slot-gap-indicator');
                if (gapIndicator) gapIndicator.style.display = show ? 'flex' : 'none';
            }}
        }}
        
        // ==========================================================================
        // Playback Control
        // ==========================================================================
        
        function togglePlayPause() {{
            isPlaying = !isPlaying;
            updatePlayPauseButton();
            
            sendCommand(isPlaying 
                ? {{ type: 'play', speed: playbackSpeed }}
                : {{ type: 'pause' }}
            );
        }}
        
        function updatePlayPauseButton() {{
            document.getElementById('playIcon').style.display = isPlaying ? 'none' : 'block';
            document.getElementById('pauseIcon').style.display = isPlaying ? 'block' : 'none';
        }}
        
        function setSpeed(speed) {{
            playbackSpeed = parseFloat(speed);
            sendCommand({{ type: 'setSpeed', speed: playbackSpeed }});
        }}
        
        function skip(seconds) {{
            sendCommand({{ type: 'skip', seconds: seconds }});
        }}
        
        function seekTo(timestamp) {{
            sendCommand({{ type: 'seek', timestamp: timestamp.toISOString() }});
        }}
        
        // ==========================================================================
        // Focus Mode - Upgrade one track to high quality
        // ==========================================================================
        
        function focusCamera(cameraId) {{
            const trackIdx = CAMERA_IDS.indexOf(cameraId);
            if (trackIdx >= 0) {{
                // Tell server to focus this camera (upgrade to high layer)
                sendCommand({{ type: 'focusCamera', cameraId: cameraId }});
                focusTrack(trackIdx, true);
            }}
        }}
        
        function focusTrack(trackIdx, sendToServer) {{
            if (focusedTrack === trackIdx) return;
            
            console.log(`Focusing track ${{trackIdx}}`);
            focusedTrack = trackIdx;
            
            // Update grid to focus mode
            const grid = document.getElementById('cameraGrid');
            grid.classList.add('focus-mode');
            
            // Mark focused slot
            document.querySelectorAll('.camera-slot').forEach(slot => {{
                slot.classList.remove('focused');
                if (parseInt(slot.dataset.slot) === trackIdx) {{
                    slot.classList.add('focused');
                }}
            }});
            
            // Show exit button
            document.getElementById('exitFocusBtn').style.display = 'flex';
        }}
        
        function exitFocus() {{
            sendCommand({{ type: 'exitFocus' }});
            exitFocusInternal();
        }}
        
        function exitFocusInternal() {{
            if (focusedTrack === null) return;
            
            focusedTrack = null;
            
            const grid = document.getElementById('cameraGrid');
            grid.classList.remove('focus-mode');
            
            document.querySelectorAll('.camera-slot').forEach(slot => {{
                slot.classList.remove('focused');
            }});
            
            document.getElementById('exitFocusBtn').style.display = 'none';
        }}
        
        // ==========================================================================
        // Timeline
        // ==========================================================================
        
        function onTimelineClick(event) {{
            const timeline = document.getElementById('timeline');
            const rect = timeline.getBoundingClientRect();
            const percent = (event.clientX - rect.left) / rect.width;
            
            const totalDuration = END_TIME.getTime() - START_TIME.getTime();
            const targetTime = new Date(START_TIME.getTime() + percent * totalDuration);
            
            seekTo(targetTime);
        }}
        
        function updatePlayheadPosition() {{
            const totalDuration = END_TIME.getTime() - START_TIME.getTime();
            const elapsed = currentTime.getTime() - START_TIME.getTime();
            const percent = Math.max(0, Math.min(100, (elapsed / totalDuration) * 100));
            
            document.getElementById('playhead').style.left = `${{percent}}%`;
        }}
        
        function updateTimeDisplay() {{
            const timeStr = currentTime.toLocaleTimeString('en-US', {{
                hour: '2-digit',
                minute: '2-digit',
                second: '2-digit',
                hour12: false
            }});
            const dateStr = currentTime.toLocaleDateString('en-US', {{
                month: 'short',
                day: 'numeric'
            }});
            document.getElementById('currentTime').textContent = `${{dateStr}} ${{timeStr}}`;
        }}
        
        function updateUI() {{
            updateTimeDisplay();
            updatePlayheadPosition();
            updatePlayPauseButton();
        }}
        
        function renderTimeline() {{
            const ruler = document.getElementById('timelineRuler');
            const totalDuration = END_TIME.getTime() - START_TIME.getTime();
            const hours = Math.ceil(totalDuration / 3600000);
            
            let rulerHtml = '';
            for (let i = 0; i <= Math.min(hours, 24); i++) {{
                const t = new Date(START_TIME.getTime() + i * 3600000);
                if (t <= END_TIME) {{
                    const percent = ((t.getTime() - START_TIME.getTime()) / totalDuration) * 100;
                    rulerHtml += `<span style="position:absolute;left:${{percent}}%">${{t.getHours()}}:00</span>`;
                }}
            }}
            ruler.innerHTML = rulerHtml;
            
            // Render tracks (one per camera)
            const tracks = document.getElementById('timelineTracks');
            let tracksHtml = '';
            CAMERA_IDS.forEach((id, idx) => {{
                tracksHtml += `
                    <div class="timeline-track" data-track="${{idx}}">
                        <div class="track-segment" style="left:0%;width:100%"></div>
                    </div>
                `;
            }});
            tracks.innerHTML = tracksHtml;
        }}
        
        // ==========================================================================
        // Keyboard Controls
        // ==========================================================================
        
        document.addEventListener('keydown', (e) => {{
            switch (e.key) {{
                case ' ':
                    e.preventDefault();
                    togglePlayPause();
                    break;
                case 'ArrowLeft':
                    skip(e.shiftKey ? -60 : -10);
                    break;
                case 'ArrowRight':
                    skip(e.shiftKey ? 60 : 10);
                    break;
                case 'Escape':
                    if (focusedTrack !== null) exitFocus();
                    break;
            }}
        }});
        
        // ==========================================================================
        // Initialization
        // ==========================================================================
        
        async function init() {{
            console.log('Multi-Camera Replay - Single Session Architecture');
            console.log(`Cameras (${{CAMERA_IDS.length}}): ${{CAMERA_IDS.join(', ')}}`);
            console.log(`Time range: ${{START_TIME}} - ${{END_TIME}}`);
            
            updateTimeDisplay();
            renderTimeline();
            updatePlayheadPosition();
            
            // Connect single session with all camera tracks
            await connect();
        }}
        
        init();
    </script>
</body>
</html>"##,
        common_styles = get_common_styles(),
        grid_class = grid_class,
        camera_slots = camera_slots,
        camera_ids_js = camera_ids_js,
        start_time = start_time,
        end_time = end_time
    )
}