<?php $attributes ??= new \Illuminate\View\ComponentAttributeBag;

$__newAttributes = [];
$__propNames = \Illuminate\View\ComponentAttributeBag::extractPropNames(([
    'title' => null,
    'dotsRight' => false,
]));

foreach ($attributes->all() as $__key => $__value) {
    if (in_array($__key, $__propNames)) {
        $$__key = $$__key ?? $__value;
    } else {
        $__newAttributes[$__key] = $__value;
    }
}

$attributes = new \Illuminate\View\ComponentAttributeBag($__newAttributes);

unset($__propNames);
unset($__newAttributes);

foreach (array_filter(([
    'title' => null,
    'dotsRight' => false,
]), 'is_string', ARRAY_FILTER_USE_KEY) as $__key => $__value) {
    $$__key = $$__key ?? $__value;
}

$__defined_vars = get_defined_vars();

foreach ($attributes->all() as $__key => $__value) {
    if (array_key_exists($__key, $__defined_vars)) unset($$__key);
}

unset($__defined_vars, $__key, $__value); ?>

<div
    <?php echo e($attributes->cn('overflow-hidden rounded-2xl border border-placeholder bg-elevated font-mono')); ?>

>
    
    <div class="bg-card flex items-center justify-between px-4 py-3">
        <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if BLOCK]><![endif]--><?php endif; ?><?php if($dotsRight): ?>
            <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if BLOCK]><![endif]--><?php endif; ?><?php if($title): ?>
                <span
                    class="text-muted text-xs font-semibold"
                    ><?php echo e($title); ?></span
                >
            <?php endif; ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if ENDBLOCK]><![endif]--><?php endif; ?>
            <div class="flex gap-1.5">
                <span class="bg-muted size-2.5 rounded-full"></span>
                <span class="bg-accent size-2.5 rounded-full"></span>
                <span class="bg-accent-orange size-2.5 rounded-full"></span>
            </div>
        <?php else: ?>
            <div class="flex items-center gap-2">
                <span class="bg-accent-red size-3 rounded-full"></span>
                <span class="bg-accent-orange size-3 rounded-full"></span>
                <span class="bg-accent size-3 rounded-full"></span>
                <?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if BLOCK]><![endif]--><?php endif; ?><?php if($title): ?>
                    <span
                        class="text-muted ml-1 text-[11px]"
                        ><?php echo e($title); ?></span
                    >
                <?php endif; ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if ENDBLOCK]><![endif]--><?php endif; ?>
            </div>
        <?php endif; ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if ENDBLOCK]><![endif]--><?php endif; ?>
    </div>

    
    <div class="space-y-1.5 p-5 leading-relaxed"><?php echo e($slot); ?></div>
</div>
<?php /**PATH /Users/clovismuneza/Apps/pv/docs/resources/views/components/terminal.blade.php ENDPATH**/ ?>