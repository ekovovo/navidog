-- Sakila 示例库的精简子集，供开发与测试使用
-- 初始化脚本默认以 latin1 客户端字符集执行，不声明会把中文注释二次编码
SET NAMES utf8mb4;
CREATE DATABASE IF NOT EXISTS sakila DEFAULT CHARACTER SET utf8mb4;
USE sakila;

CREATE TABLE actor (
  actor_id SMALLINT UNSIGNED NOT NULL AUTO_INCREMENT,
  first_name VARCHAR(45) NOT NULL,
  last_name VARCHAR(45) NOT NULL,
  last_update TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  PRIMARY KEY (actor_id),
  KEY idx_actor_last_name (last_name)
) ENGINE=InnoDB COMMENT='演员';

CREATE TABLE film (
  film_id SMALLINT UNSIGNED NOT NULL AUTO_INCREMENT,
  title VARCHAR(128) NOT NULL,
  description TEXT,
  release_year YEAR,
  rental_rate DECIMAL(4,2) NOT NULL DEFAULT 4.99,
  rating ENUM('G','PG','PG-13','R','NC-17') DEFAULT 'G',
  PRIMARY KEY (film_id)
) ENGINE=InnoDB;

CREATE TABLE film_actor (
  actor_id SMALLINT UNSIGNED NOT NULL,
  film_id SMALLINT UNSIGNED NOT NULL,
  PRIMARY KEY (actor_id, film_id),
  CONSTRAINT fk_film_actor_actor FOREIGN KEY (actor_id) REFERENCES actor (actor_id),
  CONSTRAINT fk_film_actor_film FOREIGN KEY (film_id) REFERENCES film (film_id)
) ENGINE=InnoDB;

CREATE VIEW actor_info AS
  SELECT a.actor_id, a.first_name, a.last_name, COUNT(fa.film_id) AS films
  FROM actor a LEFT JOIN film_actor fa ON fa.actor_id = a.actor_id
  GROUP BY a.actor_id, a.first_name, a.last_name;

INSERT INTO actor (first_name, last_name) VALUES
  ('PENELOPE', 'GUINESS'), ('NICK', 'WAHLBERG'), ('ED', 'CHASE'), ('JENNIFER', 'DAVIS');
INSERT INTO film (title, description, release_year, rating) VALUES
  ('ACADEMY DINOSAUR', 'A Epic Drama of a Feminist', 2006, 'PG'),
  ('ACE GOLDFINGER', 'A Astounding Epistle', 2006, 'G'),
  ('ADAPTATION HOLES', NULL, 2006, 'NC-17');
INSERT INTO film_actor VALUES (1, 1), (1, 2), (2, 3), (3, 1);

CREATE DATABASE IF NOT EXISTS empty_db;
