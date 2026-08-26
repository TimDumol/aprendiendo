-- Adapter for the inspected spanish_learning database in Neon.
-- Base tables remain in public: sessions, attempts, observations, weaknesses.
-- Review this migration, replace the runtime role placeholder, and apply it with
-- a direct (non-pooled) owner connection.

BEGIN;

CREATE SCHEMA IF NOT EXISTS mcp_api;

CREATE TABLE IF NOT EXISTS mcp_api.recorded_requests (
    idempotency_key text PRIMARY KEY,
    session_id bigint NOT NULL UNIQUE REFERENCES public.sessions(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION mcp_api.get_learning_context(p_recent_sessions integer)
RETURNS jsonb
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, mcp_api
AS $$
    SELECT jsonb_build_object(
        'active_weaknesses', COALESCE((
            SELECT jsonb_agg(to_jsonb(x) ORDER BY x.incorrect_count DESC, x.key)
            FROM (
                SELECT
                    w.key,
                    w.category,
                    w.description,
                    w.target_pattern,
                    w.first_seen,
                    w.last_seen,
                    count(o.id) FILTER (WHERE o.outcome = 'incorrect') AS incorrect_count,
                    count(o.id) FILTER (WHERE o.outcome = 'correct') AS correct_count,
                    count(o.id) AS observation_count
                FROM public.weaknesses w
                LEFT JOIN public.observations o ON o.weakness_id = w.id
                WHERE w.active
                GROUP BY w.id
                ORDER BY incorrect_count DESC, w.key
                LIMIT 12
            ) x
        ), '[]'::jsonb),
        'recent_sessions', COALESCE((
            SELECT jsonb_agg(to_jsonb(x) ORDER BY x.session_date DESC, x.id DESC)
            FROM (
                SELECT
                    s.id,
                    s.session_date,
                    s.exercise_type,
                    s.topic,
                    s.notes,
                    count(DISTINCT a.id) AS attempt_count,
                    count(DISTINCT o.id) AS observation_count
                FROM public.sessions s
                LEFT JOIN public.attempts a ON a.session_id = s.id
                LEFT JOIN public.observations o ON o.session_id = s.id
                GROUP BY s.id
                ORDER BY s.session_date DESC, s.id DESC
                LIMIT LEAST(GREATEST(p_recent_sessions, 1), 20)
            ) x
        ), '[]'::jsonb)
    );
$$;

CREATE OR REPLACE FUNCTION mcp_api.get_recent_practice(p_limit integer, p_skill text)
RETURNS jsonb
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, mcp_api
AS $$
    SELECT jsonb_build_object(
        'items', COALESCE(jsonb_agg(to_jsonb(x) ORDER BY x.session_date DESC, x.id DESC), '[]'::jsonb)
    )
    FROM (
        SELECT
            s.id,
            s.session_date,
            s.exercise_type,
            s.topic,
            s.notes,
            COALESCE((
                SELECT jsonb_agg(
                    jsonb_build_object(
                        'attempt_no', a.attempt_no,
                        'transcript', left(a.transcript, 8000),
                        'created_at', a.created_at,
                        'observations', COALESCE((
                            SELECT jsonb_agg(jsonb_build_object(
                                'weakness_key', w.key,
                                'category', w.category,
                                'outcome', o.outcome,
                                'produced', o.produced,
                                'correction', o.correction,
                                'notes', o.notes
                            ) ORDER BY o.id)
                            FROM public.observations o
                            JOIN public.weaknesses w ON w.id = o.weakness_id
                            WHERE o.attempt_id = a.id
                        ), '[]'::jsonb)
                    ) ORDER BY a.attempt_no
                )
                FROM public.attempts a
                WHERE a.session_id = s.id
            ), '[]'::jsonb) AS attempts,
            COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'weakness_key', w.key,
                    'category', w.category,
                    'outcome', o.outcome,
                    'produced', o.produced,
                    'correction', o.correction,
                    'notes', o.notes
                ) ORDER BY o.id)
                FROM public.observations o
                JOIN public.weaknesses w ON w.id = o.weakness_id
                WHERE o.session_id = s.id AND o.attempt_id IS NULL
            ), '[]'::jsonb) AS session_observations
        FROM public.sessions s
        WHERE p_skill IS NULL
           OR s.exercise_type ILIKE '%' || p_skill || '%'
           OR COALESCE(s.topic, '') ILIKE '%' || p_skill || '%'
           OR EXISTS (
                SELECT 1
                FROM public.observations o
                JOIN public.weaknesses w ON w.id = o.weakness_id
                WHERE o.session_id = s.id
                  AND (w.key = p_skill OR w.category = p_skill)
           )
        ORDER BY s.session_date DESC, s.id DESC
        LIMIT LEAST(GREATEST(p_limit, 1), 50)
    ) x;
