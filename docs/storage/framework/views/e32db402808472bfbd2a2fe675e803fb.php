<?php if (isset($component)) { $__componentOriginal5863877a5171c196453bfa0bd807e410 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal5863877a5171c196453bfa0bd807e410 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.layouts.app','data' => ['title' => 'pv — Local PHP development, zero config']] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('layouts.app'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes(['title' => 'pv — Local PHP development, zero config']); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

    
    
    
    <?php if (isset($component)) { $__componentOriginalfdc8967a87956c0a7185abbef03fae20 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginalfdc8967a87956c0a7185abbef03fae20 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.site-header','data' => []] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('site-header'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes([]); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

<?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginalfdc8967a87956c0a7185abbef03fae20)): ?>
<?php $attributes = $__attributesOriginalfdc8967a87956c0a7185abbef03fae20; ?>
<?php unset($__attributesOriginalfdc8967a87956c0a7185abbef03fae20); ?>
<?php endif; ?>
<?php if (isset($__componentOriginalfdc8967a87956c0a7185abbef03fae20)): ?>
<?php $component = $__componentOriginalfdc8967a87956c0a7185abbef03fae20; ?>
<?php unset($__componentOriginalfdc8967a87956c0a7185abbef03fae20); ?>
<?php endif; ?>

    <div class="relative bg-background">

        
        <div class="pointer-events-none absolute inset-x-0 top-16 h-[500px]" style="mask-image: linear-gradient(to bottom, black 40%, transparent)" aria-hidden="true">
            <div class="absolute left-1/2 top-0 h-[400px] w-[800px] -translate-x-1/2 rounded-full bg-white/20 blur-[120px]"></div>
            <div class="absolute left-1/3 top-0 h-[300px] w-[500px] -translate-x-1/2 rounded-full bg-white/10 blur-[100px]"></div>
        </div>

        
        <div class="flex flex-col items-center gap-8 px-6 pt-28 pb-16 md:px-20 md:pb-24">
            
            <h1 class="max-w-5xl text-center font-heading text-5xl font-bold leading-[1.1] text-foreground md:text-[60px]">
                One command. Full PHP environment.
            </h1>

            
            <p class="max-w-[700px] text-center font-mono text-[15px] leading-relaxed text-muted">
                Install pv with a single curl, then set up PHP, FrankenPHP, Composer and Mago — no Docker, no Nginx, no config files.
            </p>

            <?php if (isset($component)) { $__componentOriginal89031cc110f849d0d4d10fd7d2877c92 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal89031cc110f849d0d4d10fd7d2877c92 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.install-command','data' => []] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('install-command'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes([]); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

<?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal89031cc110f849d0d4d10fd7d2877c92)): ?>
<?php $attributes = $__attributesOriginal89031cc110f849d0d4d10fd7d2877c92; ?>
<?php unset($__attributesOriginal89031cc110f849d0d4d10fd7d2877c92); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal89031cc110f849d0d4d10fd7d2877c92)): ?>
<?php $component = $__componentOriginal89031cc110f849d0d4d10fd7d2877c92; ?>
<?php unset($__componentOriginal89031cc110f849d0d4d10fd7d2877c92); ?>
<?php endif; ?>

            
            <a href="https://pv.prvious.dev/docs" class="rounded-2xl border border-muted px-7 py-3.5 font-mono text-[13px] font-semibold text-foreground transition-colors hover:bg-elevated">
                Read Docs
            </a>

            <?php if (isset($component)) { $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.terminal','data' => ['title' => 'terminal','class' => 'w-full max-w-[780px] text-[12px] md:text-[13px]','xData' => '{ tab: \'php\' }']] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('terminal'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes(['title' => 'terminal','class' => 'w-full max-w-[780px] text-[12px] md:text-[13px]','x-data' => '{ tab: \'php\' }']); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

                
                <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">curl</span><span class="font-semibold text-muted"> -fsSL </span><span class="font-semibold text-foreground">https://pv.prvious.dev/install</span><span class="font-semibold text-muted"> | </span><span class="font-semibold text-accent-orange">bash</span></div>
                <div class="h-2"></div>

                
                <div class="text-accent">Setting up environment...</div>
                <div class="h-4"></div>

                
                <div class="flex items-center">
                    <button x-on:click="tab = 'php'" class="cursor-pointer px-3 transition-colors" :class="tab === 'php' ? 'font-bold text-foreground' : 'text-muted hover:text-foreground/70'">PHP Versions</button>
                    <span class="text-accent">│</span>
                    <button x-on:click="tab = 'tools'" class="cursor-pointer px-3 transition-colors" :class="tab === 'tools' ? 'font-bold text-foreground' : 'text-muted hover:text-foreground/70'">Tools</button>
                    <span class="text-accent">│</span>
                    <button x-on:click="tab = 'services'" class="cursor-pointer px-3 transition-colors" :class="tab === 'services' ? 'font-bold text-foreground' : 'text-muted hover:text-foreground/70'">Services</button>
                    <span class="text-accent">│</span>
                    <button x-on:click="tab = 'settings'" class="cursor-pointer px-3 transition-colors" :class="tab === 'settings' ? 'font-bold text-foreground' : 'text-muted hover:text-foreground/70'">Settings</button>
                </div>
                <div class="h-px w-full bg-accent"></div>
                <div class="h-4"></div>

                
                <div x-show="tab === 'php'">
                    <div class="text-muted">Select which PHP versions to install:</div>
                    <div class="h-3"></div>
                    <div><span class="font-bold text-accent">&gt; </span><span class="text-muted">○</span><span class="text-foreground"> PHP 8.3</span></div>
                    <div class="h-1"></div>
                    <div><span class="invisible">&gt; </span><span class="text-foreground">○ PHP 8.4</span></div>
                    <div class="h-1"></div>
                    <div><span class="invisible">&gt; </span><span class="text-accent">● PHP 8.5 </span><span class="text-muted">(installed)</span></div>
                </div>

                
                <div x-show="tab === 'tools'" x-cloak>
                    <div class="text-muted">Composer is always installed. Select additional tools:</div>
                    <div class="h-3"></div>
                    <div><span class="font-bold text-accent">&gt; </span><span class="text-accent">●</span><span class="text-accent"> Mago </span><span class="text-muted">(PHP linter &amp; formatter)</span></div>
                </div>

                
                <div x-show="tab === 'services'" x-cloak>
                    <div class="text-muted">Select backing services to set up:</div>
                    <div class="h-3"></div>
                    <div><span class="font-bold text-accent">&gt; </span><span class="text-accent">●</span><span class="text-accent"> MySQL</span></div>
                    <div class="h-1"></div>
                    <div><span class="invisible">&gt; </span><span class="text-accent">● PostgreSQL</span></div>
                    <div class="h-1"></div>
                    <div><span class="invisible">&gt; </span><span class="text-foreground">○ Redis</span></div>
                    <div class="h-1"></div>
                    <div><span class="invisible">&gt; </span><span class="text-foreground">○ Mail</span></div>
                    <div class="h-1"></div>
                    <div><span class="invisible">&gt; </span><span class="text-foreground">○ S3 Storage</span></div>
                </div>

                
                <div x-show="tab === 'settings'" x-cloak>
                    <div class="font-bold text-accent">Domain</div>
                    <div class="text-muted">Top-level domain for local sites</div>
                    <div class="h-2"></div>
                    <div><span class="font-bold text-accent">&gt; </span><span class="text-foreground">test</span> <span class="text-muted">(press e to edit)</span></div>
                    <div class="h-3"></div>
                    <div class="font-bold text-accent">Daemon</div>
                    <div class="text-muted">Start pv automatically on login</div>
                    <div class="h-2"></div>
                    <div><span class="invisible">&gt; </span><span class="text-accent">true</span></div>
                </div>

                <div class="h-4"></div>

                
                <div class="text-muted">←/→ tab • ↑/↓ move • space toggle • enter confirm • esc quit</div>
             <?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $attributes = $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $component = $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>
        </div>

        
        <div class="flex flex-col items-center justify-center gap-4 px-6 py-6 md:flex-row">
            <span class="font-mono text-[9px] text-muted">// built_with</span>
            <div class="flex flex-wrap items-center justify-center gap-2.5">
                <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if BLOCK]><![endif]--><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::openLoop(); ?><?php endif; ?><?php $__currentLoopData = ['Go', 'FrankenPHP', 'Caddy', 'Docker']; $__env->addLoop($__currentLoopData); foreach($__currentLoopData as $tech): $__env->incrementLoopIndices(); $loop = $__env->getLastLoop(); ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::startLoop($loop->index); ?><?php endif; ?>
                    <span class="rounded-full bg-card px-3 py-1.5 font-mono text-[11px] font-semibold text-foreground"><?php echo e($tech); ?></span>
                <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::endLoop(); ?><?php endif; ?><?php endforeach; $__env->popLoop(); $loop = $__env->getLastLoop(); ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if ENDBLOCK]><![endif]--><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::closeLoop(); ?><?php endif; ?>
            </div>
        </div>
    </div>

    
    
    
    <div class="flex flex-col items-center gap-8 bg-background px-6 py-16 md:px-20">
        
        <div class="flex flex-col items-center gap-2">
            <span class="font-mono text-[13px] text-muted">// link_a_project</span>
            <h2 class="font-heading text-5xl font-bold text-foreground">Link. Serve. Done.</h2>
        </div>

        <?php if (isset($component)) { $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.terminal','data' => ['title' => 'terminal','class' => 'w-full max-w-[720px] text-[13px] shadow-[0_8px_32px_rgba(0,0,0,0.8)]']] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('terminal'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes(['title' => 'terminal','class' => 'w-full max-w-[720px] text-[13px] shadow-[0_8px_32px_rgba(0,0,0,0.8)]']); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

            <div><span class="font-semibold text-muted">$ </span><span class="text-muted">cd ~/Code/myapp</span></div>
            <div class="h-2"></div>
            <div><span class="font-semibold text-muted">$ </span><span class="font-semibold text-accent">pv</span><span class="text-foreground"> link</span></div>
            <div class="h-2"></div>
            <div class="text-foreground">  ✓ myapp.caddy</div>
            <div class="text-foreground">  ✓ Caddyfile updated</div>
            <div class="text-foreground">  ✓ myapp.test</div>
            <div class="text-foreground">  ✓ no services detected</div>
            <div class="text-foreground">  ✓ https://myapp.test</div>
            <div class="h-2"></div>
            <div>  <span class="text-accent">✓</span> <span class="text-foreground">Linked</span> <span class="font-bold text-accent">https://myapp.test</span></div>
            <div class="h-2"></div>
            <div class="text-muted">  Path  ~/Code/myapp</div>
            <div class="text-muted">  Type  laravel</div>
            <div class="text-muted">  PHP   <span class="text-accent">8.5</span></div>
         <?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $attributes = $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $component = $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>

        <p class="font-mono text-[13px] text-muted">One command to link any PHP project. HTTPS, domain, and PHP version — automatic.</p>
    </div>

    
    
    
    <div class="space-y-16 px-6 py-20 md:px-20" id="features">

        
        <div class="mx-auto max-w-6xl space-y-10">
            <div class="space-y-2">
                <span class="font-mono text-[13px] text-muted">// core_features</span>
                <h2 class="font-heading text-4xl font-bold text-foreground">Everything you need. Nothing you don't.</h2>
            </div>

            <div class="grid gap-6 md:grid-cols-3">
                
                <div class="space-y-4 rounded-2xl bg-elevated p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange"><path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/></svg>
                    <h3 class="font-heading text-xl font-semibold text-foreground">PHP Version Manager</h3>
                    <p class="font-mono text-[13px] leading-relaxed text-muted">Install and switch between PHP versions instantly. Per-project version support via pv.yml or composer.json.</p>
                </div>
                
                <div class="space-y-4 rounded-2xl bg-elevated p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>
                    <h3 class="font-heading text-xl font-semibold text-foreground">HTTPS Out of the Box</h3>
                    <p class="font-mono text-[13px] leading-relaxed text-muted">Every linked project gets automatic HTTPS at project.test domains. No certificates to manage.</p>
                </div>
                
                <div class="space-y-4 rounded-2xl bg-elevated p-6">
                    <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-accent-orange"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/></svg>
                    <h3 class="font-heading text-xl font-semibold text-foreground">Backing Services</h3>
                    <p class="font-mono text-[13px] leading-relaxed text-muted">MySQL, PostgreSQL, Redis, Mailpit, MinIO — containerized and managed. One command to add.</p>
                </div>
            </div>
        </div>

        
        <div class="mx-auto max-w-6xl space-y-10" id="how_it_works">
            <div class="space-y-2">
                <span class="font-mono text-[13px] text-muted">// how_it_works</span>
                <h2 class="font-heading text-4xl font-bold text-foreground">From zero to serving in two steps.</h2>
            </div>

            <div class="grid gap-6 md:grid-cols-3">
                <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if BLOCK]><![endif]--><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::openLoop(); ?><?php endif; ?><?php $__currentLoopData = [
                    ['01', 'curl -fsSL .../install | bash', 'Downloads the pv binary to your machine. One line, no dependencies.'],
                    ['02', 'pv setup', 'Installs PHP, FrankenPHP, Composer, and Mago in one guided setup.'],
                    ['03', 'your environment is ready', 'After setup, link a project when needed and start building immediately.'],
                ]; $__env->addLoop($__currentLoopData); foreach($__currentLoopData as [$num, $cmd, $desc]): $__env->incrementLoopIndices(); $loop = $__env->getLastLoop(); ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::startLoop($loop->index); ?><?php endif; ?>
                    <div class="space-y-4 rounded-2xl bg-elevated p-6">
                        <span class="inline-block rounded-full bg-accent-orange px-3 py-1.5 font-mono text-xs font-semibold text-on-accent"><?php echo e($num); ?></span>
                        <h3 class="font-mono text-base font-semibold text-foreground"><?php echo e($cmd); ?></h3>
                        <p class="font-mono text-[13px] leading-relaxed text-muted"><?php echo e($desc); ?></p>
                    </div>
                <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::endLoop(); ?><?php endif; ?><?php endforeach; $__env->popLoop(); $loop = $__env->getLastLoop(); ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if ENDBLOCK]><![endif]--><?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::closeLoop(); ?><?php endif; ?>
            </div>
        </div>
    </div>

    
    
    
    <div class="bg-background px-6 py-20 md:px-20">
        <div class="mx-auto max-w-6xl space-y-10">
            <div class="flex flex-col items-center gap-3">
                <span class="font-mono text-[13px] text-muted">// see_it_in_action</span>
                <h2 class="text-center font-heading text-4xl font-bold text-foreground">YOUR ENTIRE WORKFLOW. ONE TOOL.</h2>
                <p class="text-center font-mono text-[13px] text-muted">Install with curl, run pv setup, and your local PHP environment is ready.</p>
            </div>

            <div class="grid gap-6 md:grid-cols-2">
                <?php if (isset($component)) { $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.terminal','data' => ['title' => 'TERMINAL:~PHP_VERSIONS','dotsRight' => true,'class' => 'text-[13px]']] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('terminal'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes(['title' => 'TERMINAL:~PHP_VERSIONS','dots-right' => true,'class' => 'text-[13px]']); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

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
                 <?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $attributes = $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $component = $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>

                <?php if (isset($component)) { $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.terminal','data' => ['title' => 'TERMINAL:~SERVICES','dotsRight' => true,'class' => 'text-[13px]']] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('terminal'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes(['title' => 'TERMINAL:~SERVICES','dots-right' => true,'class' => 'text-[13px]']); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

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
                 <?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $attributes = $__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__attributesOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0)): ?>
