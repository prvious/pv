@props ([
    'number' => null,
    'title' => null,
    'description' => null,
])

<div {{ $attributes->cn('flex gap-4') }}>
    @if ($number)
        <div
            class="bg-accent text-on-accent flex size-8 shrink-0 items-center justify-center rounded-full text-sm font-bold"
        >
            {{ $number }}
        </div>
    @endif

    <div class="flex flex-col gap-1 pt-0.5">
        @if ($title)
            <x-heading :level="3" size="xs">{{ $title }}</x-heading>
        @endif

        @if ($description)
            <x-text variant="muted" size="sm">{{ $description }}</x-text>
        @endif

        @if ($slot->isNotEmpty())
            <div class="mt-2">{{ $slot }}</div>
        @endif
    </div>
</div>