$$;

CREATE OR REPLACE FUNCTION mcp_api.record_practice_session(p_payload jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, mcp_api
AS $$
DECLARE
    v_session_id bigint;
    v_session_date date;
    v_attempt jsonb;
    v_attempt_id bigint;
    v_observation jsonb;
    v_missing_keys text[];
    v_attempt_count integer := 0;
    v_observation_count integer := 0;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_payload->>'idempotency_key', 0)
    );

    SELECT r.session_id INTO v_session_id
    FROM mcp_api.recorded_requests r
    WHERE r.idempotency_key = p_payload->>'idempotency_key';

    IF v_session_id IS NOT NULL THEN
        RETURN jsonb_build_object(
            'session_id', v_session_id,
            'recorded', false,
            'idempotent_replay', true
        );
    END IF;

    SELECT array_agg(DISTINCT requested.key ORDER BY requested.key)
    INTO v_missing_keys
    FROM (
        SELECT observation->>'weakness_key' AS key
        FROM jsonb_array_elements(COALESCE(p_payload->'observations', '[]'::jsonb)) observation
        UNION
        SELECT observation->>'weakness_key'
        FROM jsonb_array_elements(COALESCE(p_payload->'attempts', '[]'::jsonb)) attempt
        CROSS JOIN LATERAL jsonb_array_elements(COALESCE(attempt->'observations', '[]'::jsonb)) observation
    ) requested
    LEFT JOIN public.weaknesses w ON w.key = requested.key
    WHERE w.id IS NULL;

    IF v_missing_keys IS NOT NULL THEN
        RAISE EXCEPTION 'unknown weakness keys: %', array_to_string(v_missing_keys, ', ')
            USING ERRCODE = '23503';
    END IF;

    v_session_date := COALESCE(NULLIF(p_payload->>'session_date', '')::date, current_date);
    INSERT INTO public.sessions (session_date, exercise_type, topic, notes)
    VALUES (
        v_session_date,
        p_payload->>'exercise_type',
        p_payload->>'topic',
        p_payload->>'notes'
    )
    RETURNING id INTO v_session_id;

    FOR v_attempt IN
        SELECT value FROM jsonb_array_elements(COALESCE(p_payload->'attempts', '[]'::jsonb))
    LOOP
        INSERT INTO public.attempts (session_id, attempt_no, transcript)
        VALUES (
            v_session_id,
            (v_attempt->>'attempt_no')::integer,
            v_attempt->>'transcript'
        )
        RETURNING id INTO v_attempt_id;
        v_attempt_count := v_attempt_count + 1;

        FOR v_observation IN
            SELECT value FROM jsonb_array_elements(COALESCE(v_attempt->'observations', '[]'::jsonb))
        LOOP
            INSERT INTO public.observations (
                session_id, attempt_id, weakness_id, outcome, produced, correction, notes
            )
            SELECT
                v_session_id,
                v_attempt_id,
                w.id,
                v_observation->>'outcome',
                v_observation->>'produced',
                v_observation->>'correction',
                v_observation->>'notes'
            FROM public.weaknesses w
            WHERE w.key = v_observation->>'weakness_key';
            v_observation_count := v_observation_count + 1;
        END LOOP;
    END LOOP;

    FOR v_observation IN
        SELECT value FROM jsonb_array_elements(COALESCE(p_payload->'observations', '[]'::jsonb))
    LOOP
        INSERT INTO public.observations (
            session_id, attempt_id, weakness_id, outcome, produced, correction, notes
        )
        SELECT
            v_session_id,
            NULL,
            w.id,
            v_observation->>'outcome',
            v_observation->>'produced',
            v_observation->>'correction',
            v_observation->>'notes'
        FROM public.weaknesses w
        WHERE w.key = v_observation->>'weakness_key';
        v_observation_count := v_observation_count + 1;
    END LOOP;

    UPDATE public.weaknesses w
    SET first_seen = LEAST(COALESCE(w.first_seen, v_session_date), v_session_date),
        last_seen = GREATEST(COALESCE(w.last_seen, v_session_date), v_session_date)
    WHERE w.key IN (
        SELECT observation->>'weakness_key'
        FROM jsonb_array_elements(COALESCE(p_payload->'observations', '[]'::jsonb)) observation
        UNION
        SELECT observation->>'weakness_key'
        FROM jsonb_array_elements(COALESCE(p_payload->'attempts', '[]'::jsonb)) attempt
        CROSS JOIN LATERAL jsonb_array_elements(COALESCE(attempt->'observations', '[]'::jsonb)) observation
    );

    INSERT INTO mcp_api.recorded_requests (idempotency_key, session_id)
    VALUES (p_payload->>'idempotency_key', v_session_id);

    RETURN jsonb_build_object(
        'session_id', v_session_id,
        'recorded', true,
        'idempotent_replay', false,
        'attempt_count', v_attempt_count,
        'observation_count', v_observation_count
    );
