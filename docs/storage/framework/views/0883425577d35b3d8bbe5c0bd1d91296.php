<?php
/** @var \Laravel\Boost\Install\GuidelineAssist $assist */
?>
# Laravel Pint Code Formatter

<?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if BLOCK]><![endif]--><?php endif; ?><?php if($assist->supportsPintAgentFormatter()): ?>
- If you have modified any PHP files, you must run ___SINGLE_BACKTICK___<?php echo e($assist->binCommand('pint')); ?> --dirty --format agent___SINGLE_BACKTICK___ before finalizing changes to ensure your code matches the project's expected style.
- Do not run ___SINGLE_BACKTICK___<?php echo e($assist->binCommand('pint')); ?> --test --format agent___SINGLE_BACKTICK___, simply run ___SINGLE_BACKTICK___<?php echo e($assist->binCommand('pint')); ?> --format agent___SINGLE_BACKTICK___ to fix any formatting issues.
<?php else: ?>
- If you have modified any PHP files, you must run ___SINGLE_BACKTICK___<?php echo e($assist->binCommand('pint')); ?> --dirty___SINGLE_BACKTICK___ before finalizing changes to ensure your code matches the project's expected style.
- Do not run ___SINGLE_BACKTICK___<?php echo e($assist->binCommand('pint')); ?> --test___SINGLE_BACKTICK___, simply run ___SINGLE_BACKTICK___<?php echo e($assist->binCommand('pint')); ?>___SINGLE_BACKTICK___ to fix any formatting issues.
<?php endif; ?><?php if(\Livewire\Mechanisms\ExtendBlade\ExtendBlade::isRenderingLivewireComponent()): ?><!--[if ENDBLOCK]><![endif]--><?php endif; ?>
<?php /**PATH /Users/clovismuneza/Apps/pv/docs/storage/framework/views/2b3b75ec9892892981b355f1d066492b.blade.php ENDPATH**/ ?>