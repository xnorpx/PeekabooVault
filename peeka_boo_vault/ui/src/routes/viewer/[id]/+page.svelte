<script lang="ts">
	import { page } from '$app/stores';
	import { onMount, onDestroy } from 'svelte';

	let cameraId = $derived($page.params.id);
	let cameraName = $state('Camera');
	let status = $state<'connecting' | 'live' | 'offline'>('connecting');
	let statusText = $state('Connecting...');
	let videoElement: HTMLVideoElement;
	let peerConnection: RTCPeerConnection | null = null;
	let streamType = $state<'main' | 'sub'>('main');
	let isRecording = $state(false);
	let showStats = $state(false);
	let stats = $state({
		codec: '',
		resolution: '',
		fps: 0,
		bitrate: 0,
		bufferHealth: 0,
		framesDropped: 0
	});
	let statsInterval: number | null = null;

	onMount(async () => {
		// Load camera info
		try {
			const resp = await fetch(`/api/cameras/${cameraId}`);
			const data = await resp.json();
			if (data.success && data.data) {
				cameraName = data.data.name;
				isRecording = data.data.status === 'online';
			}
		} catch (e) {
			console.error('Failed to load camera:', e);
		}

		// Start WebRTC connection
		await startWebRTC();

		// Start stats polling
		startStatsPolling();
	});

	onDestroy(() => {
		cleanup();
		stopStatsPolling();
	});

	function startStatsPolling() {
		stopStatsPolling();
		updateStats(); // Initial fetch
		statsInterval = window.setInterval(updateStats, 1000); // Update every second
	}

	function stopStatsPolling() {
		if (statsInterval !== null) {
			clearInterval(statsInterval);
			statsInterval = null;
		}
	}

	async function updateStats() {
		try {
			const resp = await fetch(`/api/cameras/${cameraId}/stream/stats`);
			const data = await resp.json();
			if (data.success && data.data) {
				const streamStats = data.data;
				stats = {
					codec: streamStats.codec || 'Unknown',
					resolution: streamStats.resolution || '',
					fps: streamStats.fps || 0,
					bitrate: streamStats.bitrate_kbps || 0,
					bufferHealth: streamStats.buffer_health || 0,
					framesDropped: streamStats.frames_dropped || 0
				};
			}
		} catch (e) {
			// Silently fail - stats are non-critical
		}
	}

	function cleanup() {
		if (peerConnection) {
			peerConnection.close();
			peerConnection = null;
		}
	}

	async function startWebRTC() {
		cleanup();
		status = 'connecting';
		statusText = 'Connecting...';

		try {
			peerConnection = new RTCPeerConnection({
				iceServers: [{ urls: 'stun:stun.l.google.com:19302' }]
			});

			peerConnection.ontrack = (event) => {
				if (event.streams[0] && videoElement) {
					videoElement.srcObject = event.streams[0];
					status = 'live';
					statusText = 'Live';
				}
			};

			peerConnection.onicecandidate = async (event) => {
				if (event.candidate) {
					await fetch('/api/webrtc/ice-candidate', {
						method: 'POST',
						headers: { 'Content-Type': 'application/json' },
						body: JSON.stringify({
							cameraId,
							candidate: event.candidate.toJSON()
						})
					});
				}
			};

			peerConnection.onconnectionstatechange = () => {
				if (peerConnection?.connectionState === 'disconnected' || 
				    peerConnection?.connectionState === 'failed') {
					status = 'offline';
					statusText = 'Disconnected';
				}
			};

			// Add transceivers
			peerConnection.addTransceiver('video', { direction: 'recvonly' });
			peerConnection.addTransceiver('audio', { direction: 'recvonly' });

			// Create offer
			const offer = await peerConnection.createOffer();
			await peerConnection.setLocalDescription(offer);

			// Send offer to server
			const response = await fetch('/api/webrtc/offer', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({
					cameraId,
					streamType,
					offer: offer.sdp
				})
			});

			const data = await response.json();
			if (data.success && data.data?.answer) {
				await peerConnection.setRemoteDescription({
					type: 'answer',
					sdp: data.data.answer
				});
			} else {
				throw new Error(data.error || 'Failed to get answer');
			}
		} catch (e) {
			console.error('WebRTC error:', e);
			status = 'offline';
			statusText = 'Connection failed';
		}
	}

	async function toggleStream() {
		streamType = streamType === 'main' ? 'sub' : 'main';
		await startWebRTC();
	}

	async function toggleRecording() {
		const endpoint = isRecording ? 'stop' : 'start';
		try {
			await fetch(`/api/cameras/${cameraId}/recording/${endpoint}`, { method: 'POST' });
			isRecording = !isRecording;
		} catch (e) {
			console.error('Failed to toggle recording:', e);
		}
	}

	function goFullscreen() {
		videoElement?.requestFullscreen();
	}
