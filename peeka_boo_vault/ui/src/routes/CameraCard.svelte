<script lang="ts">
	import type { Camera, CameraStatus, StreamProfile } from '$lib';

	interface Props {
		camera: Camera;
		onProbe: () => void;
		onCredentials: () => void;
		onDelete: () => void;
	}

	let { camera, onProbe, onCredentials, onDelete }: Props = $props();
	let showProfiles = $state(false);

	function getStatusBadgeClass(status: CameraStatus): string {
		switch (status) {
			case 'online': return 'badge-success';
			case 'offline': return 'badge-error';
			case 'unauthorized': return 'badge-warning';
			case 'probing': return 'badge-info';
			default: return 'badge-muted';
		}
	}

	function getStatusLabel(status: CameraStatus): string {
		switch (status) {
			case 'online': return 'Online';
			case 'offline': return 'Offline';
			case 'unauthorized': return 'Auth Required';
			case 'probing': return 'Probing...';
			default: return 'Unknown';
		}
	}

	function formatResolution(profile: StreamProfile): string {
		if (profile.width && profile.height) {
			return `${profile.width}x${profile.height}`;
		}
		return 'N/A';
	}

	function formatBitrate(kbps: number | null): string {
		if (!kbps) return 'N/A';
		if (kbps >= 1000) {
			return `${(kbps / 1000).toFixed(1)} Mbps`;
		}
		return `${kbps} kbps`;
	}

	function formatFrameRate(fps: number | null): string {
		if (!fps) return 'N/A';
		return `${fps} fps`;
	}
</script>

