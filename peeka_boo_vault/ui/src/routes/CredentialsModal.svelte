<script lang="ts">
	import type { Camera } from '$lib';

	interface Props {
		camera: Camera;
		onClose: () => void;
		onSave: (username: string, password: string) => void;
	}

	let { camera, onClose, onSave }: Props = $props();

	let username = $state('');
	let password = $state('');
	let showPassword = $state(false);
	let saving = $state(false);

	async function handleSave() {
		if (!username.trim() || !password) return;
		saving = true;
		try {
			await onSave(username.trim(), password);
		} finally {
			saving = false;
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
			<h2>Set Credentials</h2>
			<button class="btn btn-icon btn-secondary" onclick={onClose} aria-label="Close">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
					<line x1="18" y1="6" x2="6" y2="18"/>
					<line x1="6" y1="6" x2="18" y2="18"/>
				</svg>
			</button>
		</div>

		<div class="form-content">
			<div class="camera-info">
				<span class="info-label">Camera:</span>
				<span class="info-value">{camera.name}</span>
			</div>

			<div class="form-group">
				<label for="username">Username</label>
				<input 
					id="username"
					type="text" 
					bind:value={username}
					placeholder="admin"
					autocomplete="username"
					required
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
						required
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

			<p class="form-note">
				Credentials will be used for ONVIF authentication and RTSP streaming.
			</p>

			<div class="modal-actions">
				<button type="button" class="btn btn-secondary" onclick={onClose}>
					Cancel
				</button>
				<button type="button" class="btn btn-primary" disabled={!username.trim() || !password || saving} onclick={handleSave}>
					{saving ? 'Saving...' : 'Save & Verify'}
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
		max-width: 420px;
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

	.camera-info {
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

	.form-note {
		font-size: 0.8125rem;
		color: var(--color-text-muted);
	}

	.modal-actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.75rem;
		padding-top: 0.5rem;
	}
</style>
