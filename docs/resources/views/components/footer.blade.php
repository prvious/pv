@php
    $classes = [
        'border-t border-elevated bg-elevated',
    ];
@endphp

<footer {{ $attributes->cn($classes, 'py-12 px-6 md:px-20 lg:px-30') }}>
    <div class="mx-auto max-w-6xl">{{ $slot }}</div>
</footer>
