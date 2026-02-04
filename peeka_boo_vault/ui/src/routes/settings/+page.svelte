<script lang="ts">
	import { onMount } from 'svelte';

	interface Config {
		server: {
			bind_address: string;
			static_dir: string | null;
		};
		discovery: {
			default_duration_secs: number;
			auto_discover: boolean;
			scan_interval_secs: number;
		};
		storage: {
			hot_db_path: string;
			cold_db_path: string;
			hot_storage_path: string;
			cold_storage_path: string;
			retention: {
				hot_quota_bytes: number;
				cold_quota_bytes: number;
				hot_max_age_secs: number;
				cold_max_age_secs: number;
				migration_interval_secs: number;
				migration_threshold: number;
			};
		};
		webrtc: {
			bind_address: string;
			enabled: boolean;
			max_sessions_per_camera: number;
			session_timeout_secs: number;
		};
	}

	interface StorageUsage {
		hot_path: string;
		hot_used_bytes: number;
		hot_quota_bytes: number;
		hot_usage_percent: number;
		cold_path: string;
		cold_used_bytes: number;
		cold_quota_bytes: number;
		cold_usage_percent: number;
	}

	let config: Config | null = null;
	let storageUsage: StorageUsage | null = null;
	let loading = true;
	let saving = false;
	let error: string | null = null;
	let successMessage: string | null = null;

	onMount(async () => {
		await loadSettings();
	});

	async function loadSettings() {
		try {
			loading = true;
			error = null;

			// Fetch config
			const configRes = await fetch('/api/config');
			const configData = await configRes.json();
			if (configData.success) {
				config = configData.data;
			} else {
				throw new Error(configData.error || 'Failed to load configuration');
			}

			// Fetch storage usage
			const storageRes = await fetch('/api/storage/usage');
			const storageData = await storageRes.json();
			if (storageData.success) {
				storageUsage = storageData.data;
			}
		} catch (e: any) {
			error = e.message;
		} finally {
			loading = false;
		}
	}

	async function saveSettings() {
		if (!config) return;

		try {
			saving = true;
			error = null;
			successMessage = null;

			const res = await fetch('/api/config', {
				method: 'PATCH',
				headers: {
					'Content-Type': 'application/json'
				},
				body: JSON.stringify(config)
			});

			const data = await res.json();
			if (data.success) {
				successMessage =
					'Configuration saved successfully. Some changes may require a server restart to take effect.';
				await loadSettings();
			} else {
				throw new Error(data.error || 'Failed to save configuration');
			}
		} catch (e: any) {
			error = e.message;
		} finally {
			saving = false;
		}
	}

	async function triggerMigration() {
		try {
			const res = await fetch('/api/storage/migration/trigger', {
				method: 'POST'
			});

			if (res.ok) {
				successMessage = 'Migration triggered successfully';
				await loadSettings(); // Refresh storage usage
			} else {
				throw new Error('Failed to trigger migration');
			}
		} catch (e: any) {
			error = e.message;
		}
	}

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
	}

	function formatDuration(seconds: number): string {
		if (seconds === 0) return 'Unlimited';
		const days = Math.floor(seconds / 86400);
		if (days > 0) return `${days} days`;
		const hours = Math.floor(seconds / 3600);
		if (hours > 0) return `${hours} hours`;
		const minutes = Math.floor(seconds / 60);
		return `${minutes} minutes`;
	}
</script>

<svelte:head>
	<title>Settings | PeekabooVault</title>
</svelte:head>

