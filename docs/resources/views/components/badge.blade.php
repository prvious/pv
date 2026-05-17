@props ([
    'variant' => 'default',
    'as' => 'span',
])

@php
    $variantClasses = match ($variant) {
        'accent' => 'bg-accent/15 text-accent border-accent/25',
        'orange' => 'bg-accent-orange/15 text-accent-orange border-accent-orange/25',
        'muted' => 'bg-elevated text-muted border-elevated',
        'outline' => 'bg-transparent text-muted border-elevated',
        default => 'bg-foreground/10 text-foreground border-foreground/20',
    };

    $defaultClasses = 'inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-medium whitespace-nowrap';
@endphp

<{{ $as }} {{ $attributes->cn($defaultClasses, $variantClasses) }}>
    {{ $slot }}
</{{ $as }}>
