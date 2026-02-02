<script lang="ts">
	// Settings page placeholder
</script>

<svelte:head>
	<title>Settings | PeekabooVault</title>
</svelte:head>

<div class="page">
	<h1>Settings</h1>
	<p class="subtitle">Configure your PeekabooVault instance</p>

	<section class="section">
		<div class="card">
			<h2>Server Configuration</h2>
			<p class="text-muted">Server settings are configured via the config.toml file.</p>
			
			<div class="config-preview">
				<pre><code>[server]
bind_address = "0.0.0.0:8080"

[discovery]
default_duration_secs = 5
auto_discover = true
scan_interval_secs = 300

[storage]
db_path = "./data/peekaboovault.db"
hot_storage_path = "./data/recordings/hot"</code></pre>
			</div>
		</div>
	</section>

	<section class="section">
		<div class="card">
			<h2>About</h2>
			<div class="about-info">
				<div class="about-row">
					<span class="about-label">Application</span>
					<span class="about-value">PeekabooVault NVR</span>
				</div>
				<div class="about-row">
					<span class="about-label">Phase</span>
					<span class="about-value">Phase 1 - ONVIF Discovery</span>
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
		max-width: 800px;
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

	.section {
		margin-bottom: 1.5rem;
	}

	.card h2 {
		font-size: 1rem;
		font-weight: 600;
		margin-bottom: 0.5rem;
	}

	.text-muted {
		color: var(--color-text-muted);
		font-size: 0.875rem;
	}

	.config-preview {
		margin-top: 1rem;
		padding: 1rem;
		background: var(--color-bg-tertiary);
		border-radius: var(--radius);
		overflow-x: auto;
	}

	.config-preview pre {
		margin: 0;
		font-family: 'SF Mono', 'Consolas', monospace;
		font-size: 0.8125rem;
		line-height: 1.5;
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
