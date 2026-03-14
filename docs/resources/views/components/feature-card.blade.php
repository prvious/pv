@props([
    'icon' => null,
    'title' => null,
    'description' => null,
])

<x-card {{ $attributes }}>
    <div class="flex flex-col gap-3">
        @if ($icon)
            <div class="flex size-10 items-center justify-center rounded-lg bg-accent/10 text-accent">
                {{ $icon }}
            </div>
        @endif

        @if ($title)
            <x-heading :level="3" size="sm">{{ $title }}</x-heading>
        @endif

        @if ($description)
            <x-text variant="muted" size="sm">{{ $description }}</x-text>
        @endif

        @if ($slot->isNotEmpty())
            {{ $slot }}
        @endif
    </div>
</x-card>
