import * as SQLite from "expo-sqlite";
import { File as ExpoFile, Paths } from "expo-file-system";

import type { TaskSnapshot } from "@/lib/domain/tasks";
import { hashBlobSha256 } from "@/lib/media/recording";
import {
  DEFAULT_FREE_SPACE_RESERVE_BYTES,
  quotaBytes,
  selectEvictions,
  type QuotaRecording,
} from "@/lib/storage/quota";
import {
  DEFAULT_PRACTICE_SETTINGS,
  type PracticeSettings,
  type StoredAnalysis,
  type StoredPracticeRound,
  type StoredPracticeRun,
} from "@/lib/types";

const DATABASE_NAME = "aprendiendo-practice.db";
const WEB_STATE_KEY = "aprendiendo.practice.storage.v1";
const isNative = process.env.EXPO_OS !== "web";

export type StoredRecording = {
  id: string;
  roundId: string;
  relativePath: string | null;
  mimeType: string;
  container: string;
  codec: string;
  bytes: number;
  hash: string;
  decodedDurationMs: number | null;
  clientDurationMs: number;
  createdOrder: number;
  interrupted: boolean;
  mediaAvailability: "ready" | "evicted" | "missing";
  warning: string | null;
  feedbackJson: string | null;
  model: string | null;
  usageJson: string | null;
};

export type StorageUsage = {
  audioBytes: number;
  reservedBytes: number;
  recordingCount: number;
};

export type PracticeOutboxEntry = {
  operationId: string;
  recordingId: string;
  payloadHash: string;
  requestJson: string;
  uploadId: string | null;
  jobId: string | null;
  retryState: "queued" | "uploading" | "processing" | "retry" | "completed" | "failed";
  nextAttemptAt: string | null;
  lastSanitizedError: string | null;
};

type WebState = {
  version: 1;
  runs: StoredPracticeRun[];
  rounds: StoredPracticeRound[];
  recordings: StoredRecording[];
  analyses: StoredAnalysis[];
  outbox: Array<Record<string, unknown>>;
  mediaOperations: Array<Record<string, unknown>>;
  settings: PracticeSettings;
  sequence: number;
};

const emptyWebState = (): WebState => ({
  version: 1,
  runs: [],
  rounds: [],
  recordings: [],
  analyses: [],
  outbox: [],
  mediaOperations: [],
  settings: DEFAULT_PRACTICE_SETTINGS,
  sequence: 0,
});

let databasePromise: Promise<SQLite.SQLiteDatabase> | null = null;
let initializationPromise: Promise<void> | null = null;
let operationQueue: Promise<unknown> = Promise.resolve();

function now(): string {
  return new Date().toISOString();
}

function nextId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function webStorage(): Storage | null {
  return typeof globalThis.localStorage === "undefined" ? null : globalThis.localStorage;
}

function readWebState(): WebState {
  const storage = webStorage();
  if (!storage) return emptyWebState();
  try {
    const raw = storage.getItem(WEB_STATE_KEY);
    if (!raw) return emptyWebState();
    const parsed = JSON.parse(raw) as Partial<WebState>;
    return {
      ...emptyWebState(),
      ...parsed,
      settings: { ...DEFAULT_PRACTICE_SETTINGS, ...(parsed.settings ?? {}) },
    };
  } catch {
    return emptyWebState();
  }
}

function writeWebState(state: WebState): void {
  webStorage()?.setItem(WEB_STATE_KEY, JSON.stringify(state));
}

async function database(): Promise<SQLite.SQLiteDatabase> {
  if (!databasePromise) databasePromise = SQLite.openDatabaseAsync(DATABASE_NAME);
  return databasePromise;
}

