@props([
    'orientation' => 'horizontal',
])

@php
    $orientationClasses = match ($orientation) {
        'vertical' => 'w-px h-full',
        default => 'h-px w-full',
    };
@endphp

<div {{ $attributes->cn('bg-elevated shrink-0', $orientationClasses) }}></div>