END;
$$;

CREATE OR REPLACE FUNCTION mcp_api.get_review_queue(p_limit integer, p_category text)
RETURNS jsonb
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, mcp_api
AS $$
    SELECT jsonb_build_object(
        'basis', 'active weaknesses ordered by incorrect count and error rate; this schema has no due dates',
        'items', COALESCE(jsonb_agg(to_jsonb(x) ORDER BY x.incorrect_count DESC, x.error_rate DESC, x.key), '[]'::jsonb)
    )
    FROM (
        SELECT
            w.key,
            w.category,
            w.description,
            w.target_pattern,
            w.first_seen,
            w.last_seen,
            count(o.id) FILTER (WHERE o.outcome = 'incorrect') AS incorrect_count,
            count(o.id) FILTER (WHERE o.outcome = 'correct') AS correct_count,
            count(o.id) AS observation_count,
            CASE WHEN count(o.id) = 0 THEN 0
                 ELSE round(
                    count(o.id) FILTER (WHERE o.outcome = 'incorrect')::numeric / count(o.id),
                    3
                 )
            END AS error_rate
        FROM public.weaknesses w
        LEFT JOIN public.observations o ON o.weakness_id = w.id
        WHERE w.active AND (p_category IS NULL OR w.category = p_category)
        GROUP BY w.id
        ORDER BY incorrect_count DESC, error_rate DESC, w.key
        LIMIT LEAST(GREATEST(p_limit, 1), 50)
    ) x;
$$;

CREATE OR REPLACE FUNCTION mcp_api.upsert_weakness(p_payload jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, mcp_api
AS $$
DECLARE
    v_id bigint;
    v_created boolean;
BEGIN
    SELECT NOT EXISTS (
        SELECT 1 FROM public.weaknesses WHERE key = p_payload->>'key'
    ) INTO v_created;

    INSERT INTO public.weaknesses (key, category, description, target_pattern, active)
    VALUES (
        p_payload->>'key',
        p_payload->>'category',
        p_payload->>'description',
        p_payload->>'target_pattern',
        COALESCE((p_payload->>'active')::boolean, true)
    )
    ON CONFLICT (key) DO UPDATE SET
        category = EXCLUDED.category,
        description = EXCLUDED.description,
        target_pattern = EXCLUDED.target_pattern,
        active = CASE
            WHEN p_payload ? 'active' AND p_payload->'active' <> 'null'::jsonb
            THEN (p_payload->>'active')::boolean
            ELSE weaknesses.active
        END
    RETURNING id INTO v_id;

    RETURN jsonb_build_object(
        'weakness_id', v_id,
        'key', p_payload->>'key',
        'created', v_created,
        'updated', NOT v_created
    );
END;
$$;

CREATE OR REPLACE FUNCTION mcp_api.get_data_status()
RETURNS jsonb
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public, mcp_api
AS $$
    SELECT jsonb_build_object(
        'schema_version', 1,
        'storage_model', 'single_learner',
        'last_session_date', (SELECT max(session_date) FROM public.sessions),
        'counts', jsonb_build_object(
            'sessions', (SELECT count(*) FROM public.sessions),
            'attempts', (SELECT count(*) FROM public.attempts),
            'observations', (SELECT count(*) FROM public.observations),
            'weaknesses', (SELECT count(*) FROM public.weaknesses),
            'active_weaknesses', (SELECT count(*) FROM public.weaknesses WHERE active)
        )
    );
$$;

REVOKE ALL ON SCHEMA mcp_api FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA mcp_api FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA mcp_api FROM PUBLIC;

-- Replace mcp_runtime with the role embedded in DATABASE_URL:
-- GRANT USAGE ON SCHEMA mcp_api TO mcp_runtime;
-- GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mcp_api TO mcp_runtime;

COMMIT;
