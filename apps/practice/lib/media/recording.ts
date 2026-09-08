import { Directory, File as ExpoFile, Paths } from "expo-file-system";
import { CryptoDigestAlgorithm, digest } from "expo-crypto";
import * as ImageManipulator from "expo-image-manipulator";
import * as Sharing from "expo-sharing";

import type { RecordingArtifact } from "@/lib/types";

const IS_WEB = process.env.EXPO_OS === "web";
const RECORDINGS_DIRECTORY = "recordings";

export const AUDIO_MAX_BYTES = 10 * 1024 * 1024;
export const AUDIO_MAX_DURATION_MS = 10 * 60 * 1000;
export const DEFAULT_CAPTURE_RESERVATION_BYTES = AUDIO_MAX_BYTES;

export type CapturedRecordingInput = {
  id: string;
  uri: string;
  mimeType?: string;
  clientDurationMs: number;
  interrupted?: boolean;
  warning?: string;
};

export type PersistedImage = {
  uri: string;
  relativePath: string | null;
  mimeType: string;
  bytes: number;
  sha256?: string;
};

function extensionForMime(mimeType: string): string {
  const base = mimeType.toLowerCase().split(";", 1)[0];
  if (base === "audio/m4a" || base === "audio/mp4") return ".m4a";
  if (base === "audio/ogg" || base === "audio/opus") return ".ogg";
  return ".webm";
}

export function guessMimeType(uri: string, supplied?: string): string {
  if (supplied?.trim()) return supplied.split(";", 1)[0].trim().toLowerCase();
  const lower = uri.toLowerCase();
  if (lower.endsWith(".m4a") || lower.endsWith(".mp4")) return "audio/m4a";
  if (lower.endsWith(".ogg")) return "audio/ogg";
  if (lower.endsWith(".opus")) return "audio/opus";
  return "audio/webm";
}

function containerFor(mimeType: string): string {
  const base = mimeType.toLowerCase().split(";", 1)[0];
  if (base === "audio/m4a" || base === "audio/mp4") return "m4a/mp4";
  if (base === "audio/ogg" || base === "audio/opus") return "ogg";
  return "webm";
}

function codecFor(mimeType: string): string {
  const base = mimeType.toLowerCase().split(";", 1)[0];
  if (base === "audio/m4a" || base === "audio/mp4") return "aac-lc-or-device-reported";
  if (base === "audio/opus" || base === "audio/ogg") return "opus";
  return "opus-or-browser-reported";
}

