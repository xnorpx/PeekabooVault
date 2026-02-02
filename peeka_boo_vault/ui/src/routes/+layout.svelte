<script lang="ts">
	import '../app.css';
	
	let { children } = $props();
</script>

<div class="app">
	<header>
		<div class="header-content">
			<a href="/" class="logo">
				<svg width="32" height="32" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
					<circle cx="16" cy="16" r="14" stroke="currentColor" stroke-width="2"/>
					<circle cx="16" cy="16" r="6" fill="currentColor"/>
					<circle cx="16" cy="16" r="2" fill="var(--color-bg)"/>
				</svg>
				<span>PeekabooVault</span>
			</a>
			<nav>
				<a href="/">Cameras</a>
				<a href="/settings">Settings</a>
			</nav>
		</div>
	</header>

	<main>
		{@render children()}
	</main>
</div>

<style>
	.app {
		display: flex;
		flex-direction: column;
		min-height: 100vh;
	}

	header {
		background: var(--color-bg-secondary);
		border-bottom: 1px solid var(--color-border);
		padding: 0 1.5rem;
	}

	.header-content {
		display: flex;
		align-items: center;
		justify-content: space-between;
		max-width: 1400px;
		margin: 0 auto;
		height: 64px;
	}

	.logo {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		font-size: 1.25rem;
		font-weight: 600;
		color: var(--color-text);
	}

	.logo:hover {
		color: var(--color-text);
	}

	nav {
		display: flex;
		gap: 1.5rem;
	}

	nav a {
		color: var(--color-text-muted);
		font-size: 0.875rem;
		font-weight: 500;
		padding: 0.5rem 0;
		border-bottom: 2px solid transparent;
		transition: all 0.15s ease;
	}

	nav a:hover {
		color: var(--color-text);
	}

	main {
		flex: 1;
		padding: 2rem 1.5rem;
		max-width: 1400px;
		margin: 0 auto;
		width: 100%;
	}
</style>
