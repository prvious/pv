-- Representative sections for schemas not emitted by the container dump.
CREATE DATABASE IF NOT EXISTS `sys`;
USE `sys`;
CREATE TABLE should_skip (id integer);

CREATE DATABASE IF NOT EXISTS `performance_schema`;
USE `performance_schema`;
CREATE TABLE should_skip (id integer);

CREATE DATABASE IF NOT EXISTS `information_schema`;
USE `information_schema`;
CREATE TABLE should_skip (id integer);

CREATE DATABASE IF NOT EXISTS `admin`;
USE `admin`;
CREATE TABLE users (id integer PRIMARY KEY);
