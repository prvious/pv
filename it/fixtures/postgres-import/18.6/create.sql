--
-- PostgreSQL database dump
--

\restrict pvfixture

-- Dumped from database version 18.6
-- Dumped by pg_dump version 18.6

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: admin; Type: DATABASE; Schema: -; Owner: production_owner
--

CREATE DATABASE admin WITH TEMPLATE = template0 ENCODING = 'UTF8' LOCALE_PROVIDER = libc LOCALE = 'en_US.utf8';


ALTER DATABASE admin OWNER TO production_owner;

\unrestrict pvfixture
\connect admin
\restrict pvfixture

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: admin; Type: DATABASE PROPERTIES; Schema: -; Owner: production_owner
--

ALTER DATABASE admin SET work_mem TO '8MB';


\unrestrict pvfixture
\connect admin
\restrict pvfixture

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: pgcrypto; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;


--
-- Name: EXTENSION pgcrypto; Type: COMMENT; Schema: -; Owner: 
--

COMMENT ON EXTENSION pgcrypto IS 'cryptographic functions';


--
-- Name: label(); Type: FUNCTION; Schema: public; Owner: postgres
--

CREATE FUNCTION public.label() RETURNS text
    LANGUAGE plpgsql SECURITY DEFINER
    AS $$
BEGIN
  RETURN 'admin; analytics; \\connect postgres';
END
$$;


ALTER FUNCTION public.label() OWNER TO postgres;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: proof; Type: TABLE; Schema: public; Owner: production_owner
--

CREATE TABLE public.proof (
    id integer NOT NULL,
    note text
);


ALTER TABLE public.proof OWNER TO production_owner;

SET default_tablespace = production_space;

--
-- Name: space_proof; Type: TABLE; Schema: public; Owner: postgres; Tablespace: production_space
--

CREATE TABLE public.space_proof (
    id integer
);


ALTER TABLE public.space_proof OWNER TO postgres;

--
-- Data for Name: proof; Type: TABLE DATA; Schema: public; Owner: production_owner
--

COPY public.proof (id, note) FROM stdin;
1	admin and analytics are data, not routing
2	backslash \\ and snowman ☃
\.


--
-- Data for Name: space_proof; Type: TABLE DATA; Schema: public; Owner: postgres
--

COPY public.space_proof (id) FROM stdin;
\.


--
-- Name: 16440; Type: BLOB METADATA; Schema: -; Owner: postgres
--

SELECT pg_catalog.lo_create('16440');

ALTER LARGE OBJECT 16440 OWNER TO postgres;

--
-- Data for Name: 16440; Type: BLOBS; Schema: -; Owner: postgres
--

BEGIN;

SELECT pg_catalog.lo_open('16440', 131072);
SELECT pg_catalog.lowrite(0, '\x6c61726765206f626a6563742064617461');
SELECT pg_catalog.lo_close(0);

COMMIT;

SET default_tablespace = '';

--
-- Name: proof proof_pkey; Type: CONSTRAINT; Schema: public; Owner: production_owner
--

ALTER TABLE ONLY public.proof
    ADD CONSTRAINT proof_pkey PRIMARY KEY (id);


--
-- Name: proof_publication; Type: PUBLICATION; Schema: -; Owner: postgres
--

CREATE PUBLICATION proof_publication WITH (publish = 'insert, update, delete, truncate');


ALTER PUBLICATION proof_publication OWNER TO postgres;

--
-- Name: proof_publication proof; Type: PUBLICATION TABLE; Schema: public; Owner: postgres
--

ALTER PUBLICATION proof_publication ADD TABLE ONLY public.proof;


--
-- Name: TABLE proof; Type: ACL; Schema: public; Owner: production_owner
--

GRANT SELECT ON TABLE public.proof TO production_reader;


--
-- PostgreSQL database dump complete
--

\unrestrict pvfixture

