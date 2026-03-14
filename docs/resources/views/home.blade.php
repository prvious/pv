<x-layouts.app title="pv — Local PHP development, zero config">
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Nav + Hero + Trust Bar                                       --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="bg-background">

        <x-site-header />

        {{-- Hero --}}
        <div class="flex flex-col items-center gap-8 px-6 py-16 md:px-20 md:py-24">
            {{-- Badge --}}
            <div class="rounded-full bg-card px-3 py-1.5 font-mono text-[11px] font-semibold text-accent">
                [ AUTOMATION READY ]
            </div>

            {{-- Headline --}}
            <h1 class="max-w-5xl text-center font-heading text-5xl font-bold leading-[1.1] text-foreground md:text-[60px]">
                One command. Full PHP environment.
            </h1>

            {{-- Subtitle --}}
            <p class="max-w-[700px] text-center font-mono text-[15px] leading-relaxed text-muted">
                Install pv with a single curl, then set up PHP, FrankenPHP, Composer and Mago — no Docker, no Nginx, no config files.
            </p>

            <x-install-command />

            {{-- CTA Buttons --}}
            <div class="flex items-center gap-4">
                <a href="#get_started" class="rounded-2xl bg-accent-orange px-7 py-3.5 font-mono text-[13px] font-semibold text-on-accent transition-colors hover:bg-accent-orange/90">
                    pv install
                </a>
                <a href="https://pv.prvious.dev/docs" class="rounded-2xl border border-muted px-7 py-3.5 font-mono text-[13px] font-semibold text-foreground transition-colors hover:bg-elevated">
                    Read Docs
                </a>
            </div>

            {{-- Terminal Mockup --}}
            <div class="w-full max-w-[780px] overflow-hidden rounded-2xl bg-elevated">
                {{-- Title bar --}}
                <div class="flex items-center gap-2 bg-card px-4 py-3">
                    <span class="size-3 rounded-full bg-accent-red"></span>
                    <span class="size-3 rounded-full bg-accent-orange"></span>
                    <span class="size-3 rounded-full bg-accent"></span>
                    <span class="ml-2 font-mono text-[11px] text-muted">terminal</span>
                </div>
                {{-- Body --}}
                <div class="space-y-1.5 p-5 font-mono text-[12px] leading-relaxed md:text-[13px]">
                    <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">curl</span><span class="font-semibold text-muted"> -fsSL </span><span class="font-semibold text-foreground">https://pv.prvious.dev/install</span><span class="font-semibold text-muted"> | </span><span class="font-semibold text-accent-orange">bash</span></div>
                    <div class="text-accent">  ✓ pv installed to ~/.pv/bin/pv</div>
                    <div class="h-2"></div>
                    <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="font-semibold text-accent-orange"> setup</span></div>
                    <div class="h-2"></div>
                    <div class="text-accent">  ? Select PHP versions: [8.4] [8.3]</div>
                    <div class="text-accent">  ? Select tools: [composer] [mago]</div>
                    <div class="text-accent">  ? Select services: [mysql] [postgres]</div>
                    <div class="text-accent">  ? Configure domain: [.test]</div>
                    <div class="h-2"></div>
                    <div class="text-muted">  ✓ Setup saved. Run pv link in your project.</div>
                </div>
            </div>
        </div>

        {{-- Trust Bar --}}
        <div class="flex flex-col items-center justify-center gap-4 px-6 py-6 md:flex-row">
            <span class="font-mono text-[9px] text-muted">// built_with</span>
            <div class="flex flex-wrap items-center justify-center gap-2.5">
                @foreach (['Go', 'FrankenPHP', 'Caddy', 'Docker'] as $tech)
                    <span class="rounded-full bg-card px-3 py-1.5 font-mono text-[11px] font-semibold text-foreground">{{ $tech }}</span>
                @endforeach
            </div>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Quick Install                                                --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="flex flex-col items-center gap-8 bg-background px-6 py-16 md:px-20">
        {{-- Header --}}
        <div class="flex flex-col items-center gap-2">
            <span class="font-mono text-[13px] text-muted">// quick_setup</span>
            <h2 class="font-heading text-5xl font-bold text-foreground">Setup in seconds.</h2>
        </div>

        {{-- Terminal --}}
        <div class="w-full max-w-[720px] overflow-hidden rounded-2xl bg-elevated shadow-[0_8px_32px_rgba(26,26,26,0.8)]">
            <div class="flex items-center gap-2 px-5 py-4">
                <span class="size-3 rounded-full bg-accent-red"></span>
                <span class="size-3 rounded-full bg-accent-orange"></span>
                <span class="size-3 rounded-full bg-accent"></span>
            </div>
            <div class="h-px bg-placeholder"></div>
            <div class="p-6 font-mono text-[17px]">
                <span class="text-muted">$ </span>
                <span class="font-semibold text-accent">curl</span>
                <span class="text-muted"> -fsSL </span>
                <span class="text-foreground">https://pv.prvious.dev/install</span>
                <span class="text-muted"> | </span>
                <span class="font-semibold text-accent">bash</span>
            </div>
        </div>

        {{-- Explain --}}
        <p class="font-mono text-[13px] text-muted">Then run pv setup to configure PHP, FrankenPHP, Composer and Mago.</p>

        {{-- Flow indicator --}}
        <div class="flex items-center gap-4">
            <span class="rounded-full bg-card px-4 py-2 font-mono text-[11px] font-semibold text-accent">curl install</span>
            <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-muted"><path d="m9 18 6-6-6-6"/></svg>
            <span class="rounded-full bg-card px-4 py-2 font-mono text-[11px] font-semibold text-accent-orange">pv setup</span>
            <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-muted"><path d="m9 18 6-6-6-6"/></svg>
            <span class="flex items-center gap-1.5 rounded-full bg-accent px-4 py-2 font-mono text-[11px] font-semibold text-on-accent">
                <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                ready
            </span>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Features + How It Works                                      --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="space-y-16 px-6 py-20 md:px-20" id="features">

        {{-- Features --}}
        <div class="mx-auto max-w-6xl space-y-10">
            <div class="space-y-2">
                <span class="font-mono text-[13px] text-muted">// core_features</span>
                <h2 class="font-heading text-4xl font-bold text-foreground">Everything you need. Nothing you don't.</h2>
            </div>

            <div class="grid gap-6 md:grid-cols-3">
                {{-- Card 1 --}}
                <div class="space-y-4 rounded-2xl bg-elevated p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange"><path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/></svg>
                    <h3 class="font-heading text-xl font-semibold text-foreground">PHP Version Manager</h3>
                    <p class="font-mono text-[13px] leading-relaxed text-muted">Install and switch between PHP versions instantly. Per-project version support via pv.yml or composer.json.</p>
                </div>
                {{-- Card 2 --}}
                <div class="space-y-4 rounded-2xl bg-elevated p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>
                    <h3 class="font-heading text-xl font-semibold text-foreground">HTTPS Out of the Box</h3>
                    <p class="font-mono text-[13px] leading-relaxed text-muted">Every linked project gets automatic HTTPS at project.test domains. No certificates to manage.</p>
                </div>
                {{-- Card 3 --}}
                <div class="space-y-4 rounded-2xl bg-elevated p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/></svg>
                    <h3 class="font-heading text-xl font-semibold text-foreground">Backing Services</h3>
                    <p class="font-mono text-[13px] leading-relaxed text-muted">MySQL, PostgreSQL, Redis, Mailpit, MinIO — containerized and managed. One command to add.</p>
                </div>
            </div>
        </div>

        {{-- How It Works --}}
        <div class="mx-auto max-w-6xl space-y-10" id="how_it_works">
            <div class="space-y-2">
                <span class="font-mono text-[13px] text-muted">// how_it_works</span>
                <h2 class="font-heading text-4xl font-bold text-foreground">From zero to serving in two steps.</h2>
            </div>

            <div class="grid gap-6 md:grid-cols-3">
                @foreach ([
                    ['01', 'curl -fsSL .../install | bash', 'Downloads the pv binary to your machine. One line, no dependencies.'],
                    ['02', 'pv setup', 'Installs PHP, FrankenPHP, Composer, and Mago in one guided setup.'],
                    ['03', 'your environment is ready', 'After setup, link a project when needed and start building immediately.'],
                ] as [$num, $cmd, $desc])
                    <div class="space-y-4 rounded-2xl bg-elevated p-6">
                        <span class="inline-block rounded-full bg-accent-orange px-3 py-1.5 font-mono text-xs font-semibold text-on-accent">{{ $num }}</span>
                        <h3 class="font-mono text-base font-semibold text-foreground">{{ $cmd }}</h3>
                        <p class="font-mono text-[13px] leading-relaxed text-muted">{{ $desc }}</p>
                    </div>
                @endforeach
            </div>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Terminal Showcase                                            --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="bg-background px-6 py-20 md:px-20">
        <div class="mx-auto max-w-6xl space-y-10">
            <div class="flex flex-col items-center gap-3">
                <span class="font-mono text-[13px] text-muted">// see_it_in_action</span>
                <h2 class="text-center font-heading text-4xl font-bold text-foreground">YOUR ENTIRE WORKFLOW. ONE TOOL.</h2>
                <p class="text-center font-mono text-[13px] text-muted">Install with curl, run pv setup, and your local PHP environment is ready.</p>
            </div>

            <div class="grid gap-6 md:grid-cols-2">
                {{-- Terminal: PHP Versions --}}
                <div class="overflow-hidden rounded-2xl bg-card">
                    <div class="flex items-center justify-between bg-elevated px-4 py-3">
                        <span class="font-mono text-xs font-semibold text-muted">TERMINAL:~PHP_VERSIONS</span>
                        <div class="flex gap-1.5">
                            <span class="size-2.5 rounded-full bg-muted"></span>
                            <span class="size-2.5 rounded-full bg-accent"></span>
                            <span class="size-2.5 rounded-full bg-accent-orange"></span>
                        </div>
                    </div>
                    <div class="space-y-1.5 p-5 font-mono text-[13px] leading-relaxed">
                        <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> php:install </span><span class="text-muted">8.4</span></div>
                        <div class="text-muted">  Downloading PHP 8.4.3...</div>
                        <div class="text-accent">  PHP 8.4.3 installed successfully</div>
                        <div class="h-2"></div>
                        <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> php:use </span><span class="text-muted">8.4</span></div>
                        <div class="text-accent">  Global PHP version set to 8.4</div>
                        <div class="h-2"></div>
                        <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> php:list</span></div>
                        <div class="text-xs font-semibold text-muted">  VERSION    STATUS      PATH</div>
                        <div class="text-xs text-placeholder">  ─────────────────────────────────────</div>
                        <div class="text-xs text-muted">  8.3        installed   ~/.pv/php/8.3</div>
                        <div class="text-xs text-accent-orange">* 8.4        active      ~/.pv/php/8.4</div>
                    </div>
                </div>

                {{-- Terminal: Services --}}
                <div class="overflow-hidden rounded-2xl bg-card">
                    <div class="flex items-center justify-between bg-elevated px-4 py-3">
                        <span class="font-mono text-xs font-semibold text-muted">TERMINAL:~SERVICES</span>
                        <div class="flex gap-1.5">
                            <span class="size-2.5 rounded-full bg-muted"></span>
                            <span class="size-2.5 rounded-full bg-accent"></span>
                            <span class="size-2.5 rounded-full bg-accent-orange"></span>
                        </div>
                    </div>
                    <div class="space-y-1.5 p-5 font-mono text-[13px] leading-relaxed">
                        <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> service:add </span><span class="text-muted">mysql</span></div>
                        <div class="text-muted">  Pulling mysql:8.0...</div>
                        <div class="text-accent">  MySQL 8.0 running on 127.0.0.1:3306</div>
                        <div class="h-2"></div>
                        <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> service:add </span><span class="text-muted">redis</span></div>
                        <div class="text-accent">  Redis 7 running on 127.0.0.1:6379</div>
                        <div class="h-2"></div>
                        <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> service:list</span></div>
                        <div class="text-xs font-semibold text-muted">  SERVICE     STATUS     PORT</div>
                        <div class="text-xs text-placeholder">  ─────────────────────────────────────</div>
                        <div class="text-xs text-accent">  mysql       running    3306</div>
                        <div class="text-xs text-accent">  redis       running    6379</div>
                    </div>
                </div>
            </div>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Final CTA                                                    --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="flex flex-col items-center gap-8 bg-card px-6 py-20 md:px-20" id="get_started">
        <span class="font-mono text-[13px] text-muted">// get_started</span>
        <h2 class="text-center font-heading text-5xl font-bold text-foreground">STOP CONFIGURING. START BUILDING.</h2>
        <p class="text-center font-mono text-sm text-muted">One curl. Then pv setup. Your PHP environment in under a minute.</p>

        <x-install-command />

        {{-- CTA buttons --}}
        <div class="flex items-center gap-4">
            <a href="https://pv.prvious.dev/docs" class="flex items-center gap-2 rounded-2xl bg-accent-orange px-8 py-3.5 font-mono text-sm font-semibold text-on-accent transition-colors hover:bg-accent-orange/90">
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" x2="20" y1="19" y2="19"/></svg>
                Then: pv setup
            </a>
            <a href="https://github.com/prvious/pv" class="flex items-center gap-2 rounded-2xl border border-muted px-8 py-3.5 font-mono text-sm font-semibold text-foreground transition-colors hover:bg-elevated">
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/></svg>
                View on GitHub
            </a>
        </div>

        <p class="font-mono text-[11px] text-muted">Free and open source. Works on macOS. No sudo required.</p>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- FOOTER                                                                --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <x-site-footer />
</x-layouts.app>
