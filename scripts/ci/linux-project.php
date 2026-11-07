<?php
$environment = parse_ini_file(__DIR__ . '/../.env', false, INI_SCANNER_RAW);
$mysql = new PDO(
    sprintf('mysql:host=%s;port=%s;dbname=%s', $environment['MYSQL_HOST'], $environment['MYSQL_PORT'], $environment['MYSQL_DATABASE']),
    $environment['MYSQL_USERNAME'], $environment['MYSQL_PASSWORD']
);
$postgres = new PDO(
    sprintf('pgsql:host=%s;port=%s;dbname=%s', $environment['POSTGRES_HOST'], $environment['POSTGRES_PORT'], $environment['POSTGRES_DATABASE']),
    $environment['POSTGRES_USERNAME'], $environment['POSTGRES_PASSWORD']
);
$redis = new Redis();
$redis->connect($environment['REDIS_HOST'], (int) $environment['REDIS_PORT']);
header('Content-Type: application/json');
echo json_encode([
    'php' => PHP_VERSION,
    'extensions' => get_loaded_extensions(),
    'mysql' => (int) $mysql->query('SELECT 1')->fetchColumn(),
    'postgres' => (int) $postgres->query('SELECT 1')->fetchColumn(),
    'redis' => (bool) $redis->ping(),
    'mailpit' => file_get_contents($environment['MAILPIT_DASHBOARD']) !== false,
    'rustfs' => file_get_contents($environment['S3_ENDPOINT'] . '/health') !== false,
]);
