CREATE DATABASE admin;
CREATE DATABASE analytics;
CREATE DATABASE `Mixed-Name`;

CREATE TABLE admin.users (
  id bigint NOT NULL PRIMARY KEY,
  name varchar(255) NOT NULL,
  payload blob
);
INSERT INTO admin.users VALUES
  (1, 'André; `analytics`', X'005CFF'),
  (2, 'a\\b', X'00FF');

CREATE TABLE analytics.audit (
  id bigint NOT NULL PRIMARY KEY AUTO_INCREMENT,
  message varchar(255) NOT NULL
);
CREATE VIEW analytics.admin_names AS SELECT name FROM admin.users;

DELIMITER $$
CREATE TRIGGER admin.users_after_insert AFTER INSERT ON admin.users
FOR EACH ROW BEGIN
  INSERT INTO analytics.audit(message) VALUES (NEW.name);
END$$
CREATE PROCEDURE admin.write_audit(IN entry varchar(255))
BEGIN
  INSERT INTO analytics.audit(message) VALUES (entry);
END$$
CREATE FUNCTION admin.user_total() RETURNS integer DETERMINISTIC READS SQL DATA
BEGIN
  DECLARE user_count integer;
  SELECT COUNT(*) INTO user_count FROM admin.users;
  RETURN user_count;
END$$
CREATE PROCEDURE admin.dynamic_probe(IN table_name varchar(255))
BEGIN
  SET @sql_text = CONCAT('SELECT count(*) FROM ', table_name);
  PREPARE statement FROM @sql_text;
  EXECUTE statement;
  DEALLOCATE PREPARE statement;
END$$
DELIMITER ;

CREATE EVENT analytics.daily_audit ON SCHEDULE EVERY 1 DAY STARTS '2020-01-01 00:00:00' DISABLE
DO INSERT INTO analytics.audit(message) VALUES ('event');

CREATE TABLE `Mixed-Name`.notes (id integer PRIMARY KEY, body text);
INSERT INTO `Mixed-Name`.notes VALUES (1, 'admin.analytics');
CREATE TABLE `Mixed-Name`.`café` (id integer PRIMARY KEY, body text);
INSERT INTO `Mixed-Name`.`café` VALUES (1, 'été');
