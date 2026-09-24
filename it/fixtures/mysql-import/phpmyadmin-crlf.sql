-- phpMyAdmin SQL Dump
-- Database: `admin`
SET SQL_MODE = "NO_AUTO_VALUE_ON_ZERO";
START TRANSACTION;
SET time_zone = "+00:00";
/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;
/*!40101 SET NAMES utf8mb4 */;

CREATE TABLE `admin`.`users` (
  `id` bigint NOT NULL,
  `name` varchar(255) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
INSERT INTO `admin`.`users` (`id`, `name`) VALUES
(1, 'André; analytics'),
(2, 'backslash \\ and a semicolon ;');
ALTER TABLE `admin`.`users` ADD PRIMARY KEY (`id`);
COMMIT;