export async function hashBlobSha256(blob: Blob): Promise<string> {
  if (globalThis.crypto?.subtle) {
    const digestBuffer = await globalThis.crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
    return Array.from(new Uint8Array(digestBuffer), (byte) => byte.toString(16).padStart(2, "0")).join("");
  }
  const digestBuffer = await digest(CryptoDigestAlgorithm.SHA256, await blob.arrayBuffer());
  return Array.from(new Uint8Array(digestBuffer), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function relativePathFor(id: string, mimeType: string): string {
  return `${RECORDINGS_DIRECTORY}/${id}${extensionForMime(mimeType)}`;
}

export function uriForRelativePath(relativePath: string): string {
  const parts = relativePath.split("/").filter(Boolean);
  return new ExpoFile(new Directory(Paths.document, ...parts.slice(0, -1)), parts.at(-1) ?? "recording.m4a").uri;
}

async function finalizeWeb(input: CapturedRecordingInput, mimeType: string): Promise<RecordingArtifact> {
  const response = await fetch(input.uri);
  const blob = await response.blob();
  if (!blob.size) throw new Error("The recorder returned an empty file.");
  const objectUrl = typeof URL === "undefined" ? undefined : URL.createObjectURL(blob);
  return {
    id: input.id,
    uri: input.uri,
    blob,
    mimeType: blob.type || mimeType,
    container: containerFor(blob.type || mimeType),
    codec: codecFor(blob.type || mimeType),
    durationMs: Math.max(0, input.clientDurationMs),
    clientDurationMs: Math.max(0, input.clientDurationMs),
    decodedDurationMs: null,
    bytes: blob.size,
    hash: await hashBlobSha256(blob),
    createdOrder: Date.now(),
    interrupted: input.interrupted ?? false,
    mediaAvailability: "ready",
    objectUrl,
    warning: input.warning,
  };
}

async function finalizeNative(input: CapturedRecordingInput, mimeType: string): Promise<RecordingArtifact> {
  const directory = new Directory(Paths.document, RECORDINGS_DIRECTORY);
  directory.create({ intermediates: true, idempotent: true });

  const source = new ExpoFile(input.uri);
  const sourceInfo = source.info();
  if (!sourceInfo.exists) throw new Error("The staged recording no longer exists.");

  const relativePath = relativePathFor(input.id, mimeType);
  const destination = new ExpoFile(Paths.document, relativePath);
  if (destination.info().exists) destination.delete();
  try {
    await source.move(destination, { overwrite: true });
  } catch {
    await source.copy(destination, { overwrite: true });
  }
  const info = destination.info({ md5: true });
  if (!info.exists || !info.size) throw new Error("The durable recording file is empty.");

  return {
    id: input.id,
    uri: destination.uri,
    relativePath,
    mimeType,
    container: containerFor(mimeType),
    codec: codecFor(mimeType),
    durationMs: Math.max(0, input.clientDurationMs),
    clientDurationMs: Math.max(0, input.clientDurationMs),
    decodedDurationMs: null,
    bytes: info.size,
    hash: await hashBlobSha256(destination),
    createdOrder: Date.now(),
    interrupted: input.interrupted ?? false,
    mediaAvailability: "ready",
    warning: input.warning,
  };
}

export async function finalizeCapturedRecording(input: CapturedRecordingInput): Promise<RecordingArtifact> {
  const mimeType = guessMimeType(input.uri, input.mimeType);
  return IS_WEB ? finalizeWeb(input, mimeType) : finalizeNative(input, mimeType);
}

export async function persistImageAsset(
  uri: string,
  mimeType: string,
  id: string,
): Promise<PersistedImage> {
  const normalizedMimeType = mimeType || "image/jpeg";
  if (IS_WEB) {
    const blob = uri.startsWith("blob:") ? await (await fetch(uri)).blob() : null;
    return {
      uri,
      relativePath: null,
      mimeType: blob?.type || normalizedMimeType,
      bytes: blob?.size ?? 0,
      sha256: blob ? await hashBlobSha256(blob) : undefined,
    };
  }
  const directory = new Directory(Paths.document, "images");
  directory.create({ intermediates: true, idempotent: true });
  const manipulated = await ImageManipulator.manipulateAsync(
    uri,
    [{ resize: { width: 1600 } }],
    { compress: 0.82, format: ImageManipulator.SaveFormat.JPEG },
  );
  const source = new ExpoFile(manipulated.uri);
  const destination = new ExpoFile(directory, `${id}.jpg`);
  if (destination.info().exists) destination.delete();
  await source.copy(destination, { overwrite: true });
  const info = destination.info();
  if (!info.size || info.size > 5 * 1024 * 1024) {
    if (destination.info().exists) destination.delete();
    throw new Error("The processed image exceeds the 5 MiB upload limit.");
  }
  const blob = new Blob([await destination.arrayBuffer()], { type: "image/jpeg" });
  return {
    uri: destination.uri,
    relativePath: `images/${id}.jpg`,
    mimeType: "image/jpeg",
    bytes: info.size,
    sha256: await hashBlobSha256(blob),
  };
}

export function fileUriForArtifact(artifact: RecordingArtifact): string {
  return artifact.relativePath ? uriForRelativePath(artifact.relativePath) : artifact.uri;
}

export async function shareRecordingArtifact(artifact: RecordingArtifact): Promise<void> {
  if (artifact.mediaAvailability && artifact.mediaAvailability !== "ready") {
    throw new Error("This audio file is no longer available in local storage.");
  }
  if (IS_WEB && typeof document !== "undefined") {
    const link = document.createElement("a");
    link.href = artifact.objectUrl ?? artifact.uri;
    link.download = artifact.mimeType.includes("m4a") ? "aprendiendo-practice-take.m4a" : "aprendiendo-practice-take.webm";
    link.click();
    return;
  }
  const uri = fileUriForArtifact(artifact);
  if (await Sharing.isAvailableAsync()) {
    await Sharing.shareAsync(uri, { mimeType: artifact.mimeType, dialogTitle: "Share practice recording" });
    return;
  }
  throw new Error("Sharing is unavailable on this device.");
}

export function deleteRecordingFile(artifact: RecordingArtifact): void {
  if (IS_WEB) {
    if (artifact.objectUrl && typeof URL !== "undefined") URL.revokeObjectURL(artifact.objectUrl);
    return;
  }
  try {
    const file = new ExpoFile(fileUriForArtifact(artifact));
    if (file.info().exists) file.delete();
  } catch {
    // Reconciliation will leave a missing row visible instead of claiming deletion.
  }
}

export function assertAudioWithinLimits(artifact: RecordingArtifact): string | null {
  if (artifact.bytes > AUDIO_MAX_BYTES) return "This recording is larger than the 10 MiB processing limit.";
  if (artifact.durationMs > AUDIO_MAX_DURATION_MS) return "This recording is longer than the 10 minute processing limit.";
  return null;
}
