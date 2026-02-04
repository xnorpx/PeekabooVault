<script lang="ts">
	import { page } from '$app/stores';
	import { onMount } from 'svelte';

	let cameraId = $derived($page.params.id);
	let cameraName = $state('Camera');
	let videoElement: HTMLVideoElement;
	let timeline = $state<{ start: Date; end: Date; segments: any[] }>({ 
		start: new Date(Date.now() - 24 * 60 * 60 * 1000), 
		end: new Date(), 
		segments: [] 
	});
	let currentTime = $state(new Date());
	let isPlaying = $state(false);
	let playbackSpeed = $state(1);
	let isLoading = $state(false);

	onMount(async () => {
		// Load camera info
		try {
			const resp = await fetch(`/api/cameras/${cameraId}`);
			const data = await resp.json();
			if (data.success && data.data) {
				cameraName = data.data.name;
			}
		} catch (e) {
			console.error('Failed to load camera:', e);
		}

		// Load timeline
		await loadTimeline();
	});

	async function loadTimeline() {
		try {
			const start = timeline.start.toISOString();
			const end = timeline.end.toISOString();
			const resp = await fetch(`/api/cameras/${cameraId}/timeline?start=${start}&end=${end}`);
			const data = await resp.json();
			if (data.success && data.data) {
				timeline.segments = data.data.segments || [];
			}
		} catch (e) {
			console.error('Failed to load timeline:', e);
		}
	}

	async function seekTo(time: Date) {
		isLoading = true;
		currentTime = time;
		
		try {
			const resp = await fetch(`/api/cameras/${cameraId}/playback/seek`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ timestamp: time.toISOString() })
			});
			const data = await resp.json();
			if (data.success) {
				// Start WebRTC playback from this point
				await startPlayback(time);
			}
		} catch (e) {
			console.error('Failed to seek:', e);
		} finally {
			isLoading = false;
		}
	}

	async function startPlayback(fromTime: Date) {
		// WebRTC replay connection would go here
		// Similar to viewer but with replay mode
		isPlaying = true;
	}

	function togglePlayPause() {
		isPlaying = !isPlaying;
	}

	function formatTime(date: Date): string {
		return date.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
	}

	function formatDate(date: Date): string {
		return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
	}

	function handleTimelineClick(event: MouseEvent) {
		const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
		const percent = (event.clientX - rect.left) / rect.width;
		const timeRange = timeline.end.getTime() - timeline.start.getTime();
		const newTime = new Date(timeline.start.getTime() + timeRange * percent);
		seekTo(newTime);
	}

	async function exportClip() {
		const start = new Date(currentTime.getTime() - 30000); // 30 sec before
		const end = new Date(currentTime.getTime() + 30000); // 30 sec after
		
		try {
			const resp = await fetch('/api/clips/export', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({
					cameraId,
					startTime: start.toISOString(),
					endTime: end.toISOString()
				})
			});
			
			if (resp.ok) {
				const blob = await resp.blob();
				const url = URL.createObjectURL(blob);
				const a = document.createElement('a');
				a.href = url;
				a.download = `clip_${cameraId}_${Date.now()}.mp4`;
				a.click();
				URL.revokeObjectURL(url);
			}
		} catch (e) {
			console.error('Failed to export clip:', e);
		}
	}

	// Calculate timeline position percentage
	let timelinePosition = $derived(() => {
		const range = timeline.end.getTime() - timeline.start.getTime();
		const pos = currentTime.getTime() - timeline.start.getTime();
		return Math.max(0, Math.min(100, (pos / range) * 100));
	});
</script>

<svelte:head>
	<title>{cameraName} - Replay | PeekabooVault</title>
</svelte:head>

