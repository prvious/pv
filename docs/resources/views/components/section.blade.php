@props ([
    'variant' => 'default',
])

@php
    $variantClasses = match ($variant) {
        'card' => 'bg-card',
        'elevated' => 'bg-elevated',
        default => 'bg-background',
    };
@endphp

<section {{ $attributes->cn($variantClasses, 'py-20 px-6 md:px-20 lg:px-30') }}>
    <div class="mx-auto max-w-6xl">{{ $slot }}</div>
</section>
