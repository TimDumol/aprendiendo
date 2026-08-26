-- Read-only preflight. It succeeds only when the six adapter functions exist
-- with the exact signatures expected by the MCP server.
SELECT jsonb_build_object(
    'schema_version', 1,
    'ready', bool_and(function_oid IS NOT NULL),
    'functions', jsonb_object_agg(signature, function_oid IS NOT NULL)
) AS mcp_contract
FROM (
    VALUES
        ('mcp_api.get_learning_context(integer)', to_regprocedure('mcp_api.get_learning_context(integer)')),
        ('mcp_api.get_recent_practice(integer,text)', to_regprocedure('mcp_api.get_recent_practice(integer,text)')),
        ('mcp_api.record_practice_session(jsonb)', to_regprocedure('mcp_api.record_practice_session(jsonb)')),
        ('mcp_api.get_review_queue(integer,text)', to_regprocedure('mcp_api.get_review_queue(integer,text)')),
        ('mcp_api.upsert_weakness(jsonb)', to_regprocedure('mcp_api.upsert_weakness(jsonb)')),
        ('mcp_api.get_data_status()', to_regprocedure('mcp_api.get_data_status()'))
) AS expected(signature, function_oid);
