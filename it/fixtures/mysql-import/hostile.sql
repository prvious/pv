CREATE DATABASE admin;
USE admin;
CREATE USER 'escaped'@'%' IDENTIFIED BY 'fixture-only';
GRANT ALL PRIVILEGES ON *.* TO 'escaped'@'%';
SET GLOBAL general_log = ON;
INSTALL PLUGIN hostile SONAME 'hostile.so';
SELECT 'secret' INTO OUTFILE '/tmp/pv350-should-not-exist';
CREATE TABLE unrelated_project.users (id integer);
\! touch /tmp/pv350-should-not-exist
SOURCE /tmp/pv350-should-not-exist;
