@props ([
    'variant' => 'default',
    'size' => 'default',
    'as' => 'button',
])

@php
    $variantClasses = match ($variant) {
        'accent' => 'bg-accent text-on-accent hover:bg-accent/90',
        'outline' => 'border border-elevated bg-transparent text-foreground hover:bg-elevated',
        'ghost' => 'bg-transparent text-foreground hover:bg-elevated',
        'muted' => 'bg-card text-muted hover:bg-elevated hover:text-foreground',
        default => 'bg-foreground text-background hover:bg-foreground/90',
    };

    $sizeClasses = match ($size) {
        'sm' => 'h-8 px-3 text-xs gap-1.5',
        'lg' => 'h-12 px-6 text-base gap-2.5',
        'icon' => 'size-9',
        'icon-sm' => 'size-8',
        default => 'h-10 px-5 text-sm gap-2',
    };

    $classes = [
        'inline-flex items-center justify-center shrink-0 whitespace-nowrap rounded-lg font-medium',
        'transition-colors duration-150 outline-none focus-visible:ring-2 focus-visible:ring-accent/50',
        'disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0',
    ];
@endphp

<{{ $as }} {{ $attributes->cn($classes, $variantClasses, $sizeClasses) }}>
    {{ $slot }}
</{{ $as }}>
