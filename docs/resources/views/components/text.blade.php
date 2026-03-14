@props([
    'variant' => 'default',
    'size' => 'default',
    'as' => 'p',
])

@php
    $variantClasses = match ($variant) {
        'muted' => 'text-muted',
        'accent' => 'text-accent',
        default => 'text-foreground',
    };

    $sizeClasses = match ($size) {
        'xs' => 'text-xs',
        'sm' => 'text-sm',
        'lg' => 'text-lg',
        'xl' => 'text-xl',
        default => 'text-base',
    };
@endphp

<{{ $as }} {{ $attributes->cn($variantClasses, $sizeClasses) }}>
    {{ $slot }}
</{{ $as }}>
