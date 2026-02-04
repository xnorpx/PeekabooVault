<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type Camera, type DiscoveredDevice, type ServerStatus, type CameraStatus } from '$lib';
	import CameraCard from './CameraCard.svelte';
	import DiscoveredDeviceCard from './DiscoveredDeviceCard.svelte';
	import AddCameraModal from './AddCameraModal.svelte';
	import CredentialsModal from './CredentialsModal.svelte';

	let status: ServerStatus | null = $state(null);
	let cameras: Camera[] = $state([]);
	let discoveredDevices: DiscoveredDevice[] = $state([]);
	let scanning = $state(false);
	let scanError = $state<string | null>(null);
	let showAddModal = $state(false);
	let selectedDeviceForAdd = $state<DiscoveredDevice | null>(null);
	let credentialsModalCamera = $state<Camera | null>(null);
	
	onMount(() => {
		loadData();
		// Poll status every 5 seconds
		const interval = setInterval(loadStatus, 5000);
		return () => clearInterval(interval);
	});

	async function loadData() {
		await Promise.all([loadStatus(), loadCameras(), loadDiscoveredDevices()]);
	}

	async function loadStatus() {
		try {
			status = await api.getStatus();
			scanning = status.discoveryInProgress;
		} catch (e) {
			console.error('Failed to load status:', e);
		}
	}

	async function loadCameras() {
		try {
			const response = await api.listCameras();
			if (response.success && response.data) {
				cameras = response.data;
			}
		} catch (e) {
			console.error('Failed to load cameras:', e);
		}
	}

	async function loadDiscoveredDevices() {
		try {
			const response = await api.getDiscoveredDevices();
			if (response.success && response.data) {
				discoveredDevices = response.data;
			}
		} catch (e) {
			console.error('Failed to load discovered devices:', e);
		}
	}

	async function startScan() {
		scanning = true;
		scanError = null;
		try {
			const response = await api.scanDevices(5);
			if (response.success) {
				discoveredDevices = response.devices;
			} else {
				scanError = response.error;
			}
		} catch (e) {
			scanError = e instanceof Error ? e.message : 'Scan failed';
		} finally {
			scanning = false;
			await loadCameras(); // Refresh to update onboarded status
		}
	}

	function handleAddFromDevice(device: DiscoveredDevice) {
		selectedDeviceForAdd = device;
		showAddModal = true;
	}

	async function handleAddCamera(name: string, host: string, username?: string, password?: string) {
		try {
			const response = await api.addCamera(name, host, username, password);
			if (response.success && response.data) {
				cameras = [...cameras, response.data];
				showAddModal = false;
				selectedDeviceForAdd = null;
				// Update discovered devices
				await loadDiscoveredDevices();
			}
		} catch (e) {
			console.error('Failed to add camera:', e);
			throw e; // Re-throw so the modal can display the error
		}
	}

	function handleSetCredentials(camera: Camera) {
		credentialsModalCamera = camera;
	}

	async function handleSaveCredentials(username: string, password: string) {
		if (!credentialsModalCamera) return;
		
		try {
			const response = await api.setCredentials(credentialsModalCamera.id, { username, password });
			if (response.camera) {
				cameras = cameras.map(c => c.id === response.camera!.id ? response.camera! : c);
			}
			credentialsModalCamera = null;
		} catch (e) {
			console.error('Failed to set credentials:', e);
		}
	}

	async function handleProbe(camera: Camera) {
		try {
			const response = await api.probeCamera(camera.id);
			if (response.camera) {
				cameras = cameras.map(c => c.id === response.camera!.id ? response.camera! : c);
			}
		} catch (e) {
			console.error('Failed to probe camera:', e);
		}
	}

	async function handleDelete(camera: Camera) {
		// confirm() may not work in embedded browsers
		const shouldDelete = window.confirm(`Delete camera "${camera.name}"?`);
		if (!shouldDelete) return;
		
		try {
			await api.deleteCamera(camera.id);
			cameras = cameras.filter(c => c.id !== camera.id);
			await loadDiscoveredDevices();
		} catch (e) {
			console.error('Failed to delete camera:', e);
		}
	}

	// Filter out devices that are already onboarded
	let unOnboardedDevices = $derived(discoveredDevices.filter(d => !d.isOnboarded));

	function getStatusColor(status: CameraStatus): string {
		switch (status) {
			case 'online': return 'var(--color-success)';
			case 'offline': return 'var(--color-error)';
			case 'unauthorized': return 'var(--color-warning)';
			case 'probing': return 'var(--color-info)';
			default: return 'var(--color-text-muted)';
		}
	}
</script>

<svelte:head>
	<title>Cameras | PeekabooVault</title>
</svelte:head>

