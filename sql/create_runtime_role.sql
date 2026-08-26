-- Create the dedicated Neon runtime role for the Aprendiendo MCP service.
--
-- Run ONCE with a direct (non-pooled) owner connection:
--
--   psql "$DIRECT_DATABASE_URL" \
--     -v runtime_password='<generated-secret>' \
--     -f sql/create_runtime_role.sql
--
-- The role receives ONLY function execution on the mcp_api boundary. It has
-- no access to the public tables, Neon administration, or schema discovery.
-- The SECURITY DEFINER functions are the intended data-access boundary.

\set ON_ERROR_STOP on

DO $do$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'aprendiendo_mcp_runtime') THEN
        RAISE NOTICE 'role aprendiendo_mcp_runtime already exists; updating password';
        EXECUTE format('ALTER ROLE aprendiendo_mcp_runtime WITH LOGIN PASSWORD %L', :'runtime_password');
    ELSE
        EXECUTE format('CREATE ROLE aprendiendo_mcp_runtime WITH LOGIN PASSWORD %L', :'runtime_password');
    END IF;
END
$do$;

GRANT CONNECT ON DATABASE spanish_learning TO aprendiendo_mcp_runtime;
GRANT USAGE ON SCHEMA mcp_api TO aprendiendo_mcp_runtime;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mcp_api TO aprendiendo_mcp_runtime;

-- Belt-and-suspenders: keep the runtime role out of public and out of the
-- mcp_api idempotency table. (PostgreSQL 15+ no longer grants public-schema
-- USAGE to PUBLIC, but make the boundary explicit.)
REVOKE ALL ON SCHEMA public FROM aprendiendo_mcp_runtime;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM aprendiendo_mcp_runtime;
REVOKE ALL ON ALL TABLES IN SCHEMA mcp_api FROM aprendiendo_mcp_runtime;

-- Verify, as owner, in a follow-up session:
--
--   SET ROLE aprendiendo_mcp_runtime;
--   SELECT mcp_api.get_data_status();        -- should succeed
--   SELECT mcp_api.get_learning_context(1);  -- should succeed
--   SELECT * FROM public.sessions;           -- should FAIL (permission denied)
--   RESET ROLE;