<div class="card camera-card">
	<div class="camera-header">
		<div class="camera-icon">
			<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/>
				<circle cx="12" cy="13" r="4"/>
			</svg>
		</div>
		<div class="camera-info">
			<h3>{camera.name}</h3>
			<span class="badge {getStatusBadgeClass(camera.status)}">
				{getStatusLabel(camera.status)}
			</span>
		</div>
	</div>

	<div class="camera-details">
		{#if camera.manufacturer || camera.model}
			<div class="detail-row">
				<span class="detail-label">Device</span>
				<span class="detail-value">{[camera.manufacturer, camera.model].filter(Boolean).join(' ')}</span>
			</div>
		{/if}

		{#if camera.firmwareVersion}
			<div class="detail-row">
				<span class="detail-label">Firmware</span>
				<span class="detail-value">{camera.firmwareVersion}</span>
			</div>
		{/if}

		{#if camera.serialNumber}
			<div class="detail-row">
				<span class="detail-label">Serial</span>
				<span class="detail-value" title={camera.serialNumber}>
					{camera.serialNumber.length > 20 ? camera.serialNumber.substring(0, 20) + '...' : camera.serialNumber}
				</span>
			</div>
		{/if}

		{#if camera.onvifAddress}
			<div class="detail-row">
				<span class="detail-label">IP Address</span>
				<span class="detail-value">{camera.onvifAddress}</span>
			</div>
		{/if}

		{#if camera.lastSeen}
			<div class="detail-row">
				<span class="detail-label">Last Seen</span>
				<span class="detail-value">{new Date(camera.lastSeen).toLocaleString()}</span>
			</div>
		{/if}

		<div class="detail-row">
			<span class="detail-label">Credentials</span>
			<span class="detail-value">
				{#if camera.hasCredentials}
					<span class="badge badge-success">Configured</span>
				{:else}
					<span class="badge badge-warning">Not Set</span>
				{/if}
			</span>
		</div>
	</div>

	<!-- Stream Profiles Section -->
	{#if camera.streamProfiles && camera.streamProfiles.length > 0}
		<div class="stream-profiles-section">
			<button class="profiles-toggle" onclick={() => showProfiles = !showProfiles}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class:rotated={showProfiles}>
					<polyline points="9 18 15 12 9 6"/>
				</svg>
				<span>Stream Profiles ({camera.streamProfiles.length})</span>
			</button>
			
			{#if showProfiles}
				<div class="profiles-list">
					{#each camera.streamProfiles as profile, index}
						<div class="profile-card">
							<div class="profile-header">
								<span class="profile-name">{profile.name || `Profile ${index + 1}`}</span>
								<span class="profile-resolution">{formatResolution(profile)}</span>
							</div>
							<div class="profile-details">
								<div class="profile-row">
									<span class="profile-label">Codec</span>
									<span class="profile-value">{profile.encoding || 'N/A'}{profile.codecProfile ? ` (${profile.codecProfile})` : ''}</span>
								</div>
								<div class="profile-row">
									<span class="profile-label">Frame Rate</span>
									<span class="profile-value">{formatFrameRate(profile.frameRate)}{profile.guaranteedFrameRate ? ' (guaranteed)' : ''}</span>
								</div>
								<div class="profile-row">
									<span class="profile-label">Bitrate</span>
									<span class="profile-value">{formatBitrate(profile.bitrateKbps)}</span>
								</div>
								{#if profile.gopLength}
									<div class="profile-row">
										<span class="profile-label">GOP Length</span>
										<span class="profile-value">{profile.gopLength} frames</span>
									</div>
								{/if}
								{#if profile.quality}
									<div class="profile-row">
										<span class="profile-label">Quality</span>
										<span class="profile-value">{profile.quality.toFixed(1)}</span>
									</div>
								{/if}
								{#if profile.encodingInterval && profile.encodingInterval > 1}
									<div class="profile-row">
										<span class="profile-label">Encoding Interval</span>
										<span class="profile-value">1/{profile.encodingInterval}</span>
									</div>
								{/if}
								<div class="profile-row uri-row">
									<span class="profile-label">RTSP URI</span>
									<span class="profile-value stream-uri" title={profile.streamUri}>
										{profile.streamUri.length > 50 ? profile.streamUri.substring(0, 50) + '...' : profile.streamUri}
									</span>
								</div>
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	{/if}

	<div class="camera-actions">
		<button class="btn btn-secondary btn-sm" onclick={onProbe} disabled={camera.status === 'probing'}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2"/>
			</svg>
			Probe
		</button>
		<button class="btn btn-secondary btn-sm" onclick={onCredentials}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<rect x="3" y="11" width="18" height="11" rx="2" ry="2"/>
				<path d="M7 11V7a5 5 0 0 1 10 0v4"/>
			</svg>
			{camera.hasCredentials ? 'Update' : 'Set'} Credentials
		</button>
		<button class="btn btn-secondary btn-sm btn-danger" onclick={onDelete}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<polyline points="3 6 5 6 21 6"/>
				<path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
			</svg>
			Delete
		</button>
	</div>
</div>

<style>
	.camera-card {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.camera-header {
		display: flex;
		gap: 1rem;
		align-items: flex-start;
	}

	.camera-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 48px;
		height: 48px;
		background: var(--color-bg-tertiary);
		border-radius: var(--radius);
		color: var(--color-primary);
		flex-shrink: 0;
	}

	.camera-info {
		flex: 1;
		min-width: 0;
	}

	.camera-info h3 {
		font-size: 1rem;
		font-weight: 600;
		margin-bottom: 0.5rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.camera-details {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		padding: 0.75rem;
		background: var(--color-bg-tertiary);
		border-radius: var(--radius);
	}

	.detail-row {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 1rem;
		font-size: 0.8125rem;
	}

	.detail-label {
		color: var(--color-text-muted);
		flex-shrink: 0;
	}

	.detail-value {
		text-align: right;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.stream-uri {
		font-family: monospace;
		font-size: 0.75rem;
		color: var(--color-text-muted);
	}

	.camera-actions {
		display: flex;
		gap: 0.5rem;
		flex-wrap: wrap;
	}

	.btn-sm {
		padding: 0.375rem 0.625rem;
		font-size: 0.8125rem;
	}

	.btn-danger:hover {
		background: rgba(239, 68, 68, 0.15);
		border-color: rgba(239, 68, 68, 0.5);
		color: var(--color-error);
	}

	/* Stream Profiles Section */
	.stream-profiles-section {
		border-top: 1px solid var(--color-border);
		padding-top: 0.75rem;
	}

	.profiles-toggle {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		background: none;
		border: none;
		color: var(--color-text);
		font-size: 0.875rem;
		font-weight: 500;
		cursor: pointer;
		padding: 0.5rem;
		margin: -0.5rem;
		border-radius: var(--radius);
		transition: background-color 0.15s;
	}

	.profiles-toggle:hover {
		background: var(--color-bg-tertiary);
	}

	.profiles-toggle svg {
		transition: transform 0.2s;
	}

	.profiles-toggle svg.rotated {
		transform: rotate(90deg);
	}

	.profiles-list {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		margin-top: 0.75rem;
	}

	.profile-card {
		background: var(--color-bg-tertiary);
		border-radius: var(--radius);
		padding: 0.75rem;
		border: 1px solid var(--color-border);
	}

	.profile-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 0.5rem;
		padding-bottom: 0.5rem;
		border-bottom: 1px solid var(--color-border);
	}

	.profile-name {
		font-weight: 600;
		font-size: 0.875rem;
		color: var(--color-primary);
	}

	.profile-resolution {
		font-family: monospace;
		font-size: 0.8125rem;
		background: var(--color-bg-secondary);
		padding: 0.125rem 0.5rem;
		border-radius: var(--radius);
		color: var(--color-text-muted);
	}

	.profile-details {
		display: grid;
		grid-template-columns: repeat(2, 1fr);
		gap: 0.375rem 1rem;
	}

	.profile-row {
		display: flex;
		justify-content: space-between;
		font-size: 0.75rem;
	}

	.profile-row.uri-row {
		grid-column: 1 / -1;
		flex-direction: column;
		gap: 0.25rem;
	}

	.profile-label {
		color: var(--color-text-muted);
	}

	.profile-value {
		color: var(--color-text);
		text-align: right;
	}

	.profile-row.uri-row .profile-value {
		text-align: left;
	}
</style>
