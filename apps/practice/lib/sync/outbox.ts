import { CryptoDigestAlgorithm, digest, randomUUID } from "expo-crypto";
import { File as ExpoFile } from "expo-file-system";

import { getAccessToken, PRACTICE_API_URL as AUTH_PRACTICE_API_URL } from "@/lib/auth";
import { errorMessage, logError } from "@/lib/logging";
import { hashBlobSha256, uriForRelativePath } from "@/lib/media/recording";
import {
  getSettings,
  listOutbox,
  saveAnalysis,
  upsertOutbox,
  updateRecordingDecodedDuration,
  updateRecordingFeedback,
  updateOutbox,
  type PracticeOutboxEntry,
} from "@/lib/storage/repository";
import type { AssessmentResponse, RecordingArtifact } from "@/lib/types";
import type { ImageAttachment } from "@/lib/api";

const PRACTICE_API_URL = AUTH_PRACTICE_API_URL;
const REQUEST_TIMEOUT_MS = 135_000;
const POLL_INTERVAL_MS = 1_000;
const POLL_LIMIT = 120;

type DurableImage = {
  relativePath: string | null;
  uri: string;
  mimeType: string;
  bytes: number;
  name: string;
};

type DurableRequest = {
  artifact: {
    id: string;
    relativePath: string | null;
    mimeType: string;
    container: string;
    codec: string;
    bytes: number;
    hash: string;
    durationMs: number;
    decodedDurationMs: number | null;
    clientDurationMs: number;
    interrupted: boolean;
    warning?: string;
  };
  task: string;
  image: DurableImage | null;
};

type DurableDeleteRequest = { kind: "delete-recording"; recordingId: string };

type UploadResponse = {
  upload_id: string;
  recording_id: string;
  status: string;
};

type JobResponse = {
  job_id: string;
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
  result?: {
    coaching?: AssessmentResponse;
    metrics?: unknown;
    limitations?: unknown;
    processor_version?: string;
    model_version?: string;
    config?: unknown;
  } | null;
  error?: { message?: string } | null;
};

export class DurableSyncError extends Error {
  readonly retryable: boolean;
  readonly status: number | null;
  readonly requestId: string | null;
  readonly path: string | null;

  constructor(
    message: string,
    retryable = true,
    metadata: { status?: number | null; requestId?: string | null; path?: string | null } = {},
  ) {
    super(message);
    this.name = "DurableSyncError";
    this.retryable = retryable;
    this.status = metadata.status ?? null;
    this.requestId = metadata.requestId ?? null;
    this.path = metadata.path ?? null;
  }
}

function assertConfigured(): void {
  if (!PRACTICE_API_URL) {
    throw new DurableSyncError(
      "Cloud coaching is not configured for this build. The finalized recording remains on this device.",
      true,
    );
  }
}