const SCHEMA = `
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS practice_schema_migrations (
  version INTEGER PRIMARY KEY NOT NULL,
  applied_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS practice_runs (
  id TEXT PRIMARY KEY NOT NULL,
  snapshot_json TEXT NOT NULL,
  mode TEXT NOT NULL,
  created_order INTEGER NOT NULL,
  status TEXT NOT NULL,
  assistance TEXT,
  feedback_exposure TEXT,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS practice_rounds (
  id TEXT PRIMARY KEY NOT NULL,
  run_id TEXT NOT NULL REFERENCES practice_runs(id) ON DELETE CASCADE,
  sequence INTEGER NOT NULL,
  target_duration_ms INTEGER NOT NULL,
  classification TEXT NOT NULL,
  status TEXT NOT NULL,
  interrupted INTEGER NOT NULL DEFAULT 0,
  created_order INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS recordings (
  id TEXT PRIMARY KEY NOT NULL,
  round_id TEXT NOT NULL REFERENCES practice_rounds(id) ON DELETE CASCADE,
  relative_path TEXT,
  mime_type TEXT NOT NULL,
  container TEXT NOT NULL,
  codec TEXT NOT NULL,
  bytes INTEGER NOT NULL,
  hash TEXT NOT NULL,
  decoded_duration_ms INTEGER,
  client_duration_ms INTEGER NOT NULL,
  created_order INTEGER NOT NULL,
  interrupted INTEGER NOT NULL DEFAULT 0,
  media_availability TEXT NOT NULL,
  warning TEXT,
  feedback_json TEXT,
  model TEXT,
  usage_json TEXT
);
CREATE TABLE IF NOT EXISTS analyses (
  id TEXT PRIMARY KEY NOT NULL,
  recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
  recording_hash TEXT NOT NULL,
  processor_version TEXT NOT NULL,
  model_version TEXT NOT NULL,
  config_json TEXT NOT NULL,
  metrics_json TEXT NOT NULL,
  status TEXT NOT NULL,
  limitations_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
  operation_id TEXT PRIMARY KEY NOT NULL,
  recording_id TEXT,
  payload_hash TEXT NOT NULL,
  request_json TEXT NOT NULL DEFAULT '{}',
  upload_id TEXT,
  job_id TEXT,
  retry_state TEXT NOT NULL,
  next_attempt_at TEXT,
  last_sanitized_error TEXT
);
CREATE TABLE IF NOT EXISTS media_operations (
  operation_id TEXT PRIMARY KEY NOT NULL,
  recording_id TEXT,
  operation TEXT NOT NULL,
  relative_path TEXT,
  reserved_bytes INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS practice_settings (
  key TEXT PRIMARY KEY NOT NULL,
  value_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS recordings_created_order_idx ON recordings(created_order);
CREATE INDEX IF NOT EXISTS rounds_run_sequence_idx ON practice_rounds(run_id, sequence);
CREATE INDEX IF NOT EXISTS analyses_recording_idx ON analyses(recording_id, created_at);
`;

async function initializeNative(): Promise<void> {
  const db = await database();
  await db.execAsync(SCHEMA);
  try {
    await db.execAsync("ALTER TABLE outbox ADD COLUMN request_json TEXT NOT NULL DEFAULT '{}'");
  } catch {
    // Fresh databases already include the column; existing development databases
    // can safely continue after the duplicate-column error.
  }
  await db.runAsync(
    "INSERT OR IGNORE INTO practice_schema_migrations (version, applied_at) VALUES (?, ?)",
    1,
    now(),
  );
  await db.runAsync(
    "INSERT OR IGNORE INTO practice_schema_migrations (version, applied_at) VALUES (?, ?)",
    2,
    now(),
  );
  const existing = await db.getFirstAsync<{ value_json: string }>(
    "SELECT value_json FROM practice_settings WHERE key = ?",
    "settings",
  );
  if (!existing) {
    await db.runAsync(
      "INSERT INTO practice_settings (key, value_json) VALUES (?, ?)",
      "settings",
      JSON.stringify(DEFAULT_PRACTICE_SETTINGS),
    );
  }
  await reconcileNativeFiles(db);
}

function nativeFileForRelativePath(relativePath: string): ExpoFile {
  return new ExpoFile(Paths.document, ...relativePath.split("/").filter(Boolean));
}

