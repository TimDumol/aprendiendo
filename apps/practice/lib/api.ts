import type {
  AssessmentResponse,
  HealthResponse,
  RecordingArtifact,
} from "./types";
import { fileUriForArtifact } from "@/lib/media/recording";
import { getAccessToken, PRACTICE_API_URL as AUTH_PRACTICE_API_URL } from "@/lib/auth";
import { logWarn } from "@/lib/logging";
import {
  allowlistedApiPath,
  allowlistedBrowserAssetUrl,
  allowlistedHttpUrl,
  configuredHttpOrigin,
} from "@/lib/network";
import { requestDurableFeedback } from "@/lib/sync/outbox";

export const MVP_API_URL = configuredHttpOrigin(
  process.env.EXPO_PUBLIC_MVP_API_URL ?? "http://127.0.0.1:8082",
);

export const FEEDBACK_TIMEOUT_MS = 135_000;
export const PRACTICE_API_URL = AUTH_PRACTICE_API_URL;

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly requestId: string | null;

  constructor(message: string, status: number, code: string, requestId: string | null = null) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
  }
}

type ErrorPayload = {
  error?: {
    code?: unknown;
    message?: unknown;
  };
};

function errorFromResponse(response: Response, payload: ErrorPayload): ApiError {
  const requestId = response.headers.get("x-request-id");
  const code =
    typeof payload.error?.code === "string" ? payload.error.code : "request_failed";
  const message =
    typeof payload.error?.message === "string"
      ? payload.error.message
      : `The practice service returned HTTP ${response.status}.`;
  logWarn("practice.api", "API request failed", {
    code,
    requestId,
    status: response.status,
  });
  const requestSuffix = requestId ? ` (request ${requestId})` : "";
  return new ApiError(`${message}${requestSuffix}`, response.status, code, requestId);
}

async function jsonOrError(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    throw new ApiError(
      "The practice service returned an unreadable response.",
      response.status,
      "invalid_response",
      response.headers.get("x-request-id"),
    );
  }
}

async function fetchWithTimeout(
  input: string,
  init: RequestInit,
  timeoutMs: number,
  allowedOrigin: string,
): Promise<Response> {
  const safeInput = allowlistedHttpUrl(input, [allowedOrigin], "Practice API request");
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  const externalSignal = init.signal;
  const abortExternal = () => controller.abort();
  externalSignal?.addEventListener("abort", abortExternal, { once: true });

  try {
    // Callers pass paths built from fixed API routes and an origin allowlist.
    // foxguard: ignore[js/no-ssrf]
    return await fetch(safeInput, { ...init, signal: controller.signal });
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") {
      throw new ApiError(
        "The request timed out while waiting for the practice service.",
        504,
        "timeout",
      );
    }
    throw new ApiError(
      "The practice service is unavailable. Your recording is still on this page.",
      0,
      "network_unavailable",
    );
  } finally {
    clearTimeout(timeout);
    externalSignal?.removeEventListener("abort", abortExternal);
  }
}