<?php $component = $__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0; ?>
<?php unset($__componentOriginal2b87b9bf8d9e15ba9982d7ee0ea9f7f0); ?>
<?php endif; ?>
            </div>
        </div>
    </div>

    
    
    
    <div class="flex flex-col items-center gap-8 bg-card px-6 py-20 md:px-20" id="get_started">
        <span class="font-mono text-[13px] text-muted">// get_started</span>
        <h2 class="text-center font-heading text-5xl font-bold text-foreground">STOP CONFIGURING. START BUILDING.</h2>
        <p class="text-center font-mono text-sm text-muted">One curl. Then pv setup. Your PHP environment in under a minute.</p>

        <?php if (isset($component)) { $__componentOriginal89031cc110f849d0d4d10fd7d2877c92 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal89031cc110f849d0d4d10fd7d2877c92 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.install-command','data' => []] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('install-command'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes([]); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

<?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal89031cc110f849d0d4d10fd7d2877c92)): ?>
<?php $attributes = $__attributesOriginal89031cc110f849d0d4d10fd7d2877c92; ?>
<?php unset($__attributesOriginal89031cc110f849d0d4d10fd7d2877c92); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal89031cc110f849d0d4d10fd7d2877c92)): ?>
<?php $component = $__componentOriginal89031cc110f849d0d4d10fd7d2877c92; ?>
<?php unset($__componentOriginal89031cc110f849d0d4d10fd7d2877c92); ?>
<?php endif; ?>

        
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

    
    
    
    <?php if (isset($component)) { $__componentOriginal222c87a019257fb1d70ae0ff46ab02e1 = $component; } ?>
