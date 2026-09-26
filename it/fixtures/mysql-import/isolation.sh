#!/bin/sh
set -eu

for version in 8.0.46 8.4.9 9.7.0; do
  container="pv350-isolation-$version-$$"
  docker run --rm -d --name "$container" -e MYSQL_ROOT_PASSWORD=fixture-only "mysql:$version" --skip-log-bin >/dev/null
  trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT INT TERM
  attempt=0
  until docker exec "$container" mysql -uroot -pfixture-only -N -e 'SELECT 1' >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 60 ]; then exit 1; fi
    sleep 1
  done

  docker exec -i "$container" mysql -uroot -pfixture-only <<'SQL'
CREATE DATABASE project_admin;
CREATE DATABASE unrelated_project;
CREATE USER 'pv_import'@'%' IDENTIFIED BY 'fixture-only';
GRANT ALL PRIVILEGES ON project_admin.* TO 'pv_import'@'%';
SQL

  if docker exec "$container" mysql -h127.0.0.1 -upv_import -pfixture-only -e 'CREATE TABLE unrelated_project.escape (id integer);'; then
    echo "restricted import user mutated unrelated_project on MySQL $version" >&2
    exit 1
  fi
  if docker exec "$container" mysql -h127.0.0.1 -upv_import -pfixture-only -e 'CREATE DATABASE escaped_project;'; then
    echo "restricted import user created a database on MySQL $version" >&2
    exit 1
  fi
  if docker exec "$container" mysql -h127.0.0.1 -upv_import -pfixture-only -e 'SET GLOBAL general_log = ON;'; then
    echo "restricted import user changed global configuration on MySQL $version" >&2
    exit 1
  fi
  if docker exec "$container" mysql -h127.0.0.1 -upv_import -pfixture-only project_admin -e "CREATE DEFINER='root'@'localhost' VIEW escaped_definer AS SELECT 1;"; then
    echo "restricted import user created an object with the root definer on MySQL $version" >&2
    exit 1
  fi

  docker exec -i "$container" mysql -h127.0.0.1 -upv_import -pfixture-only project_admin <<'SQL'
CREATE TABLE users (id integer PRIMARY KEY, name varchar(50));
CREATE TABLE audit (name varchar(50));
CREATE DEFINER='pv_import'@'%' VIEW names AS SELECT name FROM users;
DELIMITER $$
CREATE DEFINER='pv_import'@'%' TRIGGER users_after_insert AFTER INSERT ON users FOR EACH ROW
BEGIN
  INSERT INTO audit VALUES (NEW.name);
END$$
CREATE DEFINER='pv_import'@'%' PROCEDURE add_user(IN name_value varchar(50))
BEGIN
  INSERT INTO users VALUES (2, name_value);
END$$
CREATE DEFINER='pv_import'@'%' FUNCTION user_total() RETURNS integer DETERMINISTIC READS SQL DATA
BEGIN
  DECLARE total integer;
  SELECT COUNT(*) INTO total FROM users;
  RETURN total;
END$$
DELIMITER ;
INSERT INTO users VALUES (1, 'first');
SQL

  docker exec "$container" mysql -uroot -pfixture-only -e "ALTER USER 'pv_import'@'%' ACCOUNT LOCK;"
  docker exec "$container" mysql -uroot -pfixture-only project_admin -e "CALL add_user('second');"
  result=$(docker exec "$container" mysql -uroot -pfixture-only -N -s project_admin -e 'SELECT GROUP_CONCAT(name ORDER BY name) FROM names UNION ALL SELECT GROUP_CONCAT(name ORDER BY name) FROM audit;')
  if [ "$result" != 'first,second
first,second' ]; then
    echo "definer-backed objects stopped working after account lock on MySQL $version" >&2
    exit 1
  fi
  function_result=$(docker exec "$container" mysql -uroot -pfixture-only -N -s project_admin -e 'SELECT user_total();')
  if [ "$function_result" != '2' ]; then
    echo "definer-backed function stopped working after account lock on MySQL $version" >&2
    exit 1
  fi
  if docker exec "$container" mysql -h127.0.0.1 -upv_import -pfixture-only project_admin -e 'SELECT 1'; then
    echo "import user could still log in after lock on MySQL $version" >&2
    exit 1
  fi
  docker rm -f "$container" >/dev/null
  trap - EXIT INT TERM
done
