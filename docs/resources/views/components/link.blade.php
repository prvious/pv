@props([
    'variant' => 'default',
])

@php
    $variantClasses = match ($variant) {
        'muted' => 'text-muted hover:text-foreground',
        'accent' => 'text-accent hover:text-accent/80',
        default => 'text-foreground hover:text-accent',
    };
@endphp

<a {{ $attributes->cn('transition-colors duration-150', $variantClasses) }}>
    {{ $slot }}
</a>
