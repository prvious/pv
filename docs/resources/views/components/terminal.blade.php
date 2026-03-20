@props ([
    'title' => null,
    'dotsRight' => false,
])

<div
    {{ $attributes->cn('overflow-hidden rounded-2xl border border-placeholder bg-elevated font-mono') }}
>
    {{-- Title bar --}}
    <div class="bg-card flex items-center justify-between px-4 py-3">
        @if ($dotsRight)
            @if ($title)
                <span
                    class="text-muted text-xs font-semibold"
                    >{{ $title }}</span
                >
            @endif
            <div class="flex gap-1.5">
                <span class="bg-muted size-2.5 rounded-full"></span>
                <span class="bg-accent size-2.5 rounded-full"></span>
                <span class="bg-accent-orange size-2.5 rounded-full"></span>
            </div>
        @else
            <div class="flex items-center gap-2">
                <span class="bg-accent-red size-3 rounded-full"></span>
                <span class="bg-accent-orange size-3 rounded-full"></span>
                <span class="bg-accent size-3 rounded-full"></span>
                @if ($title)
                    <span
                        class="text-muted ml-1 text-[11px]"
                        >{{ $title }}</span
                    >
                @endif
            </div>
        @endif
    </div>

    {{-- Body --}}
    <div class="space-y-1.5 p-5 leading-relaxed">{{ $slot }}</div>
</div>