</script>

<svelte:head>
	<title>{cameraName} - Live | PeekabooVault</title>
</svelte:head>

<div class="viewer">
	<header class="viewer-header">
		<div class="header-left">
			<a href="/" class="back-btn">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<path d="M19 12H5M12 19l-7-7 7-7"/>
				</svg>
			</a>
			<div class="camera-info">
				<h1 class="camera-name">{cameraName}</h1>
				<div class="camera-status">
					<span class="status-dot {status}"></span>
					<span>{statusText}</span>
				</div>
			</div>
		</div>
		<div class="header-right">
			<div class="stream-toggle">
				<button class:active={streamType === 'main'} onclick={() => { streamType = 'main'; startWebRTC(); }}>Main</button>
				<button class:active={streamType === 'sub'} onclick={() => { streamType = 'sub'; startWebRTC(); }}>Sub</button>
			</div>
			<button class="btn btn-icon" onclick={toggleRecording} title={isRecording ? 'Stop Recording' : 'Start Recording'}>
				<span class="rec-dot" class:recording={isRecording}></span>
				{isRecording ? 'REC' : 'OFF'}
			</button>
			<a href="/replay/{cameraId}" class="btn btn-secondary">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<polygon points="5 3 19 12 5 21 5 3"/>
				</svg>
				Replay
			</a>
		</div>
	</header>

	<div class="video-container">
		<video bind:this={videoElement} autoplay playsinline muted></video>
		
		{#if status === 'connecting'}
			<div class="video-overlay">
				<div class="spinner"></div>
				<p>Connecting to camera...</p>
			</div>
		{:else if status === 'offline'}
			<div class="video-overlay">
				<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
					<path d="M16.72 11.06A10.94 10.94 0 0119 12.55"/>
					<path d="M5 12.55a10.94 10.94 0 015.17-2.39"/>
					<path d="M10.71 5.05A16 16 0 0122.58 9"/>
					<path d="M1.42 9a15.91 15.91 0 014.7-2.88"/>
					<path d="M8.53 16.11a6 6 0 016.95 0"/>
					<line x1="12" y1="20" x2="12.01" y2="20"/>
					<line x1="2" y1="2" x2="22" y2="22"/>
				</svg>
				<p>{statusText}</p>
				<button class="btn btn-primary" onclick={startWebRTC}>Retry</button>
			</div>
		{/if}

		<div class="video-controls">
			<button
				class="btn btn-icon"
				onclick={() => (showStats = !showStats)}
				title="Toggle Stats"
				class:active={showStats}
			>
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<line x1="12" y1="20" x2="12" y2="10"></line>
					<line x1="18" y1="20" x2="18" y2="4"></line>
					<line x1="6" y1="20" x2="6" y2="16"></line>
				</svg>
			</button>
			<button class="btn btn-icon" onclick={goFullscreen} title="Fullscreen">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<path d="M8 3H5a2 2 0 00-2 2v3m18 0V5a2 2 0 00-2-2h-3m0 18h3a2 2 0 002-2v-3M3 16v3a2 2 0 002 2h3"/>
				</svg>
			</button>
		</div>

		{#if showStats && status === 'live'}
			<div class="stats-overlay">
				<div class="stats-header">
					<span>Stream Statistics</span>
					<button class="stats-close" onclick={() => (showStats = false)}>×</button>
				</div>
				<div class="stats-grid">
					<div class="stat-item">
						<span class="stat-label">Codec</span>
						<span class="stat-value">{stats.codec || 'N/A'}</span>
					</div>
					<div class="stat-item">
						<span class="stat-label">Resolution</span>
						<span class="stat-value">{stats.resolution || 'N/A'}</span>
					</div>
					<div class="stat-item">
						<span class="stat-label">FPS</span>
						<span class="stat-value">{stats.fps.toFixed(1)}</span>
					</div>
					<div class="stat-item">
						<span class="stat-label">Bitrate</span>
						<span class="stat-value">{stats.bitrate.toFixed(0)} kbps</span>
					</div>
					<div class="stat-item">
						<span class="stat-label">Buffer</span>
						<span class="stat-value">{stats.bufferHealth}%</span>
					</div>
					<div class="stat-item">
						<span class="stat-label">Dropped</span>
						<span class="stat-value">{stats.framesDropped}</span>
					</div>
				</div>
			</div>
		{/if}
	</div>
</div>

<style>
	.viewer {
		display: flex;
		flex-direction: column;
		height: 100vh;
		background: #000;
	}

	.viewer-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 0.75rem 1rem;
		background: var(--color-bg-secondary);
		border-bottom: 1px solid var(--color-border);
	}

	.header-left {
		display: flex;
		align-items: center;
		gap: 1rem;
	}

	.back-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 36px;
		height: 36px;
		border-radius: 8px;
		color: var(--color-text-muted);
		transition: all 0.2s;
	}

	.back-btn:hover {
		background: var(--color-bg-tertiary);
		color: var(--color-text);
	}

	.camera-info {
		display: flex;
		flex-direction: column;
	}

	.camera-name {
		font-size: 1rem;
		font-weight: 600;
	}

	.camera-status {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-size: 0.75rem;
		color: var(--color-text-muted);
	}

	.status-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--color-text-muted);
	}

	.status-dot.live {
		background: #ef4444;
		box-shadow: 0 0 0 2px rgba(239, 68, 68, 0.3);
		animation: pulse 2s infinite;
	}

	.status-dot.connecting {
		background: #f59e0b;
		animation: pulse 1s infinite;
	}

	@keyframes pulse {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.5; }
	}

	.header-right {
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}

	.stream-toggle {
		display: flex;
		background: var(--color-bg-tertiary);
		border-radius: 6px;
		padding: 2px;
	}

	.stream-toggle button {
		padding: 0.35rem 0.75rem;
		font-size: 0.75rem;
		font-weight: 500;
		border: none;
		background: transparent;
		color: var(--color-text-muted);
		border-radius: 4px;
		cursor: pointer;
		transition: all 0.2s;
	}

	.stream-toggle button.active {
		background: var(--color-primary);
		color: white;
	}

	.btn-icon {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		padding: 0.4rem 0.75rem;
		font-size: 0.75rem;
		font-weight: 600;
		background: var(--color-bg-tertiary);
		border: none;
		border-radius: 6px;
		color: var(--color-text-muted);
		cursor: pointer;
		transition: all 0.2s;
	}

	.btn-icon:hover {
		background: var(--color-bg-hover);
		color: var(--color-text);
	}

	.rec-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--color-text-muted);
	}

	.rec-dot.recording {
		background: #ef4444;
		animation: pulse 1s infinite;
	}

	.video-container {
		flex: 1;
		position: relative;
		display: flex;
		align-items: center;
		justify-content: center;
		overflow: hidden;
	}

	video {
		max-width: 100%;
		max-height: 100%;
		object-fit: contain;
	}

	.video-overlay {
		position: absolute;
		inset: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		background: rgba(0, 0, 0, 0.8);
		color: var(--color-text-muted);
		gap: 1rem;
	}

	.spinner {
		width: 48px;
		height: 48px;
		border: 3px solid var(--color-bg-tertiary);
		border-top-color: var(--color-primary);
		border-radius: 50%;
		animation: spin 1s linear infinite;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}

	.video-controls {
		position: absolute;
		bottom: 1rem;
		right: 1rem;
		display: flex;
		gap: 0.5rem;
		opacity: 0;
		transition: opacity 0.3s;
	}

	.video-container:hover .video-controls {
		opacity: 1;
	}

	.btn-icon.active {
		background: var(--color-primary);
		color: white;
	}

	.stats-overlay {
		position: absolute;
		top: 1rem;
		right: 1rem;
		background: rgba(0, 0, 0, 0.85);
		backdrop-filter: blur(10px);
		border-radius: 8px;
		padding: 0;
		min-width: 240px;
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
		color: white;
		font-size: 0.8125rem;
	}

	.stats-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 0.75rem 1rem;
		border-bottom: 1px solid rgba(255, 255, 255, 0.1);
		font-weight: 600;
	}

	.stats-close {
		background: none;
		border: none;
		color: rgba(255, 255, 255, 0.6);
		font-size: 1.5rem;
		line-height: 1;
		cursor: pointer;
		padding: 0;
		width: 24px;
		height: 24px;
		display: flex;
		align-items: center;
		justify-content: center;
		border-radius: 4px;
		transition: all 0.2s;
	}

	.stats-close:hover {
		background: rgba(255, 255, 255, 0.1);
		color: white;
	}

	.stats-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 0.75rem;
		padding: 1rem;
	}

	.stat-item {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
	}

	.stat-label {
		font-size: 0.6875rem;
		color: rgba(255, 255, 255, 0.5);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		font-weight: 500;
	}

	.stat-value {
		font-size: 0.9375rem;
		font-weight: 600;
		font-family: 'SF Mono', 'Consolas', monospace;
	}
</style>
