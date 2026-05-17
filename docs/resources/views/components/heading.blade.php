@props ([
    'level' => 2,
    'size' => 'default',
])

@php
    $tag = 'h' . $level;

    $sizeClasses = match ($size) {
        'xs' => 'text-sm font-semibold',
        'sm' => 'text-lg font-semibold',
        'lg' => 'text-3xl font-bold tracking-tight md:text-4xl',
        'xl' => 'text-4xl font-extrabold tracking-tight md:text-5xl lg:text-6xl',
        'hero' => 'text-5xl font-black tracking-tight md:text-6xl lg:text-7xl',
        default => 'text-2xl font-bold tracking-tight',
    };
@endphp

<{{ $tag }} {{ $attributes->cn($sizeClasses, 'text-foreground') }}>
    {{ $slot }}
</{{ $tag }}>
