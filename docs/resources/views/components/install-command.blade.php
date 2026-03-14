<div {{ $attributes->cn('flex items-center gap-3 rounded-2xl border border-placeholder bg-elevated px-6 py-3.5 font-mono text-sm') }}>
    <span class="font-semibold text-muted">$</span>
    <span class="font-semibold text-accent">curl</span>
    <span class="text-muted"> -fsSL </span>
    <span class="text-foreground">https://pv.prvious.dev/install</span>
    <span class="text-muted"> | </span>
    <span class="font-semibold text-accent">bash</span>
    <button class="ml-2 text-foreground transition-colors hover:text-accent" title="Copy">
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>
    </button>
</div>
