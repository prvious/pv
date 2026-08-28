DELETE FROM resource_ports
WHERE (resource_name = 'caddy' AND track = '2' AND port_name = 'admin')
OR (resource_name = 'php' AND (port_name = 'admin' OR port_name LIKE 'admin.%'));

DELETE FROM ports
WHERE (owner_kind = 'gateway' AND owner_id = 'admin')
OR owner_kind = 'php_worker_admin';
