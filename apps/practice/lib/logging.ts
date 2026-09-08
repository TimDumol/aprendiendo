import { File as ExpoFile, Paths } from "expo-file-system";

import {
  APP_BUILD_DATE,
  APP_BUILD_VARIANT,
  APP_IDENTIFIER,
  APP_VERSION,
} from "./build-info";

export type LogLevel = "debug" | "info" | "warn" | "error";

export type AppLogEntry = {
  id: string;
  timestamp: string;
  level: LogLevel;
  scope: string;
  message: string;
  details: string | null;
};

const LOG_FILE_NAME = "aprendiendo-system-logs.json";
const WEB_STORAGE_KEY = "aprendiendo.practice.system-logs.v1";
const MAX_LOG_ENTRIES = 1_000;
const MAX_TEXT_LENGTH = 8_000;
const IS_WEB = process.env.EXPO_OS === "web";

const originalConsole = {
  debug: console.debug.bind(console),
  error: console.error.bind(console),
  info: console.info.bind(console),
  log: console.log.bind(console),
  trace: console.trace.bind(console),
  warn: console.warn.bind(console),
};

let entries: AppLogEntry[] = [];
let loadPromise: Promise<void> | null = null;
let writePromise: Promise<void> = Promise.resolve();
let diagnosticsInstalled = false;
const listeners = new Set<(next: AppLogEntry[]) => void>();

function truncate(value: string): string {
  if (value.length <= MAX_TEXT_LENGTH) return value;
  return `${value.slice(0, MAX_TEXT_LENGTH)}… [truncated]`;
}