async function reconcileNativeFiles(db: SQLite.SQLiteDatabase): Promise<void> {
  const recordings = await db.getAllAsync<{
    id: string;
    relativePath: string | null;
    mediaAvailability: string;
  }>(
    "SELECT id, relative_path AS relativePath, media_availability AS mediaAvailability FROM recordings WHERE media_availability = 'ready'",
  );
  for (const recording of recordings) {
    if (!recording.relativePath) {
      await db.runAsync("UPDATE recordings SET media_availability = 'missing' WHERE id = ?", recording.id);
      continue;
    }
    try {
      if (!nativeFileForRelativePath(recording.relativePath).info().exists) {
        await db.runAsync("UPDATE recordings SET media_availability = 'missing' WHERE id = ?", recording.id);
      }
    } catch {
      await db.runAsync("UPDATE recordings SET media_availability = 'missing' WHERE id = ?", recording.id);
    }
  }
  const pendingArtifacts = await db.getAllAsync<{
    operationId: string;
    roundId: string;
    relativePath: string;
  }>(
    "SELECT operation_id AS operationId, recording_id AS roundId, relative_path AS relativePath FROM media_operations WHERE operation = 'reserve' AND state = 'pending' AND relative_path IS NOT NULL",
  );
  for (const operation of pendingArtifacts) {
    const file = nativeFileForRelativePath(operation.relativePath);
    try {
      const info = file.info();
      if (!info.exists || !info.size || operation.roundId === "unsaved-round") {
        await db.runAsync("UPDATE media_operations SET state = 'failed' WHERE operation_id = ?", operation.operationId);
        continue;
      }
      const filename = operation.relativePath.split("/").filter(Boolean).at(-1) ?? "recovered.m4a";
      const recordingId = filename.replace(/\.(m4a|ogg|webm)$/i, "");
      const alreadyStored = await db.getFirstAsync<{ id: string }>("SELECT id FROM recordings WHERE id = ?", recordingId);
      if (!alreadyStored) {
        const mimeType = filename.endsWith(".m4a") ? "audio/m4a" : filename.endsWith(".ogg") ? "audio/ogg" : "audio/webm";
        const hash = await hashBlobSha256(new Blob([await file.arrayBuffer()], { type: mimeType }));
        await db.runAsync(
          `INSERT INTO recordings
            (id, round_id, relative_path, mime_type, container, codec, bytes, hash,
             decoded_duration_ms, client_duration_ms, created_order, interrupted,
             media_availability, warning, feedback_json, model, usage_json)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, 0, ?, 1, 'ready', ?, NULL, NULL, NULL)`,
          recordingId,
          operation.roundId,
          operation.relativePath,
          mimeType,
          mimeType === "audio/m4a" ? "m4a/mp4" : mimeType === "audio/ogg" ? "ogg" : "webm",
          mimeType === "audio/m4a" ? "aac-lc-or-device-reported" : "opus-or-browser-reported",
          info.size,
          hash,
          Date.now(),
          "Recovered after an interrupted finalization; the original duration was unavailable.",
        );
        await db.runAsync(
          "UPDATE practice_rounds SET status = 'interrupted', interrupted = 1 WHERE id = ? AND status IN ('recording', 'finalizing')",
          operation.roundId,
        );
      }
      await db.runAsync("UPDATE media_operations SET state = 'committed' WHERE operation_id = ?", operation.operationId);
    } catch {
      // Keep the candidate file for a later recovery attempt; never mark it as
      // a playable saved recording when metadata could not be reconstructed.
      await db.runAsync("UPDATE media_operations SET state = 'failed' WHERE operation_id = ?", operation.operationId);
    }
  }
  // A process cannot still be recording after a cold launch. Release reservations
  // left by a killed capture after the candidate recovery pass above.
  await db.runAsync(
    "UPDATE media_operations SET state = 'failed' WHERE operation = 'reserve' AND state = 'pending' AND relative_path IS NULL",
  );
}

export async function initializeStorage(): Promise<void> {
  if (!initializationPromise) {
    initializationPromise = isNative ? initializeNative() : Promise.resolve();
  }
  await initializationPromise;
}

async function serialized<T>(operation: () => Promise<T>): Promise<T> {
  const next = operationQueue.then(operation, operation);
  operationQueue = next.then(
    () => undefined,
    () => undefined,
  );
  return next;
}

export async function createRun(snapshot: TaskSnapshot): Promise<string> {
  await initializeStorage();
  return serialized(async () => {
    const id = nextId("run");
    if (!isNative) {
      const state = readWebState();
      state.sequence += 1;
      state.runs.unshift({
        id,
        snapshot: JSON.stringify(snapshot),
        mode: snapshot.mode,
        createdOrder: state.sequence,
        status: "ready",
        assistance: null,
        feedbackExposure: null,
        updatedAt: now(),
      });
      writeWebState(state);
      return id;
    }
    const db = await database();
    const createdOrder = Date.now();
    await db.runAsync(
      `INSERT INTO practice_runs
        (id, snapshot_json, mode, created_order, status, assistance, feedback_exposure, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      id,
      JSON.stringify(snapshot),
      snapshot.mode,
      createdOrder,
      "ready",
      null,
      null,
      now(),
    );
    return id;
  });
}

export async function updateRunStatus(
  runId: string,
  status: StoredPracticeRun["status"],
  assistance?: string | null,
  feedbackExposure?: string | null,
): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const run = state.runs.find((candidate) => candidate.id === runId);
      if (run) {
        run.status = status;
        run.updatedAt = now();
        if (assistance !== undefined) run.assistance = assistance;
        if (feedbackExposure !== undefined) run.feedbackExposure = feedbackExposure;
      }
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      `UPDATE practice_runs
       SET status = ?, assistance = COALESCE(?, assistance), feedback_exposure = COALESCE(?, feedback_exposure), updated_at = ?
       WHERE id = ?`,
      status,
      assistance ?? null,
      feedbackExposure ?? null,
      now(),
      runId,
    );
  });
}

export async function createRound(input: {
  runId: string;
  sequence: number;
  targetDurationMs: number;
  classification: StoredPracticeRound["classification"];
}): Promise<string> {
  await initializeStorage();
  return serialized(async () => {
    const id = nextId("round");
    if (!isNative) {
      const state = readWebState();
      state.sequence += 1;
      state.rounds.push({
        id,
        runId: input.runId,
        sequence: input.sequence,
        targetDurationMs: input.targetDurationMs,
        classification: input.classification,
        status: "ready",
        interrupted: false,
        createdOrder: state.sequence,
      });
      writeWebState(state);
      return id;
    }
    const db = await database();
    await db.runAsync(
      `INSERT INTO practice_rounds
        (id, run_id, sequence, target_duration_ms, classification, status, interrupted, created_order)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      id,
      input.runId,
      input.sequence,
      input.targetDurationMs,
      input.classification,
      "ready",
      0,
      Date.now(),
    );
    return id;
  });
}