<div class="replay-page">
	<header class="replay-header">
		<div class="header-left">
			<a href="/" class="back-btn">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<path d="M19 12H5M12 19l-7-7 7-7"/>
				</svg>
			</a>
			<div class="camera-info">
				<h1 class="camera-name">{cameraName}</h1>
				<div class="replay-time">
					<span>{formatDate(currentTime)}</span>
					<span>{formatTime(currentTime)}</span>
				</div>
			</div>
		</div>
		<div class="header-right">
			<a href="/viewer/{cameraId}" class="btn btn-secondary">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<circle cx="12" cy="12" r="10"/>
					<circle cx="12" cy="12" r="3"/>
				</svg>
				Live
			</a>
			<button class="btn btn-primary" onclick={exportClip}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
					<polyline points="7 10 12 15 17 10"/>
					<line x1="12" y1="15" x2="12" y2="3"/>
				</svg>
				Export Clip
			</button>
		</div>
	</header>

	<div class="video-container">
		<video bind:this={videoElement} autoplay playsinline></video>
		
		{#if isLoading}
			<div class="video-overlay">
				<div class="spinner"></div>
				<p>Loading...</p>
			</div>
		{:else if !isPlaying && timeline.segments.length === 0}
			<div class="video-overlay">
				<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
					<rect x="2" y="2" width="20" height="20" rx="2.18" ry="2.18"/>
					<line x1="7" y1="2" x2="7" y2="22"/>
					<line x1="17" y1="2" x2="17" y2="22"/>
					<line x1="2" y1="12" x2="22" y2="12"/>
					<line x1="2" y1="7" x2="7" y2="7"/>
					<line x1="2" y1="17" x2="7" y2="17"/>
					<line x1="17" y1="17" x2="22" y2="17"/>
					<line x1="17" y1="7" x2="22" y2="7"/>
				</svg>
				<p>No recordings in this time range</p>
				<p class="text-muted">Try adjusting the timeline or start recording</p>
			</div>
		{/if}
	</div>

	<div class="controls-panel">
		<div class="playback-controls">
			<button class="btn btn-icon" onclick={() => seekTo(new Date(currentTime.getTime() - 10000))} title="Back 10s">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<polygon points="11 19 2 12 11 5 11 19"/>
					<polygon points="22 19 13 12 22 5 22 19"/>
				</svg>
			</button>
			<button class="btn btn-play" onclick={togglePlayPause}>
				{#if isPlaying}
					<svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
						<rect x="6" y="4" width="4" height="16"/>
						<rect x="14" y="4" width="4" height="16"/>
					</svg>
				{:else}
					<svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
						<polygon points="5 3 19 12 5 21 5 3"/>
					</svg>
				{/if}
			</button>
			<button class="btn btn-icon" onclick={() => seekTo(new Date(currentTime.getTime() + 10000))} title="Forward 10s">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<polygon points="13 19 22 12 13 5 13 19"/>
					<polygon points="2 19 11 12 2 5 2 19"/>
				</svg>
			</button>
			<select class="speed-select" bind:value={playbackSpeed}>
				<option value={0.5}>0.5x</option>
				<option value={1}>1x</option>
				<option value={2}>2x</option>
				<option value={4}>4x</option>
				<option value={8}>8x</option>
			</select>
		</div>

		<div class="timeline-container">
			<div class="timeline-labels">
				<span>{formatTime(timeline.start)}</span>
				<span>{formatTime(timeline.end)}</span>
			</div>
			<div class="timeline-track" onclick={handleTimelineClick}>
				{#each timeline.segments as segment}
					<div 
						class="timeline-segment recording"
						style="left: {((new Date(segment.start).getTime() - timeline.start.getTime()) / (timeline.end.getTime() - timeline.start.getTime())) * 100}%; width: {((new Date(segment.end).getTime() - new Date(segment.start).getTime()) / (timeline.end.getTime() - timeline.start.getTime())) * 100}%"
					></div>
				{/each}
				<div class="timeline-cursor" style="left: {timelinePosition()}%"></div>
			</div>
		</div>
	</div>
</div>

<style>
	.replay-page {
		display: flex;
		flex-direction: column;
		height: 100vh;
		background: #000;
	}

	.replay-header {
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

	.camera-name {
		font-size: 1rem;
		font-weight: 600;
		margin-bottom: 0.125rem;
	}

	.replay-time {
		display: flex;
		gap: 0.5rem;
		font-size: 0.75rem;
		color: var(--color-text-muted);
	}

	.header-right {
		display: flex;
		gap: 0.75rem;
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

	.text-muted {
		font-size: 0.875rem;
		opacity: 0.7;
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

	.controls-panel {
		background: var(--color-bg-secondary);
		border-top: 1px solid var(--color-border);
		padding: 1rem;
	}

	.playback-controls {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 1rem;
		margin-bottom: 1rem;
	}

	.btn-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 40px;
		height: 40px;
		background: var(--color-bg-tertiary);
		border: none;
		border-radius: 8px;
		color: var(--color-text);
		cursor: pointer;
		transition: all 0.2s;
	}

	.btn-icon:hover {
		background: var(--color-bg-hover);
	}

	.btn-play {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 56px;
		height: 56px;
		background: var(--color-primary);
		border: none;
		border-radius: 50%;
		color: white;
		cursor: pointer;
		transition: all 0.2s;
	}

	.btn-play:hover {
		background: var(--color-primary-hover);
		transform: scale(1.05);
	}

	.speed-select {
		padding: 0.5rem;
		background: var(--color-bg-tertiary);
		border: 1px solid var(--color-border);
		border-radius: 6px;
		color: var(--color-text);
		font-size: 0.875rem;
		cursor: pointer;
	}

	.timeline-container {
		width: 100%;
	}

	.timeline-labels {
		display: flex;
		justify-content: space-between;
		font-size: 0.75rem;
		color: var(--color-text-muted);
		margin-bottom: 0.5rem;
	}

	.timeline-track {
		position: relative;
		height: 40px;
		background: var(--color-bg-tertiary);
		border-radius: 6px;
		cursor: pointer;
		overflow: hidden;
	}

	.timeline-segment {
		position: absolute;
		top: 0;
		height: 100%;
		background: var(--color-primary);
		opacity: 0.6;
	}

	.timeline-segment.recording {
		background: #22c55e;
	}

	.timeline-cursor {
		position: absolute;
		top: 0;
		width: 2px;
		height: 100%;
		background: #ef4444;
		box-shadow: 0 0 8px rgba(239, 68, 68, 0.5);
		z-index: 10;
	}

	.timeline-cursor::after {
		content: '';
		position: absolute;
		top: -4px;
		left: -4px;
		width: 10px;
		height: 10px;
		background: #ef4444;
		border-radius: 50%;
	}
</style>
