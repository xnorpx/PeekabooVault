<script lang="ts">
	import type { DiscoveredDevice } from '$lib';

	interface Props {
		device: DiscoveredDevice;
		onAdd: () => void;
	}

	let { device, onAdd }: Props = $props();

	function getPrimaryUrl(): string | null {
		return device.urls.length > 0 ? device.urls[0] : null;
	}

	function getDeviceName(): string {
		return device.name || device.hardware || 'Unknown Device';
	}

	function formatTypes(): string {
		return device.types
			.map(t => t.split(':').pop() || t)
			.filter(t => t !== 'Device')
			.join(', ');
	}
</script>

<div class="card device-card">
	<div class="device-header">
		<div class="device-icon">
			<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
				<line x1="8" y1="21" x2="16" y2="21"/>
				<line x1="12" y1="17" x2="12" y2="21"/>
			</svg>
		</div>
		<div class="device-info">
			<h3>{getDeviceName()}</h3>
			<span class="badge badge-info">Discovered</span>
		</div>
	</div>

	<div class="device-details">
		{#if device.types.length > 0}
			<div class="detail-row">
				<span class="detail-label">Type</span>
				<span class="detail-value">{formatTypes()}</span>
			</div>
		{/if}

		{#if getPrimaryUrl()}
			<div class="detail-row">
				<span class="detail-label">URL</span>
				<span class="detail-value url" title={getPrimaryUrl()}>
					{getPrimaryUrl()}
				</span>
			</div>
		{/if}

		<div class="detail-row">
			<span class="detail-label">Discovered</span>
			<span class="detail-value">{new Date(device.discoveredAt).toLocaleTimeString()}</span>
		</div>
	</div>

	<button class="btn btn-primary" onclick={onAdd}>
		<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
			<line x1="12" y1="5" x2="12" y2="19"/>
			<line x1="5" y1="12" x2="19" y2="12"/>
		</svg>
		Add Camera
	</button>
</div>

<style>
	.device-card {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.device-header {
		display: flex;
		gap: 1rem;
		align-items: flex-start;
	}

	.device-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 48px;
		height: 48px;
		background: rgba(59, 130, 246, 0.15);
		border-radius: var(--radius);
		color: var(--color-info);
		flex-shrink: 0;
	}

	.device-info {
		flex: 1;
		min-width: 0;
	}

	.device-info h3 {
		font-size: 1rem;
		font-weight: 600;
		margin-bottom: 0.5rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.device-details {
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

	.url {
		font-family: monospace;
		font-size: 0.75rem;
		color: var(--color-text-muted);
	}
</style>
