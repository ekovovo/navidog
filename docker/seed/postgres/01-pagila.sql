-- Pagila 示例库的精简子集，供开发与测试使用
CREATE DATABASE pagila;
\connect pagila

CREATE SCHEMA inventory;

CREATE TABLE public.actor (
  actor_id serial PRIMARY KEY,
  first_name text NOT NULL,
  last_name text NOT NULL,
  last_update timestamptz NOT NULL DEFAULT now()
);
COMMENT ON TABLE public.actor IS '演员';

CREATE TABLE public.film (
  film_id serial PRIMARY KEY,
  title text NOT NULL,
  description text,
  release_year int,
  rental_rate numeric(4,2) NOT NULL DEFAULT 4.99,
  tags text[]
);

CREATE TABLE inventory.store (
  store_id serial PRIMARY KEY,
  address jsonb
);

CREATE MATERIALIZED VIEW public.film_count AS SELECT count(*) AS n FROM public.film;

INSERT INTO public.actor (first_name, last_name) VALUES
  ('PENELOPE', 'GUINESS'), ('NICK', 'WAHLBERG'), ('ED', 'CHASE');
INSERT INTO public.film (title, description, release_year, tags) VALUES
  ('ACADEMY DINOSAUR', 'A Epic Drama', 2006, ARRAY['drama']),
  ('ACE GOLDFINGER', NULL, 2006, ARRAY['comedy', 'spy']);
INSERT INTO inventory.store (address) VALUES ('{"city": "Lethbridge"}');
