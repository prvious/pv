<x-layouts.app title="pv — Local PHP development, zero config">
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Nav + Hero + Trust Bar                                       --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <x-site-header />

    <div class="bg-background relative">
        {{-- Ambient glow --}}
        <div
            class="pointer-events-none absolute inset-x-0 top-16 h-125"
            style="
                mask-image: linear-gradient(to bottom, black 40%, transparent);
            "
            aria-hidden="true"
        >
            <div
                class="absolute top-0 left-1/2 h-100 w-200 -translate-x-1/2 rounded-full bg-white/20 blur-[120px]"
            ></div>
            <div
                class="absolute top-0 left-1/3 h-75 w-125 -translate-x-1/2 rounded-full bg-white/10 blur-[100px]"
            ></div>
        </div>

        {{-- Hero --}}
        <div
            class="flex flex-col items-center gap-8 px-6 pt-28 pb-16 md:px-20 md:pb-24"
        >
            <div
                class="bg-card text-accent rounded-full px-3 py-1.5 font-mono text-[11px] font-semibold"
            >
                [ LOCAL DEV, SOLVED ]
            </div>

            {{-- Headline --}}
            <h1
                class="font-heading text-foreground max-w-5xl text-center text-5xl leading-[1.1] font-bold md:text-[60px]"
            >
                Your entire PHP stack. One curl away.
            </h1>

            {{-- Subtitle --}}
            <p class="text-muted max-w-175 text-center font-mono text-[15px] leading-relaxed">pv replaces your Homebrew scripts, Docker configs and Nginx files with a single tool that just works. Install, link, ship.</p>

            <x-install-command />

            {{-- Supported Platforms --}}
            <div class="flex items-center gap-3">
                <span
                    class="border-accent bg-elevated text-foreground flex items-center gap-2 rounded-full border px-3 py-2 font-mono text-xs font-semibold"
                >
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 256 256" fill="currentColor" class="text-accent"><path d="M223.3,169.59a8.07,8.07,0,0,0-3.87-5.4c-18-10.06-26.2-31.17-26.2-53.22,0-20.06,6-34.14,18.38-46.28a8,8,0,0,0,1.35-9.67C201.41,34.65,179.79,24,155.49,24c-15.07,0-27.94,5.58-38.11,10a57.13,57.13,0,0,1-18.44,6.09C92.34,40.11,85.19,40,80.49,40A72.49,72.49,0,0,0,8,112.49c0,26.24,8.92,56.14,24.56,82.28C48.38,220.71,68.07,240,88.49,240h0a56.37,56.37,0,0,0,20-3.76l1.06-.42A56.37,56.37,0,0,1,131.3,232a57.16,57.16,0,0,1,22,4.18l.6.23A55.67,55.67,0,0,0,174,240c20.48,0,40.17-19.29,55.93-45.23A8,8,0,0,0,223.3,169.59Z" /></svg>
                    macOS
                </span>
                <span
                    class="border-accent-orange bg-elevated text-foreground flex items-center gap-2 rounded-full border px-3 py-2 font-mono text-xs font-semibold"
                >
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 256 256" fill="currentColor" class="text-accent-orange"><path d="M229,213.93a8,8,0,0,1-10.92,3,8.39,8.39,0,0,1-2.65-2.17,60.07,60.07,0,0,0-8.57-8.65,67.77,67.77,0,0,1-32.76,24.28A8,8,0,0,1,168,224H88a8,8,0,0,1-6.07-6.57,67.77,67.77,0,0,1-32.76-24.28,60.07,60.07,0,0,0-8.57,8.65,8.39,8.39,0,0,1-2.65,2.17A8,8,0,0,1,27,213.93a8.3,8.3,0,0,1,1.28-3.66,76.47,76.47,0,0,1,13.89-15.4A68,68,0,0,1,60,128V104a68,68,0,0,1,136,0v24a68,68,0,0,1,17.83,66.87,76.47,76.47,0,0,1,13.89,15.4A8.3,8.3,0,0,1,229,213.93ZM100,96a12,12,0,1,0,12,12A12,12,0,0,0,100,96Zm68,12a12,12,0,1,0-12,12A12,12,0,0,0,168,108Z" /></svg>
                    Linux
                </span>
                <span
                    class="border-accent bg-elevated text-foreground flex items-center gap-2 rounded-full border px-3 py-2 font-mono text-xs font-semibold"
                >
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 256 256" fill="currentColor" class="text-accent"><path d="M216,96H136V56h24a8,8,0,0,0,0-16H96a8,8,0,0,0,0,16h24V96H40A16,16,0,0,0,24,112V216a16,16,0,0,0,16,16H216a16,16,0,0,0,16-16V112A16,16,0,0,0,216,96ZM120,200H56V128h64Zm80,0H136V128h64Z" /></svg>
                    Windows
                </span>
            </div>

            <x-terminal
                title="terminal"
                class="w-full max-w-195 text-[12px] md:text-[13px]"
                x-data="{ tab: 'php' }"
            >
                {{-- curl command --}}
                <div>
                    <span class="text-muted font-semibold">$ </span
                    ><span class="text-accent font-semibold">curl</span
                    ><span class="text-muted font-semibold"> -fsSL </span
                    ><span class="text-foreground font-semibold"
                        >https://pv.prvious.dev/install</span
                    ><span class="text-muted font-semibold"> | </span
                    ><span class="text-accent-orange font-semibold">bash</span>
                </div>
                <div class="h-2"></div>

                {{-- Setup TUI --}}
                <div class="text-muted">Setting up environment...</div>
                <div class="h-4"></div>

                {{-- Tab bar --}}
                <div class="flex items-center">
                    <button
                        x-on:click="tab = 'php'"
                        class="cursor-pointer px-3 transition-colors"
                        :class="tab === 'php'
                            ? 'font-bold text-foreground'
                            : 'text-muted hover:text-foreground/70'"
                    >
                        PHP Versions
                    </button>
                    <span class="text-muted">│</span>
                    <button
                        x-on:click="tab = 'tools'"
                        class="cursor-pointer px-3 transition-colors"
                        :class="tab === 'tools'
                            ? 'font-bold text-foreground'
                            : 'text-muted hover:text-foreground/70'"
                    >
                        Tools
                    </button>
                    <span class="text-muted">│</span>
                    <button
                        x-on:click="tab = 'services'"
                        class="cursor-pointer px-3 transition-colors"
                        :class="tab === 'services'
                            ? 'font-bold text-foreground'
                            : 'text-muted hover:text-foreground/70'"
                    >
                        Services
                    </button>
                    <span class="text-muted">│</span>
                    <button
                        x-on:click="tab = 'settings'"
                        class="cursor-pointer px-3 transition-colors"
                        :class="tab === 'settings'
                            ? 'font-bold text-foreground'
                            : 'text-muted hover:text-foreground/70'"
                    >
                        Settings
                    </button>
                </div>
                <div class="bg-foreground/20 h-px w-full"></div>
                <div class="h-4"></div>

                {{-- PHP Versions --}}
                <div x-show="tab === 'php'">
                    <div class="text-muted">
                        Select which PHP versions to install:
                    </div>
                    <div class="h-3"></div>
                    <div>
                        <span class="text-accent font-bold">&gt; </span
                        ><span class="text-muted">○</span
                        ><span class="text-foreground"> PHP 8.3</span>
                    </div>
                    <div class="h-1"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-foreground">○ PHP 8.4</span>
                    </div>
                    <div class="h-1"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-accent">●</span
                        ><span class="text-foreground"> PHP 8.5 </span
                        ><span class="text-muted">(installed)</span>
                    </div>
                </div>

                {{-- Tools --}}
                <div x-show="tab === 'tools'" x-cloak>
                    <div class="text-muted">
                        Composer is always installed. Select additional tools:
                    </div>
                    <div class="h-3"></div>
                    <div>
                        <span class="text-accent font-bold">&gt; </span
                        ><span class="text-accent">●</span
                        ><span class="text-foreground"> Mago </span
                        ><span class="text-muted"
                            >(PHP linter &amp; formatter)</span
                        >
                    </div>
                </div>

                {{-- Services --}}
                <div x-show="tab === 'services'" x-cloak>
                    <div class="text-muted">
                        Select backing services to set up:
                    </div>
                    <div class="h-3"></div>
                    <div>
                        <span class="text-accent font-bold">&gt; </span
                        ><span class="text-accent">●</span
                        ><span class="text-foreground"> MySQL</span>
                    </div>
                    <div class="h-1"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-accent">●</span
                        ><span class="text-foreground"> PostgreSQL</span>
                    </div>
                    <div class="h-1"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-foreground">○ Redis</span>
                    </div>
                    <div class="h-1"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-foreground">○ Mail</span>
                    </div>
                    <div class="h-1"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-foreground">○ S3 Storage</span>
                    </div>
                </div>

                {{-- Settings --}}
                <div x-show="tab === 'settings'" x-cloak>
                    <div class="text-foreground font-bold">Domain</div>
                    <div class="text-muted">
                        Top-level domain for local sites
                    </div>
                    <div class="h-2"></div>
                    <div>
                        <span class="text-accent font-bold">&gt; </span
                        ><span class="text-foreground">test</span>
                        <span class="text-muted">(press e to edit)</span>
                    </div>
                    <div class="h-3"></div>
                    <div class="text-foreground font-bold">Daemon</div>
                    <div class="text-muted">
                        Start pv automatically on login
                    </div>
                    <div class="h-2"></div>
                    <div>
                        <span class="invisible">&gt; </span
                        ><span class="text-accent">true</span>
                    </div>
                </div>

                <div class="h-4"></div>

                {{-- Help bar --}}
                <div class="text-muted">
                    ←/→ tab • ↑/↓ move • space toggle • enter confirm • esc quit
                </div>
            </x-terminal>
        </div>

        {{-- Trust Bar --}}
        <div
            class="flex flex-col items-center justify-center gap-4 px-6 py-6 md:flex-row"
        >
            <span class="text-muted font-mono text-[9px]">// built_with</span>
            <div class="flex flex-wrap items-center justify-center gap-2.5">
                @foreach (['Go', 'FrankenPHP', 'Caddy', 'Docker'] as $tech)
                    <span
                        class="bg-card text-foreground rounded-full px-3 py-1.5 font-mono text-[11px] font-semibold"
                        >{{ $tech }}</span
                    >
                @endforeach
            </div>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Link a Project                                               --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="bg-background px-6 py-24 md:px-20">
        <div class="mx-auto max-w-6xl">
            <div class="grid items-center gap-16 md:grid-cols-2">
                {{-- Left: copy --}}
                <div class="space-y-6">
                    <span class="text-muted font-mono text-[13px]"
                        >// link_a_project</span
                    >
                    <h2 class="font-heading text-foreground text-4xl font-bold">
                        Link. Serve. Done.
                    </h2>
                    <p class="text-muted font-mono text-[13px] leading-relaxed">One command to link any PHP project. pv generates the Caddyfile, provisions a trusted HTTPS certificate, and detects your PHP version — all automatic.</p>
                    <div class="flex items-center gap-3 font-mono text-sm">
                        <span
                            class="bg-accent/10 text-accent rounded-full px-3 py-1"
                            >HTTPS</span
                        >
                        <span
                            class="bg-accent/10 text-accent rounded-full px-3 py-1"
                            >.test domains</span
                        >
                        <span
                            class="bg-accent/10 text-accent rounded-full px-3 py-1"
                            >auto PHP</span
                        >
                    </div>
                </div>

                {{-- Right: terminal --}}
                <x-terminal title="terminal" class="text-[13px]">
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-muted">cd ~/Code/myapp</span>
                    </div>
                    <div class="h-2"></div>
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> link</span>
                    </div>
                    <div class="h-2"></div>
                    <div class="text-muted">✓ myapp.caddy</div>
                    <div class="text-muted">✓ Caddyfile updated</div>
                    <div class="text-muted">✓ myapp.test</div>
                    <div class="text-muted">✓ no services detected</div>
                    <div class="text-muted">✓ https://myapp.test</div>
                    <div class="h-2"></div>
                    <div class="text-foreground">
                        ✓ Linked
                        <span class="text-accent font-bold"
                            >https://myapp.test</span
                        >
                    </div>
                    <div class="h-2"></div>
                    <div class="text-muted">Path ~/Code/myapp</div>
                    <div class="text-muted">Type laravel</div>
                    <div class="text-muted">
                        PHP <span class="text-accent">8.5</span>
                    </div>
                </x-terminal>
            </div>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Features                                                     --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div class="px-6 py-24 md:px-20" id="features">
        <div class="mx-auto max-w-6xl space-y-10">
            <div class="space-y-2">
                <span class="text-muted font-mono text-[13px]"
                    >// core_features</span
                >
                <h2 class="font-heading text-foreground text-4xl font-bold">
                    Everything you need. Nothing you don't.
                </h2>
            </div>

            <div class="grid gap-6 md:grid-cols-3">
                <div class="bg-elevated space-y-4 rounded-2xl p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange">
                        <path d="m17 2 4 4-4 4" />
                        <path d="M3 11v-1a4 4 0 0 1 4-4h14" />
                        <path d="m7 22-4-4 4-4" />
                        <path d="M21 13v1a4 4 0 0 1-4 4H3" />
                    </svg>
                    <h3
                        class="font-heading text-foreground text-xl font-semibold"
                    >
                        PHP Version Manager
                    </h3>
                    <p class="text-muted font-mono text-[13px] leading-relaxed">Install and switch between PHP versions instantly. Per-project version support via pv.yml or composer.json.</p>
                </div>
                <div class="bg-elevated space-y-4 rounded-2xl p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange">
                        <path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z" />
                        <path d="m9 12 2 2 4-4" />
                    </svg>
                    <h3
                        class="font-heading text-foreground text-xl font-semibold"
                    >
                        HTTPS Out of the Box
                    </h3>
                    <p class="text-muted font-mono text-[13px] leading-relaxed">Every linked project gets automatic HTTPS at project.test domains. No certificates to manage.</p>
                </div>
                <div class="bg-elevated space-y-4 rounded-2xl p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange">
                        <ellipse cx="12" cy="5" rx="9" ry="3" />
                        <path d="M3 5V19A9 3 0 0 0 21 19V5" />
                        <path d="M3 12A9 3 0 0 0 21 12" />
                    </svg>
                    <h3
                        class="font-heading text-foreground text-xl font-semibold"
                    >
                        Backing Services
                    </h3>
                    <p class="text-muted font-mono text-[13px] leading-relaxed">MySQL, PostgreSQL, Redis, Mailpit, MinIO — containerized and managed. One command to add.</p>
                </div>
            </div>
        </div>

        {{-- How It Works --}}
        <div class="mx-auto max-w-6xl space-y-10 pt-16" id="how_it_works">
            <div class="space-y-2">
                <span class="text-muted font-mono text-[13px]"
                    >// how_it_works</span
                >
                <h2 class="font-heading text-foreground text-4xl font-bold">
                    From zero to serving in two steps.
                </h2>
            </div>

            <div class="grid gap-6 md:grid-cols-3">
                @foreach ([
                    ['01', 'curl -fsSL .../install | bash', 'Downloads the pv binary to your machine. One line, no dependencies.'],
                    ['02', 'pv setup', 'Installs PHP, FrankenPHP, Composer, and Mago in one guided setup.'],
                    ['03', 'your environment is ready', 'After setup, link a project when needed and start building immediately.'],
                ] as [$num, $cmd, $desc])
                    <div class="bg-elevated space-y-4 rounded-2xl p-6">
                        <span
                            class="bg-accent-orange text-on-accent inline-block rounded-full px-3 py-1.5 font-mono text-xs font-semibold"
                            >{{ $num }}</span
                        >
                        <h3
                            class="text-foreground font-mono text-base font-semibold"
                        >
                            {{ $cmd }}
                        </h3>
                        <p class="text-muted font-mono text-[13px] leading-relaxed">{{ $desc }}</p>
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
                <span class="text-muted font-mono text-[13px]"
                    >// see_it_in_action</span
                >
                <h2
                    class="font-heading text-foreground text-center text-4xl font-bold"
                >
                    YOUR ENTIRE WORKFLOW. ONE TOOL.
                </h2>
                <p class="text-muted text-center font-mono text-[13px]">Install with curl, run pv setup, and your local PHP environment is ready.</p>
            </div>

            <div class="grid gap-6 md:grid-cols-2">
                <x-terminal
                    title="TERMINAL:~PHP_VERSIONS"
                    :dots-right="true"
                    class="text-[13px]"
                >
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> php:install </span
                        ><span class="text-muted">8.4</span>
                    </div>
                    <div class="text-muted">Downloading PHP 8.4.3...</div>
                    <div class="text-foreground">✓ PHP 8.4.3 installed</div>
                    <div class="h-2"></div>
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> php:use </span
                        ><span class="text-muted">8.4</span>
                    </div>
                    <div class="text-foreground">
                        ✓ Global PHP → <span class="text-accent">8.4</span>
                    </div>
                    <div class="h-2"></div>
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> php:list</span>
                    </div>
                    <div class="text-muted text-xs font-semibold">
                        VERSION STATUS PATH
                    </div>
                    <div class="text-placeholder text-xs">
                        ─────────────────────────────────────
                    </div>
                    <div class="text-muted text-xs">
                        8.3 installed ~/.pv/php/8.3
                    </div>
                    <div class="text-accent-orange text-xs">
                        * 8.4 active ~/.pv/php/8.4
                    </div>
                </x-terminal>

                <x-terminal
                    title="TERMINAL:~SERVICES"
                    :dots-right="true"
                    class="text-[13px]"
                >
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> service:add </span
                        ><span class="text-muted">mysql</span>
                    </div>
                    <div class="text-muted">Pulling mysql:8.0...</div>
                    <div class="text-foreground">
                        ✓ MySQL 8.0 on <span class="text-accent">:3306</span>
                    </div>
                    <div class="h-2"></div>
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> service:add </span
                        ><span class="text-muted">redis</span>
                    </div>
                    <div class="text-foreground">
                        ✓ Redis 7 on <span class="text-accent">:6379</span>
                    </div>
                    <div class="h-2"></div>
                    <div>
                        <span class="text-muted font-semibold">$ </span
                        ><span class="text-accent font-semibold">pv</span
                        ><span class="text-foreground"> service:list</span>
                    </div>
                    <div class="text-muted text-xs font-semibold">
                        SERVICE STATUS PORT
                    </div>
                    <div class="text-placeholder text-xs">
                        ─────────────────────────────────────
                    </div>
                    <div class="text-foreground text-xs">
                        mysql <span class="text-accent">running</span> 3306
                    </div>
                    <div class="text-foreground text-xs">
                        redis <span class="text-accent">running</span> 6379
                    </div>
                </x-terminal>
            </div>
        </div>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- SECTION: Final CTA                                                    --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <div
        class="bg-card flex flex-col items-center gap-8 px-6 py-20 md:px-20"
        id="get_started"
    >
        <span class="text-muted font-mono text-[13px]">// get_started</span>
        <h2 class="font-heading text-foreground text-center text-5xl font-bold">
            STOP CONFIGURING. START BUILDING.
        </h2>
        <p class="text-muted text-center font-mono text-sm">One curl. Then pv setup. Your PHP environment in under a minute.</p>

        <x-install-command />

        {{-- CTA buttons --}}
        <div class="flex items-center gap-4">
            <a
                href="https://pv.prvious.dev/docs"
                class="bg-accent-orange text-on-accent hover:bg-accent-orange/90 flex items-center gap-2 rounded-2xl px-8 py-3.5 font-mono text-sm font-semibold transition-colors"
            >
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="4 17 10 11 4 5" />
                    <line x1="12" x2="20" y1="19" y2="19" />
                </svg>
                Then: pv setup
            </a>
            <a
                href="https://github.com/prvious/pv"
                class="border-muted text-foreground hover:bg-elevated flex items-center gap-2 rounded-2xl border px-8 py-3.5 font-mono text-sm font-semibold transition-colors"
            >
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" /></svg>
                View on GitHub
            </a>
        </div>

        <p class="text-muted font-mono text-[11px]">Free and open source. Works on macOS. No sudo required.</p>
    </div>

    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    {{-- FOOTER                                                                --}}
    {{-- ═══════════════════════════════════════════════════════════════════════ --}}
    <x-site-footer />
</x-layouts.app>
