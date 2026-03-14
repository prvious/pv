@props([
    'title' => null,
])

@php
    $classes = [
        'rounded-2xl bg-elevated overflow-hidden font-mono text-sm',
        'shadow-[0_8px_32px_rgba(26,26,26,0.8)]',
    ];
@endphp

<div {{ $attributes->cn($classes) }}>
    {{-- Title bar --}}
    <div class="flex items-center gap-2 px-4 py-3 border-b border-placeholder/30">
        <div class="flex gap-1.5">
            <span class="size-3 rounded-full bg-accent-red/80"></span>
            <span class="size-3 rounded-full bg-accent-orange/80"></span>
            <span class="size-3 rounded-full bg-accent/80"></span>
        </div>
        @if ($title)
            <span class="text-xs text-muted ml-2">{{ $title }}</span>
        @endif
    </div>

    {{-- Terminal content --}}
    <div class="p-5 leading-relaxed">
        {{ $slot }}
    </div>
</div>
