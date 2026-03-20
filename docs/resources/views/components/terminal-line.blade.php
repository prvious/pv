@props ([
    'prompt' => '$',
    'type' => 'command',
])

@php
    $typeClasses = match ($type) {
        'comment' => 'text-muted',
        'output' => 'text-foreground/80',
        'success' => 'text-accent',
        'error' => 'text-accent-red',
        default => 'text-foreground',
    };
@endphp

<div {{ $attributes->cn('flex gap-2', $typeClasses) }}>
    @if ($type === 'command')
        <span class="text-muted shrink-0 select-none">{{ $prompt }}</span>
    @endif
    <span>{{ $slot }}</span>
</div>
