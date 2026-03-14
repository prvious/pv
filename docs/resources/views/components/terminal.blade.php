@props([
    'title' => null,
    'dotsRight' => false,
])

<div {{ $attributes->cn('overflow-hidden rounded-2xl border border-placeholder bg-elevated font-mono') }}>
    {{-- Title bar --}}
    <div class="flex items-center justify-between bg-card px-4 py-3">
        @if ($dotsRight)
            @if ($title)
                <span class="text-xs font-semibold text-muted">{{ $title }}</span>
            @endif
            <div class="flex gap-1.5">
                <span class="size-2.5 rounded-full bg-muted"></span>
                <span class="size-2.5 rounded-full bg-accent"></span>
                <span class="size-2.5 rounded-full bg-accent-orange"></span>
            </div>
        @else
            <div class="flex items-center gap-2">
                <span class="size-3 rounded-full bg-accent-red"></span>
                <span class="size-3 rounded-full bg-accent-orange"></span>
                <span class="size-3 rounded-full bg-accent"></span>
                @if ($title)
                    <span class="ml-1 text-[11px] text-muted">{{ $title }}</span>
                @endif
            </div>
        @endif
    </div>

    {{-- Body --}}
    <div class="space-y-1.5 p-5 leading-relaxed">
        {{ $slot }}
    </div>
</div>