function redact(value: string): string {
  return truncate(
    value
      .replace(/Bearer\s+[^\s"'}]+/gi, "Bearer <redacted>")
      .replace(/((?:api[_-]?key|access[_-]?token|refresh[_-]?token|client_secret|password|verifier)\s*[=:]\s*)[^\s,}"']+/gi, "$1<redacted>"),
  );
}

function serializable(value: unknown, seen = new WeakSet<object>()): unknown {
  if (value instanceof Error) {
    return {
      name: value.name,
      message: value.message,
      stack: value.stack,
    };
  }
  if (typeof value === "bigint") return `${value}n`;
  if (typeof value !== "object" || value === null) return value;
  if (seen.has(value)) return "[Circular]";
  seen.add(value);
  if (Array.isArray(value)) return value.map((item) => serializable(item, seen));
  return Object.fromEntries(
    Object.entries(value).map(([key, item]) => {
      if (/api[_-]?key|access[_-]?token|refresh[_-]?token|client_secret|password|verifier/i.test(key)) {
        return [key, "<redacted>"];
      }
      return [key, serializable(item, seen)];
    }),
  );
}

function formatValue(value: unknown): string {
  if (typeof value === "string") return redact(value);
  try {
    return redact(JSON.stringify(serializable(value)) ?? String(value));
  } catch {
    return redact(String(value));
  }
}

function formatDetails(details: unknown): string | null {
  if (details === undefined || details === null) return null;
  return truncate(
    Array.isArray(details) ? details.map(formatValue).join(" ") : formatValue(details),
  );
}

function logFile(): ExpoFile {
  return new ExpoFile(Paths.document, LOG_FILE_NAME);
}

function notify(): void {
  const next = [...entries].reverse();
  listeners.forEach((listener) => listener(next));
}

async function readStoredEntries(): Promise<AppLogEntry[]> {
  if (IS_WEB) {
    const raw = globalThis.localStorage?.getItem(WEB_STORAGE_KEY);
    return raw ? parseEntries(raw) : [];
  }
  const file = logFile();
  if (!file.exists) return [];
  try {
    return parseEntries(await file.text());
  } catch (error) {
    originalConsole.error("[system] Could not read persisted app logs", error);
    throw new Error(`Could not read persisted app logs: ${errorMessage(error)}`);
  }
}

function parseEntries(raw: string): AppLogEntry[] {
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((value): value is AppLogEntry => {
      if (!value || typeof value !== "object") return false;
      const entry = value as Partial<AppLogEntry>;
      return (
        typeof entry.id === "string" &&
        typeof entry.timestamp === "string" &&
        typeof entry.level === "string" &&
        typeof entry.scope === "string" &&
        typeof entry.message === "string"
      );
    }).slice(-MAX_LOG_ENTRIES);
  } catch {
    return [];
  }
}

async function writeStoredEntries(snapshot: string): Promise<void> {
  if (IS_WEB) {
    globalThis.localStorage?.setItem(WEB_STORAGE_KEY, snapshot);
    return;
  }
  try {
    const file = logFile();
    if (!file.exists) file.create({ intermediates: true });
    file.write(snapshot);
  } catch (error) {
    originalConsole.error("[system] Could not persist app logs", error);
    throw new Error(`Could not persist app logs: ${errorMessage(error)}`);
  }
}

function schedulePersist(): void {
  const snapshot = JSON.stringify(entries);
  writePromise = writePromise
    .catch(() => undefined)
    .then(() => writeStoredEntries(snapshot));
}

function ensureLoaded(): Promise<void> {
  if (!loadPromise) {
    loadPromise = readStoredEntries().then((stored) => {
      entries = stored;
    });
  }
  return loadPromise;
}

function append(entry: AppLogEntry): void {
  entries = [...entries, entry].slice(-MAX_LOG_ENTRIES);
  notify();
  schedulePersist();
}

function enqueue(entry: AppLogEntry): void {
  void ensureLoaded()
    .then(() => append(entry))
    .catch((error) => originalConsole.error("[system] Could not append app log", error));
}

function consoleLevel(method: "debug" | "error" | "info" | "log" | "trace" | "warn"): LogLevel {
  if (method === "error") return "error";
  if (method === "warn") return "warn";
  if (method === "debug" || method === "trace") return "debug";
  return "info";
}

export function errorMessage(error: unknown, fallback = "Unknown error"): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return fallback;
}

export function log(level: LogLevel, scope: string, message: string, details?: unknown): void {
  const detailText = formatDetails(details);
  const entry: AppLogEntry = {
    id: `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
    timestamp: new Date().toISOString(),
    level,
    scope,
    message: redact(message),
    details: detailText,
  };
  const output = detailText ? `${entry.message} ${detailText}` : entry.message;
  originalConsole[level === "debug" ? "debug" : level](
    `[${scope}] ${output}`,
  );
  enqueue(entry);
}

export function logDebug(scope: string, message: string, details?: unknown): void {
  log("debug", scope, message, details);
}

export function logInfo(scope: string, message: string, details?: unknown): void {
  log("info", scope, message, details);
}

export function logWarn(scope: string, message: string, details?: unknown): void {
  log("warn", scope, message, details);
}

export function logError(scope: string, error: unknown, details?: unknown): void {
  const context = {
    ...(details && typeof details === "object" ? details : { context: details }),
    error: serializable(error),
  };
  log("error", scope, errorMessage(error), context);
}

export async function listLogs(): Promise<AppLogEntry[]> {
  await ensureLoaded();
  return [...entries].reverse();
}

export async function clearLogs(): Promise<void> {
  await ensureLoaded();
  entries = [];
  notify();
  schedulePersist();
  await writePromise;
}

export function subscribeLogs(listener: (next: AppLogEntry[]) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function installDiagnostics(): void {
  if (diagnosticsInstalled) return;
  diagnosticsInstalled = true;

  const consoleObject = console as unknown as Record<string, (...args: unknown[]) => void>;
  for (const method of ["debug", "error", "info", "log", "trace", "warn"] as const) {
    const original = originalConsole[method];
    consoleObject[method] = (...args: unknown[]) => {
      original(...args);
      const [first, ...rest] = args;
      const message = typeof first === "string" ? first : "Console output";
      const details = typeof first === "string" ? rest : args;
      enqueue({
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
        timestamp: new Date().toISOString(),
        level: consoleLevel(method),
        scope: "console",
        message: redact(message),
        details: formatDetails(details),
      });
    };
  }

  const globalObject = globalThis as unknown as {
    ErrorUtils?: {
      getGlobalHandler?: () => ((error: Error, isFatal?: boolean) => void) | undefined;
      setGlobalHandler?: (handler: (error: Error, isFatal?: boolean) => void) => void;
    };
    addEventListener?: (type: string, listener: (event: { reason?: unknown }) => void) => void;
    process?: { on?: (event: string, listener: (...args: unknown[]) => void) => void };
  };
  const errorUtils = globalObject.ErrorUtils;
  if (errorUtils?.setGlobalHandler) {
    const previous = errorUtils.getGlobalHandler?.();
    errorUtils.setGlobalHandler((error, isFatal) => {
      logError("global.error", error, { isFatal: Boolean(isFatal) });
      previous?.(error, isFatal);
    });
  }
  globalObject.addEventListener?.("unhandledrejection", (event) => {
    logError("global.unhandled-rejection", event.reason, { event: "unhandledrejection" });
  });
  globalObject.process?.on?.("unhandledRejection", (reason) => {
    logError("global.unhandled-rejection", reason, { event: "unhandledRejection" });
  });

  logInfo("system", "Diagnostics logging installed", {
    version: APP_VERSION,
    buildDate: APP_BUILD_DATE,
    buildVariant: APP_BUILD_VARIANT,
    identifier: APP_IDENTIFIER,
    maxEntries: MAX_LOG_ENTRIES,
  });
}