<div class="page">
	<!-- Status Bar -->
	{#if status}
		<div class="status-bar">
			<div class="status-item">
				<span class="status-label">Version</span>
				<span class="status-value">{status.version}</span>
			</div>
			<div class="status-item">
				<span class="status-label">Cameras</span>
				<span class="status-value">{status.camerasOnline}/{status.cameraCount} online</span>
			</div>
			<div class="status-item">
				<span class="status-label">Uptime</span>
				<span class="status-value">{Math.floor(status.uptimeSecs / 60)}m</span>
			</div>
		</div>
	{/if}

	<!-- Quick Actions -->
	<div class="quick-actions">
		<button class="btn btn-primary btn-large" onclick={() => { selectedDeviceForAdd = null; showAddModal = true; }}>
			<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<line x1="12" y1="5" x2="12" y2="19"/>
				<line x1="5" y1="12" x2="19" y2="12"/>
			</svg>
			Add Camera
		</button>
		<button class="btn btn-secondary btn-large" onclick={startScan} disabled={scanning}>
			{#if scanning}
				<span class="spinner"></span>
				Scanning...
			{:else}
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<circle cx="11" cy="11" r="8"/>
					<path d="m21 21-4.35-4.35"/>
				</svg>
				Scan Network
			{/if}
		</button>
	</div>

	<!-- Discovery Section -->
	<section class="section">
		<div class="section-header">
			<div>
				<h2>Discover Cameras</h2>
				<p class="section-subtitle">Find ONVIF cameras on your network</p>
			</div>
			<button class="btn btn-secondary" onclick={startScan} disabled={scanning}>
				{#if scanning}
					<span class="spinner"></span>
					Scanning...
				{:else}
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<circle cx="11" cy="11" r="8"/>
						<path d="m21 21-4.35-4.35"/>
					</svg>
					Scan Network
				{/if}
			</button>
		</div>

		{#if scanError}
			<div class="alert alert-error">
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<circle cx="12" cy="12" r="10"/>
					<line x1="12" y1="8" x2="12" y2="12"/>
					<line x1="12" y1="16" x2="12.01" y2="16"/>
				</svg>
				{scanError}
			</div>
		{/if}

		{#if unOnboardedDevices.length > 0}
			<div class="device-grid">
				{#each unOnboardedDevices as device (device.id)}
					<DiscoveredDeviceCard {device} onAdd={() => handleAddFromDevice(device)} />
				{/each}
			</div>
		{:else if !scanning}
			<div class="empty-state">
				<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" opacity="0.5">
					<path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/>
					<circle cx="12" cy="13" r="4"/>
				</svg>
				<p>No new cameras discovered</p>
				<p class="text-muted">Click "Scan Network" to discover ONVIF cameras</p>
			</div>
		{/if}
	</section>

	<!-- Cameras Section -->
	<section class="section">
		<div class="section-header">
			<div>
				<h2>My Cameras</h2>
				<p class="section-subtitle">{cameras.length} camera{cameras.length !== 1 ? 's' : ''} configured</p>
			</div>
			<button class="btn btn-secondary" onclick={() => { selectedDeviceForAdd = null; showAddModal = true; }}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<line x1="12" y1="5" x2="12" y2="19"/>
					<line x1="5" y1="12" x2="19" y2="12"/>
				</svg>
				Add Manually
			</button>
		</div>

		{#if cameras.length > 0}
			<div class="camera-grid">
				{#each cameras as camera (camera.id)}
					<CameraCard 
						{camera} 
						onProbe={() => handleProbe(camera)}
						onCredentials={() => handleSetCredentials(camera)}
						onDelete={() => handleDelete(camera)}
					/>
				{/each}
			</div>
		{:else}
			<div class="empty-state">
				<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" opacity="0.5">
					<rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
					<line x1="8" y1="21" x2="16" y2="21"/>
					<line x1="12" y1="17" x2="12" y2="21"/>
				</svg>
				<p>No cameras configured yet</p>
				<p class="text-muted">Discover cameras on your network or add one manually</p>
			</div>
		{/if}
	</section>
</div>

<!-- Modals -->
{#if showAddModal}
	<AddCameraModal 
		device={selectedDeviceForAdd}
		onClose={() => { showAddModal = false; selectedDeviceForAdd = null; }}
		onAdd={handleAddCamera}
	/>
{/if}

{#if credentialsModalCamera}
	<CredentialsModal
		camera={credentialsModalCamera}
		onClose={() => credentialsModalCamera = null}
		onSave={handleSaveCredentials}
	/>
{/if}

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 2rem;
	}

	.status-bar {
		display: flex;
		gap: 2rem;
		padding: 1rem 1.5rem;
		background: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-lg);
	}

	.status-item {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
	}

	.status-label {
		font-size: 0.75rem;
		color: var(--color-text-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.status-value {
		font-size: 0.875rem;
		font-weight: 500;
	}

	.section {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.section-header {
		display: flex;
		justify-content: space-between;
		align-items: flex-start;
	}

	.section-header h2 {
		font-size: 1.25rem;
		font-weight: 600;
	}

	.section-subtitle {
		font-size: 0.875rem;
		color: var(--color-text-muted);
		margin-top: 0.25rem;
	}

	.device-grid, .camera-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
		gap: 1rem;
	}

	.empty-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		padding: 3rem;
		text-align: center;
		color: var(--color-text-muted);
		background: var(--color-bg-secondary);
		border: 1px dashed var(--color-border);
		border-radius: var(--radius-lg);
	}

	.empty-state p {
		margin-top: 1rem;
	}

	.empty-state .text-muted {
		font-size: 0.875rem;
		margin-top: 0.25rem;
	}

	.quick-actions {
		display: flex;
		gap: 1rem;
		padding: 1.5rem;
		background: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-lg);
	}

	.btn-large {
		padding: 0.875rem 1.5rem;
		font-size: 1rem;
	}

	.alert {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.75rem 1rem;
		border-radius: var(--radius);
	}

	.alert-error {
		background: rgba(239, 68, 68, 0.1);
		border: 1px solid rgba(239, 68, 68, 0.3);
		color: var(--color-error);
	}

	.spinner {
		width: 16px;
		height: 16px;
		border: 2px solid transparent;
		border-top-color: currentColor;
		border-radius: 50%;
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}
</style>