<div class="page">
	<h1>Settings</h1>
	<p class="subtitle">Configure your PeekabooVault instance</p>

	{#if loading}
		<div class="loading">Loading configuration...</div>
	{:else if error}
		<div class="error-banner">
			<strong>Error:</strong>
			{error}
		</div>
	{/if}

	{#if successMessage}
		<div class="success-banner">
			{successMessage}
		</div>
	{/if}

	{#if config}
		<form on:submit|preventDefault={saveSettings}>
			<!-- Storage Settings -->
			<section class="section">
				<div class="card">
					<h2>Storage Configuration</h2>

					{#if storageUsage}
						<div class="storage-usage">
							<div class="usage-section">
								<h3>Hot Storage</h3>
								<p class="path">{storageUsage.hot_path}</p>
								<div class="progress-bar">
									<div
										class="progress-fill"
										style="width: {Math.min(storageUsage.hot_usage_percent, 100)}%"
										class:warning={storageUsage.hot_usage_percent > 80}
										class:danger={storageUsage.hot_usage_percent > 95}
									></div>
								</div>
								<p class="usage-text">
									{formatBytes(storageUsage.hot_used_bytes)} /
									{formatBytes(storageUsage.hot_quota_bytes)}
									({storageUsage.hot_usage_percent.toFixed(1)}% used)
								</p>
							</div>

							<div class="usage-section">
								<h3>Cold Storage</h3>
								<p class="path">{storageUsage.cold_path}</p>
								<div class="progress-bar">
									<div
										class="progress-fill"
										style="width: {Math.min(storageUsage.cold_usage_percent, 100)}%"
										class:warning={storageUsage.cold_usage_percent > 80}
										class:danger={storageUsage.cold_usage_percent > 95}
									></div>
								</div>
								<p class="usage-text">
									{formatBytes(storageUsage.cold_used_bytes)} /
									{formatBytes(storageUsage.cold_quota_bytes)}
									({storageUsage.cold_usage_percent.toFixed(1)}% used)
								</p>
							</div>
						</div>
					{/if}

					<div class="form-row">
						<label for="hot_storage_path">Hot Storage Path</label>
						<input
							type="text"
							id="hot_storage_path"
							bind:value={config.storage.hot_storage_path}
							required
						/>
					</div>

					<div class="form-row">
						<label for="hot_quota">Hot Storage Quota (GB)</label>
						<input
							type="number"
							id="hot_quota"
							bind:value={config.storage.retention.hot_quota_bytes}
							on:input={(e) => {
								const gb = parseFloat(e.currentTarget.value);
								config.storage.retention.hot_quota_bytes = gb * 1024 * 1024 * 1024;
							}}
							value={config.storage.retention.hot_quota_bytes / 1024 / 1024 / 1024}
							step="1"
							min="1"
							required
						/>
					</div>

					<div class="form-row">
						<label for="cold_storage_path">Cold Storage Path</label>
						<input
							type="text"
							id="cold_storage_path"
							bind:value={config.storage.cold_storage_path}
							required
						/>
					</div>

					<div class="form-row">
						<label for="cold_quota">Cold Storage Quota (GB)</label>
						<input
							type="number"
							id="cold_quota"
							bind:value={config.storage.retention.cold_quota_bytes}
							on:input={(e) => {
								const gb = parseFloat(e.currentTarget.value);
								config.storage.retention.cold_quota_bytes = gb * 1024 * 1024 * 1024;
							}}
							value={config.storage.retention.cold_quota_bytes / 1024 / 1024 / 1024}
							step="1"
							min="1"
							required
						/>
					</div>

					<div class="form-row">
						<label for="hot_max_age">Hot Storage Max Age (days, 0 = unlimited)</label>
						<input
							type="number"
							id="hot_max_age"
							bind:value={config.storage.retention.hot_max_age_secs}
							on:input={(e) => {
								const days = parseInt(e.currentTarget.value);
								config.storage.retention.hot_max_age_secs = days * 86400;
							}}
							value={config.storage.retention.hot_max_age_secs / 86400}
							step="1"
							min="0"
						/>
					</div>

					<div class="form-row">
						<label for="migration_threshold">Migration Threshold (%)</label>
						<input
							type="number"
							id="migration_threshold"
							bind:value={config.storage.retention.migration_threshold}
							on:input={(e) => {
								const percent = parseFloat(e.currentTarget.value);
								config.storage.retention.migration_threshold = percent / 100;
							}}
							value={config.storage.retention.migration_threshold * 100}
							step="1"
							min="0"
							max="100"
						/>
						<span class="help-text"
							>Start migrating to cold storage when hot storage reaches this percentage</span
						>
					</div>

					<div class="form-row">
						<button type="button" class="btn-secondary" on:click={triggerMigration}>
							Trigger Manual Migration
						</button>
					</div>
				</div>
			</section>

			<!-- Server Settings -->
			<section class="section">
				<div class="card">
					<h2>Server Configuration</h2>

					<div class="form-row">
						<label for="bind_address">HTTP Bind Address</label>
						<input
							type="text"
							id="bind_address"
							bind:value={config.server.bind_address}
							placeholder="0.0.0.0:8080"
							required
						/>
						<span class="help-text">Address and port for the HTTP API and UI</span>
					</div>
				</div>
			</section>

			<!-- Discovery Settings -->
			<section class="section">
				<div class="card">
					<h2>Discovery Configuration</h2>

					<div class="form-row">
						<label class="checkbox-label">
							<input type="checkbox" bind:checked={config.discovery.auto_discover} />
							<span>Auto-discover cameras on startup</span>
						</label>
					</div>

					<div class="form-row">
						<label for="scan_duration">Default Scan Duration (seconds)</label>
						<input
							type="number"
							id="scan_duration"
							bind:value={config.discovery.default_duration_secs}
							step="1"
							min="1"
							required
						/>
					</div>

					<div class="form-row">
						<label for="scan_interval">Periodic Scan Interval (seconds, 0 = disabled)</label>
						<input
							type="number"
							id="scan_interval"
							bind:value={config.discovery.scan_interval_secs}
							step="1"
							min="0"
						/>
					</div>
				</div>
			</section>

			<!-- WebRTC Settings -->
			<section class="section">
				<div class="card">
					<h2>WebRTC Configuration</h2>

					<div class="form-row">
						<label class="checkbox-label">
							<input type="checkbox" bind:checked={config.webrtc.enabled} />
							<span>Enable WebRTC server</span>
						</label>
					</div>

					<div class="form-row">
						<label for="webrtc_bind">WebRTC Bind Address</label>
						<input
							type="text"
							id="webrtc_bind"
							bind:value={config.webrtc.bind_address}
							placeholder="0.0.0.0:10000"
							required
						/>
						<span class="help-text">UDP address and port for WebRTC media</span>
					</div>

					<div class="form-row">
						<label for="max_sessions">Max Sessions Per Camera</label>
						<input
							type="number"
							id="max_sessions"
							bind:value={config.webrtc.max_sessions_per_camera}
							step="1"
							min="1"
							required
						/>
					</div>

					<div class="form-row">
						<label for="session_timeout">Session Timeout (seconds)</label>
						<input
							type="number"
							id="session_timeout"
							bind:value={config.webrtc.session_timeout_secs}
							step="1"
							min="10"
							required
						/>
					</div>
				</div>
			</section>

			<!-- Actions -->
			<div class="actions">
				<button type="submit" class="btn-primary" disabled={saving}>
					{saving ? 'Saving...' : 'Save Configuration'}
				</button>
			</div>
		</form>
	{/if}

	<!-- About -->
	<section class="section">
		<div class="card">
			<h2>About</h2>
			<div class="about-info">
				<div class="about-row">
					<span class="about-label">Application</span>
					<span class="about-value">PeekabooVault NVR</span>
				</div>
				<div class="about-row">
					<span class="about-label">Status</span>
					<span class="about-value">Phase 1-6 Complete</span>
				</div>
				<div class="about-row">
					<span class="about-label">Repository</span>
					<a href="https://github.com/xnorpx/PeekabooVault" target="_blank" rel="noopener">
						github.com/xnorpx/PeekabooVault
					</a>
				</div>
			</div>
		</div>
	</section>
</div>

<style>
	.page {
		max-width: 900px;
	}

	h1 {
		font-size: 1.5rem;
		font-weight: 600;
		margin-bottom: 0.25rem;
	}

	.subtitle {
		color: var(--color-text-muted);
		margin-bottom: 2rem;
	}

	.loading {
		text-align: center;
		padding: 2rem;
		color: var(--color-text-muted);
	}

	.error-banner {
		background: var(--color-error);
		color: white;
		padding: 1rem;
		border-radius: var(--radius);
		margin-bottom: 1.5rem;
	}

	.success-banner {
		background: var(--color-success);
		color: white;
		padding: 1rem;
		border-radius: var(--radius);
		margin-bottom: 1.5rem;
	}

	.section {
		margin-bottom: 1.5rem;
	}

	.card {
		padding: 1.5rem;
	}

	.card h2 {
		font-size: 1rem;
		font-weight: 600;
		margin-bottom: 1rem;
	}

	.card h3 {
		font-size: 0.875rem;
		font-weight: 600;
		margin-bottom: 0.5rem;
		color: var(--color-text);
	}

	.storage-usage {
		display: grid;
		gap: 1.5rem;
		margin-bottom: 1.5rem;
		padding-bottom: 1.5rem;
		border-bottom: 1px solid var(--color-border);
	}

	.usage-section {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	.path {
		font-size: 0.75rem;
		color: var(--color-text-muted);
		font-family: 'SF Mono', 'Consolas', monospace;
	}

	.progress-bar {
		height: 8px;
		background: var(--color-bg-tertiary);
		border-radius: 4px;
		overflow: hidden;
	}

	.progress-fill {
		height: 100%;
		background: var(--color-primary);
		transition: width 0.3s ease;
	}

	.progress-fill.warning {
		background: #f59e0b;
	}

	.progress-fill.danger {
		background: var(--color-error);
	}

	.usage-text {
		font-size: 0.8125rem;
		color: var(--color-text-muted);
	}

	.form-row {
		margin-bottom: 1rem;
	}

	.form-row label {
		display: block;
		font-size: 0.875rem;
		font-weight: 500;
		margin-bottom: 0.5rem;
		color: var(--color-text);
	}

	.form-row input[type='text'],
	.form-row input[type='number'] {
		width: 100%;
		padding: 0.5rem 0.75rem;
		font-size: 0.875rem;
		border: 1px solid var(--color-border);
		border-radius: var(--radius);
		background: var(--color-bg);
		color: var(--color-text);
	}

	.form-row input[type='text']:focus,
	.form-row input[type='number']:focus {
		outline: none;
		border-color: var(--color-primary);
	}

	.checkbox-label {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		cursor: pointer;
	}

	.checkbox-label input[type='checkbox'] {
		cursor: pointer;
	}

	.help-text {
		display: block;
		font-size: 0.75rem;
		color: var(--color-text-muted);
		margin-top: 0.25rem;
	}

	.actions {
		display: flex;
		gap: 1rem;
		margin-top: 2rem;
	}

	.btn-primary,
	.btn-secondary {
		padding: 0.625rem 1.25rem;
		font-size: 0.875rem;
		font-weight: 500;
		border-radius: var(--radius);
		cursor: pointer;
		transition: all 0.15s ease;
		border: none;
	}

	.btn-primary {
		background: var(--color-primary);
		color: white;
	}

	.btn-primary:hover:not(:disabled) {
		background: var(--color-primary-hover);
	}

	.btn-primary:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.btn-secondary {
		background: var(--color-bg-tertiary);
		color: var(--color-text);
		border: 1px solid var(--color-border);
	}

	.btn-secondary:hover {
		background: var(--color-bg-secondary);
	}

	.about-info {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		margin-top: 1rem;
	}

	.about-row {
		display: flex;
		gap: 1rem;
		font-size: 0.875rem;
	}

	.about-label {
		color: var(--color-text-muted);
		min-width: 100px;
	}
</style>
