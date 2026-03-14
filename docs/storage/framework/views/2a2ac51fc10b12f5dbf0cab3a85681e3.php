<!DOCTYPE html>
<html lang="<?php echo e(str_replace('_', '-', app()->getLocale())); ?>">
    <head>
        <?php echo $__env->make('partials.head', array_diff_key(get_defined_vars(), ['__data' => 1, '__path' => 1]))->render(); ?>
    </head>
    <body class="min-h-screen">
        <?php echo e($slot); ?>

    </body>
</html>
<?php /**PATH /Users/clovismuneza/Apps/pv/docs/resources/views/components/layouts/app.blade.php ENDPATH**/ ?>