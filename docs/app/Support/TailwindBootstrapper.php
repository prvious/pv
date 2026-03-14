<?php

namespace App\Support;

use Illuminate\View\ComponentAttributeBag;
use TalesFromADev\TailwindMerge\TailwindMergeInterface;

class TailwindBootstrapper
{
    public function __construct(
        private TailwindMergeInterface $tailwindMerge
    ) {}

    public function bootMacro(): void
    {
        ComponentAttributeBag::macro('cn', function (...$args): ComponentAttributeBag {
            /** @var ComponentAttributeBag $this */
            $this->offsetSet('class', app(TailwindMergeInterface::class)->merge($args, $this->get('class', '')));

            return $this;
        });
    }
}