async function authHeaders(): Promise<Record<string, string>> {
  if (process.env.EXPO_OS === "web") return {};
  const token = await getAccessToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

export async function getHealth(): Promise<HealthResponse> {
  const baseUrl = process.env.EXPO_OS === "web" ? MVP_API_URL : PRACTICE_API_URL;
  if (!baseUrl) {
    throw new ApiError(
      "Cloud coaching is not configured for this build.",
      0,
      "practice_endpoint_missing",
    );
  }
  const response = await fetchWithTimeout(
    allowlistedApiPath(baseUrl, "/health"),
    { method: "GET", headers: await authHeaders() },
    5_000,
    new URL(baseUrl).origin,
  );
  const payload = await jsonOrError(response);
  if (!response.ok) {
    throw errorFromResponse(response, payload as ErrorPayload);
  }
  return {
    status: "ok",
    endpoint: baseUrl,
    tokenPresent: Boolean((await getAccessToken()) ?? null),
  };
}

function filenameForMime(mimeType: string): string {
  const base = mimeType.toLowerCase().split(";", 1)[0];
  if (base === "audio/m4a" || base === "audio/mp4") return "practice-take.m4a";
  if (base === "audio/ogg" || base === "audio/opus") return "practice-take.ogg";
  return "practice-take.webm";
}

export async function requestFeedback(
  artifact: RecordingArtifact,
  task: string,
): Promise<AssessmentResponse> {
  if (process.env.EXPO_OS !== "web") {
    return requestDurableFeedback(artifact, task);
  }
  const form = new FormData();
  // Let fetch set the multipart boundary. Supplying Content-Type here would
  // omit the boundary and makes the Rust multipart parser reject the request.
  if (artifact.blob) {
    form.append("audio", artifact.blob, filenameForMime(artifact.mimeType));
  } else {
    form.append(
      "audio",
      {
        uri: fileUriForArtifact(artifact),
        type: artifact.mimeType,
        name: filenameForMime(artifact.mimeType),
      } as unknown as Blob,
    );
  }
  form.append("mime_type", artifact.mimeType);
  form.append("duration_ms", String(Math.round(artifact.durationMs)));
  form.append("task", task);

  const response = await fetchWithTimeout(
    allowlistedApiPath(MVP_API_URL, "/api/mvp/feedback"),
    { method: "POST", body: form, headers: await authHeaders() },
    FEEDBACK_TIMEOUT_MS,
    new URL(MVP_API_URL).origin,
  );
  const payload = await jsonOrError(response);
  if (!response.ok) {
    throw errorFromResponse(response, payload as ErrorPayload);
  }
  return payload as AssessmentResponse;
}

export type ImageAttachment = {
  uri: string;
  relativePath?: string | null;
  mimeType: string;
  bytes: number;
  name?: string;
};

export async function requestPhotoFeedback(
  artifact: RecordingArtifact,
  task: string,
  image: ImageAttachment,
): Promise<AssessmentResponse> {
  if (process.env.EXPO_OS !== "web") {
    return requestDurableFeedback(artifact, task, image);
  }
  const form = new FormData();
  if (artifact.blob) {
    form.append("audio", artifact.blob, filenameForMime(artifact.mimeType));
  } else {
    form.append(
      "audio",
      {
        uri: fileUriForArtifact(artifact),
        type: artifact.mimeType,
        name: filenameForMime(artifact.mimeType),
      } as unknown as Blob,
    );
  }
  form.append("mime_type", artifact.mimeType);
  form.append("duration_ms", String(Math.round(artifact.durationMs)));
  form.append("task", task);
  if (image.uri.startsWith("blob:") || image.uri.startsWith("http")) {
    const imageUrl = allowlistedBrowserAssetUrl(image.uri);
    // Browser object URLs and same-origin assets are never remote destinations.
    // foxguard: ignore[js/no-ssrf]
    const imageResponse = await fetch(imageUrl);
    if (!imageResponse.ok) {
      throw new ApiError("The selected image could not be read.", imageResponse.status, "image_unavailable");
    }
    const imageBlob = await imageResponse.blob();
    form.append("image", imageBlob, image.name ?? "practice-image.jpg");
  } else {
    form.append(
      "image",
      { uri: image.uri, type: image.mimeType, name: image.name ?? "practice-image.jpg" } as unknown as Blob,
    );
  }
  form.append("image_mime_type", image.mimeType);

  const response = await fetchWithTimeout(
    allowlistedApiPath(MVP_API_URL, "/api/mvp/feedback"),
    { method: "POST", body: form, headers: await authHeaders() },
    FEEDBACK_TIMEOUT_MS,
    new URL(MVP_API_URL).origin,
  );
  const payload = await jsonOrError(response);
  if (!response.ok) throw errorFromResponse(response, payload as ErrorPayload);
  return payload as AssessmentResponse;
}