async function authHeaders(): Promise<Record<string, string>> {
  const token = await getAccessToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

async function requestJson<T>(path: string, init: RequestInit): Promise<T> {
  assertConfigured();
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  try {
    const response = await fetch(`${PRACTICE_API_URL}${path}`, {
      ...init,
      headers: {
        Accept: "application/json",
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        ...(await authHeaders()),
        ...(init.headers ?? {}),
      },
      signal: controller.signal,
    });
    let payload: unknown = null;
    try {
      payload = await response.json();
    } catch {
      payload = null;
    }
    if (!response.ok) {
      const message =
        typeof payload === "object" && payload && "error" in payload
          ? String((payload as { error?: { message?: unknown } }).error?.message ?? "The practice service rejected the request.")
          : `The practice service returned HTTP ${response.status}.`;
      const requestId = response.headers.get("x-request-id");
      const requestSuffix = requestId ? ` (request ${requestId}, ${path})` : ` (${path})`;
      throw new DurableSyncError(
        `${message}${requestSuffix}`,
        response.status >= 500 || response.status === 408 || response.status === 429 || response.status === 401 || response.status === 403,
        { status: response.status, requestId, path },
      );
    }
    return payload as T;
  } catch (error) {
    if (error instanceof DurableSyncError) throw error;
    if (error instanceof Error && error.name === "AbortError") {
      throw new DurableSyncError(`The practice service timed out while requesting ${path}. The request will remain queued.`, true, { path });
    }
    throw new DurableSyncError(`The practice service is unavailable while requesting ${path}: ${errorMessage(error)}. The request will remain queued.`, true, { path });
  } finally {
    clearTimeout(timeout);
  }
}

async function mediaBlob(uri: string, mimeType: string): Promise<Blob> {
  const file = new ExpoFile(uri);
  const bytes = await file.arrayBuffer();
  return new Blob([bytes], { type: mimeType });
}

async function mediaForArtifact(artifact: DurableRequest["artifact"]): Promise<Blob> {
  if (!artifact.relativePath) {
    throw new DurableSyncError("This recording has no durable file path. It cannot be queued for upload.", false);
  }
  return mediaBlob(uriForRelativePath(artifact.relativePath), artifact.mimeType);
}

async function mediaForImage(image: DurableImage): Promise<Blob> {
  if (image.relativePath) return mediaBlob(uriForRelativePath(image.relativePath), image.mimeType);
  if (image.uri.startsWith("file:")) return mediaBlob(image.uri, image.mimeType);
  throw new DurableSyncError("This task image is no longer available for queued upload.", false);
}

async function sha256Bytes(blob: Blob): Promise<string> {
  const buffer = await blob.arrayBuffer();
  try {
    const output = await digest(CryptoDigestAlgorithm.SHA256, buffer);
    return Array.from(new Uint8Array(output), (byte) => byte.toString(16).padStart(2, "0")).join("");
  } catch {
    return hashBlobSha256(blob);
  }
}

function durableRequest(
  artifact: RecordingArtifact,
  task: string,
  image: ImageAttachment | null,
): DurableRequest {
  return {
    artifact: {
      id: artifact.id,
      relativePath: artifact.relativePath ?? null,
      mimeType: artifact.mimeType,
      container: artifact.container ?? "reported-by-device",
      codec: artifact.codec ?? "reported-by-device",
      bytes: artifact.bytes,
      hash: artifact.hash ?? "",
      durationMs: artifact.durationMs,
      decodedDurationMs: artifact.decodedDurationMs ?? null,
      clientDurationMs: artifact.clientDurationMs ?? artifact.durationMs,
      interrupted: artifact.interrupted ?? false,
      warning: artifact.warning ?? undefined,
    },
    task,
    image: image
      ? {
          relativePath: image.relativePath ?? null,
          uri: image.uri,
          mimeType: image.mimeType,
          bytes: image.bytes,
          name: image.name ?? "practice-image",
        }
      : null,
  };
}

async function uploadMedia(
  operationId: string,
  request: DurableRequest,
  existingUploadId: string | null,
  kind: "audio" | "image",
  image: DurableImage | null,
): Promise<string> {
  const media = image ? await mediaForImage(image) : await mediaForArtifact(request.artifact);
  const hash = await sha256Bytes(media);
  const recordingId = image ? `image-${request.artifact.id}` : request.artifact.id;
  const idempotencyKey = `${operationId}:${kind}`;
  let uploadId = existingUploadId;
  if (!uploadId) {
    const created = await requestJson<UploadResponse>("/api/practice/v1/uploads", {
      method: "POST",
      body: JSON.stringify({
        recording_id: recordingId,
        sha256: hash,
        bytes: media.size,
        mime_type: image?.mimeType ?? request.artifact.mimeType,
        kind,
        idempotency_key: idempotencyKey,
      }),
    });
    uploadId = created.upload_id;
    if (kind === "audio") await updateOutbox(operationId, { uploadId, retryState: "uploading" });
  }
  await requestJson<{ status: string }>(`/api/practice/v1/uploads/${uploadId}/content`, {
    method: "PUT",
    body: media,
    headers: { "Content-Type": image?.mimeType ?? request.artifact.mimeType },
  });
  await requestJson<{ status: string }>(`/api/practice/v1/uploads/${uploadId}/finalize`, { method: "POST" });
  return uploadId;
}

async function processDeletion(entry: PracticeOutboxEntry, request: DurableDeleteRequest): Promise<void> {
  if (!PRACTICE_API_URL) {
    throw new DurableSyncError("The delete tombstone is waiting for an authenticated practice endpoint.");
  }
  await updateOutbox(entry.operationId, { retryState: "uploading", lastSanitizedError: null });
  await requestJson(`/api/practice/v1/recordings/${encodeURIComponent(request.recordingId)}`, { method: "DELETE" });
  await updateOutbox(entry.operationId, { retryState: "completed", nextAttemptAt: null, lastSanitizedError: null });
}

async function processEntry(entry: PracticeOutboxEntry, request: DurableRequest): Promise<AssessmentResponse> {
  await updateOutbox(entry.operationId, { retryState: "uploading", lastSanitizedError: null });
  const audioUploadId = await uploadMedia(entry.operationId, request, entry.uploadId, "audio", null);
  let imageUploadId: string | null = null;
  let imageHash: string | null = null;
  let visualCoverageUnavailable = false;
  if (request.image) {
    try {
      const imageBlob = await mediaForImage(request.image);
      imageHash = await sha256Bytes(imageBlob);
      imageUploadId = await uploadMedia(entry.operationId, request, null, "image", request.image);
    } catch (error) {
      if (!(error instanceof DurableSyncError) || error.retryable) throw error;
      visualCoverageUnavailable = true;
    }
  }
  await updateOutbox(entry.operationId, { uploadId: audioUploadId, retryState: "processing" });
  let jobId = entry.jobId;
  if (!jobId) {
    const job = await requestJson<JobResponse>("/api/practice/v1/analyses", {
      method: "POST",
      body: JSON.stringify({
        recording_id: request.artifact.id,
        idempotency_key: `${entry.operationId}:analysis`,
        task: request.task,
        duration_ms: Math.round(request.artifact.durationMs),
        requested_stages: ["measurements", "coaching"],
        spending_reservation_usd: null,
        image_upload_id: imageUploadId,
        image_hash: imageHash,
      }),
    });
    jobId = job.job_id;
    await updateOutbox(entry.operationId, { jobId, retryState: "processing" });
  }
  for (let attempt = 0; attempt < POLL_LIMIT; attempt += 1) {
    const job = await requestJson<JobResponse>(`/api/practice/v1/jobs/${jobId}`, { method: "GET" });
    if (job.status === "completed") {
      const result = job.result;
      if (!result) throw new DurableSyncError("The practice worker returned no result.", false);
      const limitations = Array.isArray(result.limitations)
        ? result.limitations.filter((value): value is string => typeof value === "string")
        : [];
      if (result.processor_version || result.metrics !== undefined) {
        await saveAnalysis({
          id: `${entry.operationId}:delivery`,
          recordingId: request.artifact.id,
          recordingHash: request.artifact.hash,
          processorVersion: result.processor_version ?? "unknown",
          modelVersion: result.model_version ?? "unknown",
          configJson: JSON.stringify(result.config ?? {}),
          metricsJson: JSON.stringify(result.metrics ?? null),
          status: result.metrics === null || result.metrics === undefined ? "partial" : "ready",
          limitationsJson: JSON.stringify(limitations),
          createdAt: new Date().toISOString(),
        });
        if (result.metrics && typeof result.metrics === "object" && "duration_seconds" in result.metrics) {
          const decodedDurationSeconds = Number((result.metrics as { duration_seconds?: unknown }).duration_seconds);
          if (Number.isFinite(decodedDurationSeconds) && decodedDurationSeconds >= 0) {
            await updateRecordingDecodedDuration(request.artifact.id, Math.round(decodedDurationSeconds * 1000));
          }
        }
      }
      const response: AssessmentResponse = result.coaching ?? {
        feedback: {
          summary: "Delivery measurements were saved. Cloud coaching is unavailable for this request.",
          transcript: "",
          strengths: [],
          improvements: [],
          limitations: ["No model coaching was published.", ...limitations],
        },
        model: "delivery-analysis",
        elapsed_ms: 0,
        usage: { input_tokens: null, output_tokens: null, thought_tokens: null, estimated_cost_usd: null },
      };
      if (visualCoverageUnavailable) {
        response.feedback.limitations = [
          ...response.feedback.limitations,
          "Visual coverage is unavailable; this coaching result uses the audio and task only.",
        ];
      }
      await updateRecordingFeedback(
        request.artifact.id,
        JSON.stringify(response.feedback),
        response.model,
        JSON.stringify(response.usage),
      );
      await updateOutbox(entry.operationId, { retryState: "completed", nextAttemptAt: null, lastSanitizedError: null });
      return response;
    }
    if (job.status === "failed" || job.status === "cancelled") {
      throw new DurableSyncError(job.error?.message ?? "The practice worker could not analyze this take.", false);
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
  }
  throw new DurableSyncError("The practice worker is still processing. The saved job will be checked again later.");
}

function serializedPayload(request: DurableRequest): string {
  return JSON.stringify(request);
}

export async function requestDurableFeedback(
  artifact: RecordingArtifact,
  task: string,
  image: ImageAttachment | null = null,
): Promise<AssessmentResponse> {
  const settings = await getSettings();
  if (!settings.analysisConsent) {
    throw new DurableSyncError("Paid coaching is disabled in Settings; the recording remains available locally.", false);
  }
  const request = durableRequest(artifact, task, image);
  const requestJson = serializedPayload(request);
  const payloadHash = await sha256Bytes(new Blob([requestJson], { type: "application/json" }));
  const entry: PracticeOutboxEntry = {
    operationId: `analysis-${randomUUID()}`,
    recordingId: artifact.id,
    payloadHash,
    requestJson,
    uploadId: null,
    jobId: null,
    retryState: "queued",
    nextAttemptAt: null,
    lastSanitizedError: null,
  };
  await upsertOutbox(entry);
  try {
    return await processEntry(entry, request);
  } catch (error) {
    const message = error instanceof Error ? error.message : "The analysis request remains queued.";
    const retryable = !(error instanceof DurableSyncError) || error.retryable;
    logError("practice.outbox", error, {
      operation: "request_feedback",
      recordingId: artifact.id,
      retryable,
    });
    await updateOutbox(entry.operationId, {
      retryState: retryable ? "retry" : "failed",
      nextAttemptAt: retryable ? new Date(Date.now() + 60_000).toISOString() : null,
      lastSanitizedError: message.slice(0, 240),
    });
    throw error;
  }
}

export async function drainOutbox(): Promise<void> {
  if (process.env.EXPO_OS === "web") return;
  const entries = await listOutbox();
  const settings = await getSettings();
  for (const entry of entries) {
    try {
      const parsed = JSON.parse(entry.requestJson) as Partial<DurableRequest & DurableDeleteRequest>;
      if (parsed.kind === "delete-recording" && typeof parsed.recordingId === "string") {
        await processDeletion(entry, parsed as DurableDeleteRequest);
      } else if (!settings.analysisConsent) {
        continue;
      } else {
        await processEntry(entry, parsed as DurableRequest);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : "The queued analysis could not be retried.";
      const retryable = !(error instanceof DurableSyncError) || error.retryable;
      logError("practice.outbox", error, { operation: "drain", operationId: entry.operationId, retryable });
      await updateOutbox(entry.operationId, {
        retryState: retryable ? "retry" : "failed",
        nextAttemptAt: retryable ? new Date(Date.now() + 60_000).toISOString() : null,
        lastSanitizedError: message.slice(0, 240),
      });
    }
  }
}
