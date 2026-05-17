@php
    $classes = [
        'sticky top-0 z-50 w-full',
        'border-b border-elevated/50 bg-background/80 backdrop-blur-lg',
    ];
@endphp

<nav {{ $attributes->cn($classes) }}>
    <div
        class="mx-auto flex h-16 max-w-6xl items-center justify-between px-6 md:px-20 lg:px-30"
    >
        {{ $slot }}
    </div>
</nav>
