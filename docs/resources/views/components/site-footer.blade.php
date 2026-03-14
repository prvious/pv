<footer class="bg-elevated px-6 py-12 md:px-20">
    <div class="mx-auto max-w-6xl space-y-12">
        <div class="flex flex-col gap-10 md:flex-row md:justify-between">
            <div class="max-w-[280px] space-y-3">
                <a href="/" class="font-heading text-[28px] font-bold text-accent-orange">pv</a>
                <p class="font-mono text-xs leading-relaxed text-muted">Local dev environments,<br>powered by FrankenPHP.</p>
            </div>

            <div class="flex gap-16">
                <div class="space-y-3">
                    <h4 class="font-heading text-[13px] font-semibold text-foreground">PRODUCT</h4>
                    <div class="flex flex-col gap-3 font-mono text-xs text-muted">
                        <a href="#features" class="transition-colors hover:text-foreground">Features</a>
                        <a href="#get_started" class="transition-colors hover:text-foreground">Installation</a>
                        <a href="https://pv.prvious.dev/docs" class="transition-colors hover:text-foreground">Documentation</a>
                        <a href="https://github.com/prvious/pv/releases" class="transition-colors hover:text-foreground">Changelog</a>
                    </div>
                </div>
                <div class="space-y-3">
                    <h4 class="font-heading text-[13px] font-semibold text-foreground">RESOURCES</h4>
                    <div class="flex flex-col gap-3 font-mono text-xs text-muted">
                        <a href="https://pv.prvious.dev/docs" class="transition-colors hover:text-foreground">Getting Started</a>
                        <a href="https://pv.prvious.dev/docs" class="transition-colors hover:text-foreground">PHP Versions</a>
                        <a href="https://pv.prvious.dev/docs" class="transition-colors hover:text-foreground">Services Guide</a>
                        <a href="https://pv.prvious.dev/docs" class="transition-colors hover:text-foreground">Troubleshooting</a>
                    </div>
                </div>
                <div class="space-y-3">
                    <h4 class="font-heading text-[13px] font-semibold text-foreground">COMMUNITY</h4>
                    <div class="flex flex-col gap-3 font-mono text-xs text-muted">
                        <a href="https://github.com/prvious/pv" class="transition-colors hover:text-foreground">GitHub</a>
                        <a href="https://github.com/prvious/pv/issues" class="transition-colors hover:text-foreground">Issues</a>
                        <a href="https://github.com/prvious/pv/discussions" class="transition-colors hover:text-foreground">Discussions</a>
                        <a href="https://github.com/prvious/pv/blob/main/CONTRIBUTING.md" class="transition-colors hover:text-foreground">Contributing</a>
                    </div>
                </div>
            </div>
        </div>

        <div class="h-px bg-placeholder"></div>

        <div class="flex flex-col gap-2 font-mono text-[11px] text-muted md:flex-row md:justify-between">
            <span>&copy; {{ date('Y') }} pv. Open source under MIT License.</span>
            <span>Built with FrankenPHP + Go</span>
        </div>
    </div>
</footer>