export async function updateRoundStatus(
  roundId: string,
  status: StoredPracticeRound["status"],
  interrupted = false,
): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const round = state.rounds.find((candidate) => candidate.id === roundId);
      if (round) {
        round.status = status;
        round.interrupted = interrupted || round.interrupted;
      }
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      "UPDATE practice_rounds SET status = ?, interrupted = CASE WHEN ? = 1 THEN 1 ELSE interrupted END WHERE id = ?",
      status,
      interrupted ? 1 : 0,
      roundId,
    );
  });
}

export async function saveRecording(recording: StoredRecording): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      state.recordings = [
        recording,
        ...state.recordings.filter((candidate) => candidate.id !== recording.id),
      ];
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      `INSERT OR REPLACE INTO recordings
        (id, round_id, relative_path, mime_type, container, codec, bytes, hash,
         decoded_duration_ms, client_duration_ms, created_order, interrupted,
         media_availability, warning, feedback_json, model, usage_json)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      recording.id,
      recording.roundId,
      recording.relativePath,
      recording.mimeType,
      recording.container,
      recording.codec,
      recording.bytes,
      recording.hash,
      recording.decodedDurationMs,
      recording.clientDurationMs,
      recording.createdOrder,
      recording.interrupted ? 1 : 0,
      recording.mediaAvailability,
      recording.warning,
      recording.feedbackJson,
      recording.model,
      recording.usageJson,
    );
  });
}

export async function updateRecordingFeedback(
  recordingId: string,
  feedbackJson: string,
  model: string,
  usageJson: string,
): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const recording = state.recordings.find((candidate) => candidate.id === recordingId);
      if (recording) {
        recording.feedbackJson = feedbackJson;
        recording.model = model;
        recording.usageJson = usageJson;
      }
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      "UPDATE recordings SET feedback_json = ?, model = ?, usage_json = ? WHERE id = ?",
      feedbackJson,
      model,
      usageJson,
      recordingId,
    );
  });
}

export async function updateRecordingDecodedDuration(recordingId: string, decodedDurationMs: number): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const recording = state.recordings.find((candidate) => candidate.id === recordingId);
      if (recording) recording.decodedDurationMs = decodedDurationMs;
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync("UPDATE recordings SET decoded_duration_ms = ? WHERE id = ?", decodedDurationMs, recordingId);
  });
}

export async function saveAnalysis(analysis: StoredAnalysis): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      state.analyses = [
        analysis,
        ...state.analyses.filter((candidate) => candidate.id !== analysis.id),
      ];
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      `INSERT OR REPLACE INTO analyses
        (id, recording_id, recording_hash, processor_version, model_version, config_json,
         metrics_json, status, limitations_json, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      analysis.id,
      analysis.recordingId,
      analysis.recordingHash,
      analysis.processorVersion,
      analysis.modelVersion,
      analysis.configJson,
      analysis.metricsJson,
      analysis.status,
      analysis.limitationsJson,
      analysis.createdAt,
    );
  });
}

