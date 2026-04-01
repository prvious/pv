<div
    x-data="{ copied: false }"
    <?php echo e($attributes->cn('flex items-center gap-3 rounded-2xl border border-placeholder bg-elevated px-6 py-3.5 font-mono text-sm')); ?>

>
    <span class="text-muted font-semibold">$</span>
    <span class="text-accent font-semibold">curl</span>
    <span class="text-muted"> -fsSL </span>
    <span class="text-foreground">https://pv.prvious.dev/install</span>
    <span class="text-muted"> | </span>
    <span class="text-accent font-semibold">bash</span>
    <button
        class="ml-2 transition-colors"
        :class="copied ? 'text-accent' : 'text-foreground hover:text-accent'"
        title="Copy"
        x-on:click="
            await navigator.clipboard.writeText(
                'curl -fsSL https://pv.prvious.dev/install | bash',
            );
            copied = true;
            setTimeout(() => (copied = false), 2000);
        "
    >
        
        <svg
            x-show="!copied"
            xmlns="http://www.w3.org/2000/svg"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
        >
            <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
            <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
        </svg>
        
        <svg x-show="copied" x-cloak xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
    </button>
</div>
<?php /**PATH /Users/clovismuneza/Apps/pv/docs/resources/views/components/install-command.blade.php ENDPATH**/ ?>