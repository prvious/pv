CREATE ROLE production_owner LOGIN;
CREATE ROLE production_reader LOGIN;
GRANT production_reader TO production_owner;
CREATE TABLESPACE production_space LOCATION '/tmp/pvfixture-tablespace';
CREATE DATABASE admin OWNER production_owner;
CREATE DATABASE analytics OWNER production_owner;
CREATE DATABASE "Mixed-Name" OWNER production_owner;
ALTER DATABASE admin SET work_mem = '8MB';
\connect admin
CREATE EXTENSION pgcrypto;
CREATE TABLE public.proof (id integer PRIMARY KEY, note text);
CREATE TABLE public.space_proof (id integer) TABLESPACE production_space;
SELECT lo_from_bytea(0, convert_to('large object data', 'UTF8'));
INSERT INTO public.proof VALUES
  (1, 'admin and analytics are data, not routing'),
  (2, E'backslash \\ and snowman ☃');
CREATE FUNCTION public.label() RETURNS text LANGUAGE plpgsql SECURITY DEFINER AS $$
BEGIN
  RETURN 'admin; analytics; \\connect postgres';
END
$$;
ALTER TABLE public.proof OWNER TO production_owner;
GRANT SELECT ON public.proof TO production_reader;
CREATE PUBLICATION proof_publication FOR TABLE public.proof;
\connect analytics
CREATE TABLE public.metric (value integer);
INSERT INTO public.metric VALUES (42);
\connect "Mixed-Name"
CREATE TABLE public.quoted_database (note text);
INSERT INTO public.quoted_database VALUES ('requires an explicit allocation mapping');
\connect postgres
CREATE TABLE public.maintenance_note (note text);
INSERT INTO public.maintenance_note VALUES ('preserve this explicitly');
CREATE FUNCTION public.event_probe() RETURNS event_trigger LANGUAGE plpgsql AS $$
BEGIN
  RETURN;
END
$$;
CREATE EVENT TRIGGER event_probe ON ddl_command_start EXECUTE FUNCTION public.event_probe();