export async function upsertOutbox(entry: PracticeOutboxEntry): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      state.outbox = [
        { ...entry },
        ...state.outbox.filter((candidate) => candidate.operationId !== entry.operationId),
      ];
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      `INSERT OR REPLACE INTO outbox
        (operation_id, recording_id, payload_hash, request_json, upload_id, job_id,
         retry_state, next_attempt_at, last_sanitized_error)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      entry.operationId,
      entry.recordingId,
      entry.payloadHash,
      entry.requestJson,
      entry.uploadId,
      entry.jobId,
      entry.retryState,
      entry.nextAttemptAt,
      entry.lastSanitizedError,
    );
  });
}

/** Queue an authenticated server tombstone before local metadata is removed. */
export async function queueServerDeletion(recordingId: string): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    const operationId = nextId("delete-recording");
    const entry: PracticeOutboxEntry = {
      operationId,
      recordingId,
      payloadHash: `delete-${recordingId}`,
      requestJson: JSON.stringify({ kind: "delete-recording", recordingId }),
      uploadId: null,
      jobId: null,
      retryState: "queued",
      nextAttemptAt: null,
      lastSanitizedError: null,
    };
    if (!isNative) {
      const state = readWebState();
      state.outbox = [
        entry,
        ...state.outbox.map((candidate) =>
          candidate.recordingId === recordingId && !["completed", "failed"].includes(String(candidate.retryState))
            ? { ...candidate, retryState: "failed", nextAttemptAt: null, lastSanitizedError: "Cancelled because the recording was deleted locally." }
            : candidate,
        ),
      ];
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync(
      "UPDATE outbox SET retry_state = 'failed', next_attempt_at = NULL, last_sanitized_error = ? WHERE recording_id = ? AND retry_state NOT IN ('completed', 'failed')",
      "Cancelled because the recording was deleted locally.",
      recordingId,
    );
    await db.runAsync(
      `INSERT INTO outbox
        (operation_id, recording_id, payload_hash, request_json, upload_id, job_id,
         retry_state, next_attempt_at, last_sanitized_error)
       VALUES (?, ?, ?, ?, NULL, NULL, 'queued', NULL, NULL)`,
      entry.operationId,
      entry.recordingId,
      entry.payloadHash,
      entry.requestJson,
    );
  });
}

export async function updateOutbox(
  operationId: string,
  patch: Partial<Omit<PracticeOutboxEntry, "operationId">>,
): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const entry = state.outbox.find((candidate) => candidate.operationId === operationId);
      if (entry) Object.assign(entry, patch);
      writeWebState(state);
      return;
    }
    const current = await database().then((db) =>
      db.getFirstAsync<PracticeOutboxEntry>(
        `SELECT operation_id AS operationId, recording_id AS recordingId,
          payload_hash AS payloadHash, request_json AS requestJson,
          upload_id AS uploadId, job_id AS jobId, retry_state AS retryState,
          next_attempt_at AS nextAttemptAt, last_sanitized_error AS lastSanitizedError
         FROM outbox WHERE operation_id = ?`,
        operationId,
      ),
    );
    if (!current) return;
    const next = { ...current, ...patch };
    const db = await database();
    await db.runAsync(
      `UPDATE outbox SET recording_id = ?, payload_hash = ?, request_json = ?,
        upload_id = ?, job_id = ?, retry_state = ?, next_attempt_at = ?,
        last_sanitized_error = ? WHERE operation_id = ?`,
      next.recordingId,
      next.payloadHash,
      next.requestJson,
      next.uploadId,
      next.jobId,
      next.retryState,
      next.nextAttemptAt,
      next.lastSanitizedError,
      operationId,
    );
  });
}

export async function listOutbox(includeCompleted = false): Promise<PracticeOutboxEntry[]> {
  await initializeStorage();
  if (!isNative) {
    return readWebState().outbox
      .filter((entry) => includeCompleted || !["completed", "failed"].includes(String(entry.retryState)))
      .map((entry) => entry as unknown as PracticeOutboxEntry);
  }
  const db = await database();
  const query = includeCompleted
    ? `SELECT operation_id AS operationId, recording_id AS recordingId,
        payload_hash AS payloadHash, request_json AS requestJson,
        upload_id AS uploadId, job_id AS jobId, retry_state AS retryState,
        next_attempt_at AS nextAttemptAt, last_sanitized_error AS lastSanitizedError
       FROM outbox ORDER BY rowid DESC`
    : `SELECT operation_id AS operationId, recording_id AS recordingId,
        payload_hash AS payloadHash, request_json AS requestJson,
        upload_id AS uploadId, job_id AS jobId, retry_state AS retryState,
        next_attempt_at AS nextAttemptAt, last_sanitized_error AS lastSanitizedError
       FROM outbox WHERE retry_state NOT IN ('completed', 'failed') ORDER BY rowid DESC`;
  return db.getAllAsync<PracticeOutboxEntry>(query);
}

export async function listRuns(): Promise<StoredPracticeRun[]> {
  await initializeStorage();
  if (!isNative) return readWebState().runs;
  const db = await database();
  return db.getAllAsync<StoredPracticeRun>(
    "SELECT id, snapshot_json AS snapshot, mode, created_order AS createdOrder, status, assistance, feedback_exposure AS feedbackExposure, updated_at AS updatedAt FROM practice_runs ORDER BY created_order DESC",
  );
}

export async function listRounds(runId?: string): Promise<StoredPracticeRound[]> {
  await initializeStorage();
  if (!isNative) {
    const rounds = readWebState().rounds;
    return runId ? rounds.filter((round) => round.runId === runId) : rounds;
  }
  const db = await database();
  if (runId) {
    return db.getAllAsync<StoredPracticeRound>(
      "SELECT id, run_id AS runId, sequence, target_duration_ms AS targetDurationMs, classification, status, interrupted, created_order AS createdOrder FROM practice_rounds WHERE run_id = ? ORDER BY sequence ASC",
      runId,
    );
  }
  return db.getAllAsync<StoredPracticeRound>(
    "SELECT id, run_id AS runId, sequence, target_duration_ms AS targetDurationMs, classification, status, interrupted, created_order AS createdOrder FROM practice_rounds ORDER BY created_order DESC",
  );
}

export async function listRecordings(runId?: string): Promise<StoredRecording[]> {
  await initializeStorage();
  if (!isNative) {
    const state = readWebState();
    if (!runId) return state.recordings;
    const roundIds = new Set(state.rounds.filter((round) => round.runId === runId).map((round) => round.id));
    return state.recordings.filter((recording) => roundIds.has(recording.roundId));
  }
  const db = await database();
  if (runId) {
    return db.getAllAsync<StoredRecording>(
      `SELECT recordings.id, round_id AS roundId, relative_path AS relativePath,
        mime_type AS mimeType, container, codec, bytes, hash,
        decoded_duration_ms AS decodedDurationMs, client_duration_ms AS clientDurationMs,
        recordings.created_order AS createdOrder, recordings.interrupted,
        media_availability AS mediaAvailability, warning, feedback_json AS feedbackJson,
        model, usage_json AS usageJson
       FROM recordings JOIN practice_rounds ON practice_rounds.id = recordings.round_id
       WHERE practice_rounds.run_id = ? ORDER BY recordings.created_order ASC`,
      runId,
    );
  }
  return db.getAllAsync<StoredRecording>(
    `SELECT id, round_id AS roundId, relative_path AS relativePath, mime_type AS mimeType,
      container, codec, bytes, hash, decoded_duration_ms AS decodedDurationMs,
      client_duration_ms AS clientDurationMs, created_order AS createdOrder, interrupted,
      media_availability AS mediaAvailability, warning, feedback_json AS feedbackJson,
      model, usage_json AS usageJson FROM recordings ORDER BY created_order DESC`,
  );
}

export async function listAnalyses(recordingId?: string): Promise<StoredAnalysis[]> {
  await initializeStorage();
  if (!isNative) {
    const analyses = readWebState().analyses;
    return recordingId ? analyses.filter((analysis) => analysis.recordingId === recordingId) : analyses;
  }
  const db = await database();
  if (recordingId) {
    return db.getAllAsync<StoredAnalysis>(
      `SELECT id, recording_id AS recordingId, recording_hash AS recordingHash,
        processor_version AS processorVersion, model_version AS modelVersion,
        config_json AS configJson, metrics_json AS metricsJson, status,
        limitations_json AS limitationsJson, created_at AS createdAt
       FROM analyses WHERE recording_id = ? ORDER BY created_at DESC`,
      recordingId,
    );
  }
  return db.getAllAsync<StoredAnalysis>(
    `SELECT id, recording_id AS recordingId, recording_hash AS recordingHash,
      processor_version AS processorVersion, model_version AS modelVersion,
      config_json AS configJson, metrics_json AS metricsJson, status,
      limitations_json AS limitationsJson, created_at AS createdAt FROM analyses ORDER BY created_at DESC`,
  );
}

export async function getSettings(): Promise<PracticeSettings> {
  await initializeStorage();
  if (!isNative) return { ...DEFAULT_PRACTICE_SETTINGS, ...readWebState().settings };
  const db = await database();
  const row = await db.getFirstAsync<{ valueJson: string }>(
    "SELECT value_json AS valueJson FROM practice_settings WHERE key = ?",
    "settings",
  );
  try {
    return { ...DEFAULT_PRACTICE_SETTINGS, ...(row ? JSON.parse(row.valueJson) : {}) };
  } catch {
    return DEFAULT_PRACTICE_SETTINGS;
  }
}

export async function updateSettings(settings: Partial<PracticeSettings>): Promise<PracticeSettings> {
  await initializeStorage();
  return serialized(async () => {
    const next = { ...(await getSettings()), ...settings };
    if (!Number.isInteger(next.preparationSeconds) || next.preparationSeconds < 0 || next.preparationSeconds > 900) {
      throw new Error("Photo preparation must be a whole number from 0 to 900 seconds.");
    }
    if (settings.quotaGb !== undefined) {
      await enforceQuota(Number(next.quotaGb), 0);
    }
    if (!isNative) {
      const state = readWebState();
      state.settings = next;
      writeWebState(state);
    } else {
      const db = await database();
      await db.runAsync(
        "INSERT OR REPLACE INTO practice_settings (key, value_json) VALUES (?, ?)",
        "settings",
        JSON.stringify(next),
      );
    }
    return next;
  });
}

function protectedRoundStatus(status: string): boolean {
  return !["round_saved", "completed"].includes(status);
}

function activeOutboxState(value: unknown): boolean {
  return !["completed", "failed"].includes(String(value));
}

async function enforceQuota(quotaGb: number, nextReservation: number): Promise<void> {
  const quota = quotaBytes(quotaGb);
  if (nextReservation < 0 || !Number.isFinite(nextReservation)) {
    throw new Error("The requested audio reservation is invalid.");
  }

  if (!isNative) {
    const state = readWebState();
    const protectedRecordingIds = new Set(
      state.outbox
        .filter((entry) => activeOutboxState(entry.retryState))
        .map((entry) => String(entry.recordingId ?? "")),
    );
    const rounds = new Map(state.rounds.map((round) => [round.id, round.status]));
    const candidates: QuotaRecording[] = state.recordings.map((recording) => ({
      id: recording.id,
      createdOrder: recording.createdOrder,
      bytes: recording.bytes,
      protected:
        protectedRecordingIds.has(recording.id) ||
        protectedRoundStatus(rounds.get(recording.roundId) ?? "ready"),
      status: recording.mediaAvailability,
    }));
    const actualBytes = state.recordings
      .filter((recording) => recording.mediaAvailability === "ready" && recording.mimeType.startsWith("audio/"))
      .reduce((total, recording) => total + recording.bytes, 0);
    const reservations = state.mediaOperations
      .filter((operation) => operation.state === "pending")
      .reduce((total, operation) => total + Number(operation.reservedBytes ?? 0), 0);
    const evictions = selectEvictions(candidates, quota, actualBytes, reservations, nextReservation);
    if (actualBytes + reservations + nextReservation > quota && !evictions.length) {
      throw new Error("Audio storage is full. Finish or export a current exercise, delete audio, or increase the quota.");
    }
    for (const recordingId of evictions) {
      const recording = state.recordings.find((candidate) => candidate.id === recordingId);
      if (recording) recording.mediaAvailability = "evicted";
    }
    writeWebState(state);
    return;
  }

  if (Paths.availableDiskSpace < DEFAULT_FREE_SPACE_RESERVE_BYTES + nextReservation) {
    throw new Error("The device does not have enough free space for this recording. Export or delete audio first.");
  }
  const db = await database();
  const usage = await db.getFirstAsync<{ actualBytes: number; reservations: number }>(
    `SELECT
      COALESCE((SELECT SUM(bytes) FROM recordings WHERE media_availability = 'ready' AND mime_type LIKE 'audio/%'), 0) AS actualBytes,
      COALESCE((SELECT SUM(reserved_bytes) FROM media_operations WHERE state = 'pending'), 0) AS reservations`,
  );
  const actualBytes = Number(usage?.actualBytes ?? 0);
  const reservations = Number(usage?.reservations ?? 0);
  const recordings = await db.getAllAsync<{
    id: string;
    relativePath: string | null;
    createdOrder: number;
    bytes: number;
    mediaAvailability: "ready" | "evicted" | "missing";
    protected: number;
  }>(
    `SELECT recordings.id, recordings.relative_path AS relativePath,
      recordings.created_order AS createdOrder, recordings.bytes,
      recordings.media_availability AS mediaAvailability,
      CASE WHEN practice_rounds.status IN ('round_saved', 'completed')
        AND NOT EXISTS (
          SELECT 1 FROM outbox
          WHERE outbox.recording_id = recordings.id
            AND outbox.retry_state NOT IN ('completed', 'failed')
        ) THEN 0 ELSE 1 END AS protected
     FROM recordings
     JOIN practice_rounds ON practice_rounds.id = recordings.round_id
     WHERE recordings.mime_type LIKE 'audio/%'`,
  );
  const candidates: QuotaRecording[] = recordings.map((recording) => ({
    id: recording.id,
    createdOrder: Number(recording.createdOrder),
    bytes: Number(recording.bytes),
    protected: recording.protected !== 0,
    status: recording.mediaAvailability,
  }));
  const evictions = selectEvictions(candidates, quota, actualBytes, reservations, nextReservation);
  if (actualBytes + reservations + nextReservation > quota && !evictions.length) {
    throw new Error("Audio storage is full. Finish or export a current exercise, delete audio, or increase the quota.");
  }
  for (const recordingId of evictions) {
    const recording = recordings.find((candidate) => candidate.id === recordingId);
    if (!recording?.relativePath) {
      await db.runAsync("UPDATE recordings SET media_availability = 'missing' WHERE id = ?", recordingId);
      continue;
    }
    const file = nativeFileForRelativePath(recording.relativePath);
    try {
      if (file.info().exists) file.delete();
      if (file.info().exists) throw new Error("managed file still exists after deletion");
    } catch {
      throw new Error("Audio storage could not remove an old file safely. No new recording was started.");
    }
    await db.runAsync(
      "UPDATE recordings SET media_availability = 'evicted' WHERE id = ? AND media_availability = 'ready'",
      recordingId,
    );
  }
}

export async function enforceAudioQuota(quotaGb: number): Promise<void> {
  await initializeStorage();
  await serialized(() => enforceQuota(quotaGb, 0));
}

export async function reserveMediaBytes(recordingId: string, bytes: number): Promise<string> {
  await initializeStorage();
  return serialized(async () => {
    await enforceQuota((await getSettings()).quotaGb, bytes);
    const operationId = nextId("media-reservation");
    if (!isNative) {
      const state = readWebState();
      state.mediaOperations.push({
        operationId,
        recordingId,
        operation: "reserve",
        reservedBytes: bytes,
        state: "pending",
        createdAt: now(),
      });
      writeWebState(state);
      return operationId;
    }
    const db = await database();
    await db.runAsync(
      `INSERT INTO media_operations
        (operation_id, recording_id, operation, relative_path, reserved_bytes, state, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)`,
      operationId,
      recordingId,
      "reserve",
      null,
      bytes,
      "pending",
      now(),
    );
    return operationId;
  });
}

export async function completeMediaOperation(operationId: string, stateValue: "committed" | "released" | "failed"): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const operation = state.mediaOperations.find((candidate) => candidate.operationId === operationId);
      if (operation) operation.state = stateValue;
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync("UPDATE media_operations SET state = ? WHERE operation_id = ?", stateValue, operationId);
  });
}

export async function recordMediaArtifact(operationId: string, relativePath: string): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) return;
    const db = await database();
    await db.runAsync(
      "UPDATE media_operations SET relative_path = ? WHERE operation_id = ? AND state = 'pending'",
      relativePath,
      operationId,
    );
  });
}

export async function getStorageUsage(): Promise<StorageUsage> {
  await initializeStorage();
  if (!isNative) {
    const state = readWebState();
    return {
      audioBytes: state.recordings
        .filter((recording) => recording.mediaAvailability === "ready" && recording.mimeType.startsWith("audio/"))
        .reduce((total, recording) => total + recording.bytes, 0),
      reservedBytes: state.mediaOperations
        .filter((operation) => operation.state === "pending")
        .reduce((total, operation) => total + Number(operation.reservedBytes ?? 0), 0),
      recordingCount: state.recordings.filter(
        (recording) => recording.mediaAvailability === "ready" && recording.mimeType.startsWith("audio/"),
      ).length,
    };
  }
  const db = await database();
  const row = await db.getFirstAsync<{ audioBytes: number; reservedBytes: number; recordingCount: number }>(
    `SELECT
      COALESCE((SELECT SUM(bytes) FROM recordings WHERE media_availability = 'ready' AND mime_type LIKE 'audio/%'), 0) AS audioBytes,
      COALESCE((SELECT SUM(reserved_bytes) FROM media_operations WHERE state = 'pending'), 0) AS reservedBytes,
      (SELECT COUNT(*) FROM recordings WHERE media_availability = 'ready' AND mime_type LIKE 'audio/%') AS recordingCount`,
  );
  return row ?? { audioBytes: 0, reservedBytes: 0, recordingCount: 0 };
}

export async function markRecordingAvailability(
  recordingId: string,
  availability: "ready" | "evicted" | "missing",
): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      const recording = state.recordings.find((candidate) => candidate.id === recordingId);
      if (recording) recording.mediaAvailability = availability;
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync("UPDATE recordings SET media_availability = ? WHERE id = ?", availability, recordingId);
  });
}

export async function deleteRecording(recordingId: string): Promise<void> {
  await initializeStorage();
  await serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      state.recordings = state.recordings.filter((recording) => recording.id !== recordingId);
      state.analyses = state.analyses.filter((analysis) => analysis.recordingId !== recordingId);
      writeWebState(state);
      return;
    }
    const db = await database();
    await db.runAsync("DELETE FROM recordings WHERE id = ?", recordingId);
  });
}

export async function deleteRun(runId: string): Promise<StoredRecording[]> {
  await initializeStorage();
  const recordings = await listRecordings(runId);
  await Promise.all(recordings.map((recording) => queueServerDeletion(recording.id)));
  return serialized(async () => {
    if (!isNative) {
      const state = readWebState();
      state.runs = state.runs.filter((run) => run.id !== runId);
      const roundIds = new Set(state.rounds.filter((round) => round.runId === runId).map((round) => round.id));
      state.rounds = state.rounds.filter((round) => round.runId !== runId);
      state.recordings = state.recordings.filter((recording) => !roundIds.has(recording.roundId));
      state.analyses = state.analyses.filter((analysis) => !recordings.some((recording) => recording.id === analysis.recordingId));
      writeWebState(state);
      return recordings;
    }
    const db = await database();
    await db.runAsync("DELETE FROM practice_runs WHERE id = ?", runId);
    return recordings;
  });
}

export function parseSnapshot(run: StoredPracticeRun): TaskSnapshot | null {
  try {
    return JSON.parse(run.snapshot) as TaskSnapshot;
  } catch {
    return null;
  }
}