<?php if (isset($attributes)) { $__attributesOriginal222c87a019257fb1d70ae0ff46ab02e1 = $attributes; } ?>
<?php $component = Illuminate\View\AnonymousComponent::resolve(['view' => 'components.site-footer','data' => []] + (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag ? $attributes->all() : [])); ?>
<?php $component->withName('site-footer'); ?>
<?php if ($component->shouldRender()): ?>
<?php $__env->startComponent($component->resolveView(), $component->data()); ?>
<?php if (isset($attributes) && $attributes instanceof Illuminate\View\ComponentAttributeBag): ?>
<?php $attributes = $attributes->except(\Illuminate\View\AnonymousComponent::ignoredParameterNames()); ?>
<?php endif; ?>
<?php $component->withAttributes([]); ?>
<?php \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::processComponentKey($component); ?>

<?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal222c87a019257fb1d70ae0ff46ab02e1)): ?>
<?php $attributes = $__attributesOriginal222c87a019257fb1d70ae0ff46ab02e1; ?>
<?php unset($__attributesOriginal222c87a019257fb1d70ae0ff46ab02e1); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal222c87a019257fb1d70ae0ff46ab02e1)): ?>
<?php $component = $__componentOriginal222c87a019257fb1d70ae0ff46ab02e1; ?>
<?php unset($__componentOriginal222c87a019257fb1d70ae0ff46ab02e1); ?>
<?php endif; ?>
 <?php echo $__env->renderComponent(); ?>
<?php endif; ?>
<?php if (isset($__attributesOriginal5863877a5171c196453bfa0bd807e410)): ?>
<?php $attributes = $__attributesOriginal5863877a5171c196453bfa0bd807e410; ?>
<?php unset($__attributesOriginal5863877a5171c196453bfa0bd807e410); ?>
<?php endif; ?>
<?php if (isset($__componentOriginal5863877a5171c196453bfa0bd807e410)): ?>
<?php $component = $__componentOriginal5863877a5171c196453bfa0bd807e410; ?>
<?php unset($__componentOriginal5863877a5171c196453bfa0bd807e410); ?>
<?php endif; ?>
<?php /**PATH /Users/clovismuneza/Apps/pv/docs/resources/views/home.blade.php ENDPATH**/ ?>