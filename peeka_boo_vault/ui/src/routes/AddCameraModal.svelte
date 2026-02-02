<script lang="ts">
	import type { DiscoveredDevice } from '$lib';

	interface Props {
		device: DiscoveredDevice | null;
		onClose: () => void;
		onAdd: (name: string, host: string, username?: string, password?: string) => Promise<void>;
	}

	let { device, onClose, onAdd }: Props = $props();

	// Extract initial values from device
	function getInitialHost(): string {
		if (device?.urls[0]) {
			try {
				const url = new URL(device.urls[0]);
				return url.hostname;
			} catch {
				return '';
			}
		}
		return '';
	}

	let name = $state(device?.name || device?.hardware || '');
	let host = $state(getInitialHost());
	let username = $state('admin');
	let password = $state('');
	let showPassword = $state(false);
	let adding = $state(false);
	let error = $state<string | null>(null);

	async function handleAdd() {
		if (!name.trim() || !host.trim()) return;
		adding = true;
		error = null;
		try {
			await onAdd(name.trim(), host.trim(), username.trim() || undefined, password || undefined);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to add camera';
		} finally {
			adding = false;
		}
	}

	function handleBackdropClick(e: MouseEvent) {
		if (e.target === e.currentTarget) {
			onClose();
		}
	}
</script>

<div class="modal-backdrop" onclick={handleBackdropClick} role="dialog" aria-modal="true" tabindex="-1">
	<div class="modal" onclick={(e) => e.stopPropagation()}>
		<div class="modal-header">
			<h2>Add Camera</h2>
			<button class="btn btn-icon btn-secondary" onclick={onClose} aria-label="Close">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<line x1="18" y1="6" x2="6" y2="18"/>
					<line x1="6" y1="6" x2="18" y2="18"/>
				</svg>
			</button>
		</div>

		<div class="form-content">
			<div class="form-group">
				<label for="camera-name">Camera Name</label>
				<input 
					id="camera-name"
					type="text" 
					bind:value={name}
					placeholder="e.g., Front Door Camera"
				/>
			</div>

			<div class="form-group">
				<label for="camera-host">Camera IP or Hostname</label>
				<input 
					id="camera-host"
					type="text" 
					bind:value={host}
					placeholder="192.168.1.100 or 192.168.1.100:8080"
				/>
				<span class="form-hint">We'll automatically try common ONVIF URL patterns</span>
			</div>

			<div class="form-divider">
				<span>Credentials</span>
			</div>

			<div class="form-group">
				<label for="username">Username</label>
				<input 
					id="username"
					type="text" 
					bind:value={username}
					placeholder="admin"
					autocomplete="username"
				/>
			</div>

			<div class="form-group">
				<label for="password">Password</label>
				<div class="password-input">
					<input 
						id="password"
						type={showPassword ? 'text' : 'password'}
						bind:value={password}
						placeholder="Enter password"
						autocomplete="current-password"
					/>
					<button 
						type="button" 
						class="btn btn-icon btn-secondary toggle-password"
						onclick={() => showPassword = !showPassword}
						aria-label={showPassword ? 'Hide password' : 'Show password'}
					>
						{#if showPassword}
							<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
								<path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/>
								<line x1="1" y1="1" x2="23" y2="23"/>
							</svg>
						{:else}
							<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
								<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/>
								<circle cx="12" cy="12" r="3"/>
							</svg>
						{/if}
					</button>
				</div>
			</div>

			{#if device}
				<div class="device-info">
					<span class="info-label">Discovered device:</span>
					<span class="info-value">{device.name || device.hardware || device.address}</span>
				</div>
			{/if}

			{#if error}
				<div class="error-message">
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<circle cx="12" cy="12" r="10"/>
						<line x1="12" y1="8" x2="12" y2="12"/>
						<line x1="12" y1="16" x2="12.01" y2="16"/>
					</svg>
					{error}
				</div>
			{/if}

			<div class="modal-actions">
				<button type="button" class="btn btn-secondary" onclick={onClose}>
					Cancel
				</button>
				<button type="button" class="btn btn-primary" disabled={!name.trim() || !host.trim() || adding} onclick={handleAdd}>
					{#if adding}
						<span class="spinner"></span>
						Connecting...
					{:else}
						Add & Connect
					{/if}
				</button>
			</div>
		</div>
	</div>
</div>

<style>
	.modal-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.7);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 1rem;
		z-index: 100;
	}

	.modal {
		background: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-lg);
		width: 100%;
		max-width: 480px;
		max-height: 90vh;
		overflow-y: auto;
	}

	.modal-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 1.25rem 1.5rem;
		border-bottom: 1px solid var(--color-border);
	}

	.modal-header h2 {
		font-size: 1.125rem;
		font-weight: 600;
	}

	.form-content {
		padding: 1.5rem;
		display: flex;
		flex-direction: column;
		gap: 1.25rem;
	}

	.form-group {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	.form-group label {
		font-size: 0.875rem;
		font-weight: 500;
	}

	.form-group input {
		width: 100%;
	}

	.form-hint {
		font-size: 0.75rem;
		color: var(--color-text-muted);
	}

	.form-divider {
		display: flex;
		align-items: center;
		gap: 1rem;
		color: var(--color-text-muted);
		font-size: 0.75rem;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.form-divider::before,
	.form-divider::after {
		content: '';
		flex: 1;
		height: 1px;
		background: var(--color-border);
	}

	.password-input {
		display: flex;
		gap: 0.5rem;
	}

	.password-input input {
		flex: 1;
	}

	.toggle-password {
		flex-shrink: 0;
	}

	.device-info {
		display: flex;
		gap: 0.5rem;
		padding: 0.75rem;
		background: var(--color-bg-tertiary);
		border-radius: var(--radius);
		font-size: 0.875rem;
	}

	.info-label {
		color: var(--color-text-muted);
	}

	.modal-actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.75rem;
		padding-top: 0.5rem;
	}

	.error-message {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.75rem;
		background: rgba(239, 68, 68, 0.1);
		border: 1px solid rgba(239, 68, 68, 0.3);
		border-radius: var(--radius);
		color: var(--color-error);
		font-size: 0.875rem;
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
