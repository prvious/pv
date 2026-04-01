@props ([
    'variant' => 'default',
])

@php
    $variantClasses = match ($variant) {
        'elevated' => 'bg-elevated border-placeholder/30',
        'outline' => 'bg-transparent border-elevated',
        default => 'bg-card border-elevated',
    };

    $classes = [
        'rounded-2xl border p-6',
    ];
@endphp

<div {{ $attributes->cn($classes, $variantClasses) }}> {{ $slot }}</div>
