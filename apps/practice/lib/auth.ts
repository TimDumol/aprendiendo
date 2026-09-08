import * as Linking from "expo-linking";
import { CryptoDigestAlgorithm, digest, randomUUID } from "expo-crypto";
import * as SecureStore from "expo-secure-store";
import * as WebBrowser from "expo-web-browser";

import { errorMessage, logError, logInfo, logWarn } from "@/lib/logging";
import { allowlistedHttpUrl, configuredHttpOrigin } from "@/lib/network";

const STORAGE_ID = "aprendiendo.practice.access-token";
const DEFAULT_PRACTICE_API_URL = "https://mars.timdumol.com";
const DEFAULT_OAUTH_ISSUER = "https://auth.aries.timdumol.com";
const DEFAULT_OAUTH_CLIENT_ID = "aprendiendo-practice-mobile";
const DEFAULT_REDIRECT_URI = "aprendiendo-practice-mvp://oauth/callback";
const DEFAULT_OAUTH_SCOPE = "openid profile email learning:access";
const OAUTH_TOKEN_TIMEOUT_MS = 30_000;

export const PRACTICE_API_URL = configuredHttpOrigin(
  process.env.EXPO_PUBLIC_PRACTICE_API_URL ??
    (process.env.EXPO_OS === "web" ? "" : DEFAULT_PRACTICE_API_URL),
);

WebBrowser.maybeCompleteAuthSession();

export type OAuthConfiguration = {
  configured: boolean;
  issues: string[];
  apiUrl: string;
  issuer: string;
  authorizationUrl: string;
  tokenUrl: string;
  clientId: string;
  scope: string;
  redirectUri: string;
  resource: string;
};

export class AuthError extends Error {
  readonly code: string;
  readonly status: number | null;

  constructor(message: string, code: string, status: number | null = null) {
    super(message);
    this.name = "AuthError";
    this.code = code;
    this.status = status;
  }
}

function environmentValue(name: string): string {
  const value =
    name === "EXPO_PUBLIC_OAUTH_ISSUER"
      ? process.env.EXPO_PUBLIC_OAUTH_ISSUER
      : name === "EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL"
        ? process.env.EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL
        : name === "EXPO_PUBLIC_OAUTH_TOKEN_URL"
          ? process.env.EXPO_PUBLIC_OAUTH_TOKEN_URL
          : name === "EXPO_PUBLIC_OAUTH_CLIENT_ID"
            ? process.env.EXPO_PUBLIC_OAUTH_CLIENT_ID
            : name === "EXPO_PUBLIC_OAUTH_SCOPE"
              ? process.env.EXPO_PUBLIC_OAUTH_SCOPE
              : name === "EXPO_PUBLIC_OAUTH_REDIRECT_URI"
                ? process.env.EXPO_PUBLIC_OAUTH_REDIRECT_URI
                : name === "EXPO_PUBLIC_OAUTH_RESOURCE"
                  ? process.env.EXPO_PUBLIC_OAUTH_RESOURCE
                  : name === "EXPO_PUBLIC_PRACTICE_API_URL"
                    ? process.env.EXPO_PUBLIC_PRACTICE_API_URL
                    : undefined;
  return typeof value === "string" ? value.trim() : "";
}

function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      (url.protocol === "https:" || url.protocol === "http:") &&
      Boolean(url.hostname) &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
    );
  } catch {
    return false;
  }
}

function isAllowedOAuthEndpoint(value: string, issuer: string, pathname: string): boolean {
  try {
    const endpoint = new URL(value);
    const issuerUrl = new URL(issuer);
    return (
      isHttpUrl(value) &&
      isHttpUrl(issuer) &&
      endpoint.origin === issuerUrl.origin &&
      endpoint.pathname === pathname
    );
  } catch {
    return false;
  }
}

function normalizedUrl(value: string): string {
  return value.replace(/\/+$/, "");
}

export function getOAuthConfiguration(): OAuthConfiguration {
  const isNative = process.env.EXPO_OS !== "web";
  const apiUrl = normalizedUrl(environmentValue("EXPO_PUBLIC_PRACTICE_API_URL") || PRACTICE_API_URL);
  const isProductionApi = apiUrl === DEFAULT_PRACTICE_API_URL;
  const issuer = normalizedUrl(
    environmentValue("EXPO_PUBLIC_OAUTH_ISSUER") || (isProductionApi ? DEFAULT_OAUTH_ISSUER : ""),
  );
  const authorizationUrl = normalizedUrl(
    environmentValue("EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL") || (issuer ? `${issuer}/authorize` : ""),
  );
  const tokenUrl = normalizedUrl(
    environmentValue("EXPO_PUBLIC_OAUTH_TOKEN_URL") || (issuer ? `${issuer}/api/oidc/token` : ""),
  );
  const clientId = environmentValue("EXPO_PUBLIC_OAUTH_CLIENT_ID") || (isProductionApi ? DEFAULT_OAUTH_CLIENT_ID : "");
  const scope = environmentValue("EXPO_PUBLIC_OAUTH_SCOPE") || DEFAULT_OAUTH_SCOPE;
  const redirectUri = environmentValue("EXPO_PUBLIC_OAUTH_REDIRECT_URI") || DEFAULT_REDIRECT_URI;
  const resource = normalizedUrl(environmentValue("EXPO_PUBLIC_OAUTH_RESOURCE") || apiUrl);
  const issues: string[] = [];

  if (!isNative) issues.push("Native cloud sign-in is available in the Android or iOS build only.");
  if (!apiUrl) issues.push("EXPO_PUBLIC_PRACTICE_API_URL is missing.");
  if (!issuer) issues.push("EXPO_PUBLIC_OAUTH_ISSUER is missing.");
  if (issuer && !isHttpUrl(issuer)) issues.push(`OAuth issuer is not an HTTP(S) URL: ${issuer}`);
  if (!authorizationUrl) issues.push("EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL is missing.");
  if (authorizationUrl && !isHttpUrl(authorizationUrl)) issues.push(`OAuth authorization URL is invalid: ${authorizationUrl}`);
  if (
    issuer &&
    authorizationUrl &&
    !isAllowedOAuthEndpoint(authorizationUrl, issuer, "/authorize")
  ) {
    issues.push("OAuth authorization URL must be the /authorize endpoint on the configured issuer.");
  }
  if (!tokenUrl) issues.push("EXPO_PUBLIC_OAUTH_TOKEN_URL is missing.");
  if (tokenUrl && !isHttpUrl(tokenUrl)) issues.push(`OAuth token URL is invalid: ${tokenUrl}`);
  if (issuer && tokenUrl && !isAllowedOAuthEndpoint(tokenUrl, issuer, "/api/oidc/token")) {
    issues.push("OAuth token URL must be the /api/oidc/token endpoint on the configured issuer.");
  }
  if (!clientId) issues.push("EXPO_PUBLIC_OAUTH_CLIENT_ID is missing.");
  if (!scope) issues.push("EXPO_PUBLIC_OAUTH_SCOPE is empty.");
  if (!redirectUri) issues.push("EXPO_PUBLIC_OAUTH_REDIRECT_URI is missing.");
  if (!resource) issues.push("EXPO_PUBLIC_OAUTH_RESOURCE is missing.");
  if (resource && !isHttpUrl(resource)) issues.push(`OAuth resource is invalid: ${resource}`);
  if (apiUrl && resource && isHttpUrl(apiUrl) && isHttpUrl(resource)) {
    if (new URL(resource).origin !== new URL(apiUrl).origin || new URL(resource).pathname !== new URL(apiUrl).pathname) {
      issues.push("OAuth resource must match the configured practice API origin.");
    }
  }

  return {
    configured: issues.length === 0,
    issues,
    apiUrl,
    issuer,
    authorizationUrl,
    tokenUrl,
    clientId,
    scope,
    redirectUri,
    resource,
  };
}

function base64Url(bytes: Uint8Array): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let output = "";
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index] ?? 0;
    const second = bytes[index + 1] ?? 0;
    const third = bytes[index + 2] ?? 0;
    const value = (first << 16) | (second << 8) | third;
    output += alphabet[(value >> 18) & 63];
    output += alphabet[(value >> 12) & 63];
    if (index + 1 < bytes.length) output += alphabet[(value >> 6) & 63];
    if (index + 2 < bytes.length) output += alphabet[value & 63];
  }
  return output.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function configuredSettings(): OAuthConfiguration {
  const settings = getOAuthConfiguration();
  if (!settings.configured) {
    throw new AuthError(
      `Cloud sign-in is not configured. ${settings.issues.join(" ")}`,
      "configuration_missing",
    );
  }
  return settings;
}

function oauthErrorDetail(payload: unknown): string | null {
  if (!payload || typeof payload !== "object") return null;
  const value = payload as Record<string, unknown>;
  for (const key of ["error_description", "message", "error", "detail"]) {
    if (typeof value[key] === "string" && value[key]) return value[key];
  }
  return null;
}

async function readOAuthError(response: Response): Promise<string | null> {
  try {
    const payload = (await response.json()) as unknown;
    return oauthErrorDetail(payload);
  } catch {
    return null;
  }
}

async function postToken(settings: OAuthConfiguration, body: string): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), OAUTH_TOKEN_TIMEOUT_MS);
  try {
    const issuerOrigin = new URL(settings.issuer).origin;
    const tokenUrl = allowlistedHttpUrl(
      settings.tokenUrl,
      [issuerOrigin],
      "OAuth token endpoint",
    );
    // The endpoint is constrained to the configured issuer and exact OAuth path above.
    // foxguard: ignore[js/no-ssrf]
    return await fetch(tokenUrl, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
      body,
      signal: controller.signal,
    });
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") {
      throw new AuthError("Pocket ID token exchange timed out after 30 seconds.", "token_timeout");
    }
    throw new AuthError(`Pocket ID token endpoint is unreachable: ${errorMessage(error)}`, "token_network");
  } finally {
    clearTimeout(timeout);
  }
}

export async function getAccessToken(): Promise<string | null> {
  try {
    return await SecureStore.getItemAsync(STORAGE_ID);
  } catch (error) {
    logError("auth.storage", error, { operation: "read_access_token" });
    throw new AuthError(`Could not read the stored sign-in: ${errorMessage(error)}`, "secure_store_failed");
  }
}

export async function clearAccessToken(): Promise<void> {
  try {
    await SecureStore.deleteItemAsync(STORAGE_ID);
  } catch (error) {
    logError("auth.storage", error, { operation: "delete_access_token" });
    throw new AuthError(`Could not remove the stored sign-in: ${errorMessage(error)}`, "secure_store_failed");
  }
}

export async function signInWithBrowser(): Promise<void> {
  const settings = configuredSettings();
  logInfo("auth", "Starting Pocket ID sign-in", {
    issuer: settings.issuer,
    authorizationUrl: settings.authorizationUrl,
    tokenUrl: settings.tokenUrl,
    clientId: settings.clientId,
    scope: settings.scope,
    redirectUri: settings.redirectUri,
    resource: settings.resource,
  });

  const verifier = `${randomUUID()}${randomUUID()}`;
  const challengeBytes = new Uint8Array(await digest(CryptoDigestAlgorithm.SHA256, new TextEncoder().encode(verifier)));
  const state = randomUUID();
  const authorizationParams = [
    ["client_id", settings.clientId],
    ["redirect_uri", settings.redirectUri],
    ["response_type", "code"],
    ["scope", settings.scope],
    ["resource", settings.resource],
    ["state", state],
    ["code_challenge", base64Url(challengeBytes)],
    ["code_challenge_method", "S256"],
  ]
    .map(([key, value]) => `${key}=${encodeURIComponent(value)}`)
    .join("&");

  let result: WebBrowser.WebBrowserAuthSessionResult;
  try {
    result = await WebBrowser.openAuthSessionAsync(`${settings.authorizationUrl}?${authorizationParams}`, settings.redirectUri);
  } catch (error) {
    logError("auth.browser", error, { operation: "open_auth_session" });
    throw new AuthError(`Could not open Pocket ID sign-in: ${errorMessage(error)}`, "browser_open_failed");
  }

  logInfo("auth", "Pocket ID browser session finished", { resultType: result.type });
  if (result.type !== "success") {
    throw new AuthError(`Pocket ID sign-in did not complete (browser result: ${result.type}).`, "browser_cancelled");
  }

  let callback: URL;
  try {
    callback = new URL(result.url);
  } catch (error) {
    logError("auth.callback", error, { operation: "parse_callback_url" });
    throw new AuthError(`Pocket ID returned an invalid callback URL: ${result.url}`, "callback_invalid");
  }
  if (callback.searchParams.get("state") !== state) {
    logWarn("auth.callback", "Pocket ID callback state did not match", { callbackHost: callback.host, callbackPath: callback.pathname });
    throw new AuthError("Pocket ID returned an invalid OAuth state. Start sign-in again.", "state_mismatch");
  }
  const error = callback.searchParams.get("error");
  if (error) {
    const description = callback.searchParams.get("error_description");
    const suffix = description ? `: ${description}` : "";
    throw new AuthError(`Pocket ID rejected sign-in (${error}${suffix}).`, "authorization_failed");
  }
  const code = callback.searchParams.get("code");
  if (!code) throw new AuthError("Pocket ID returned no authorization code.", "code_missing");

  const body = [
    ["grant_type", "authorization_code"],
    ["code", code],
    ["client_id", settings.clientId],
    ["redirect_uri", settings.redirectUri],
    ["code_verifier", verifier],
  ]
    .map(([key, value]) => `${key}=${encodeURIComponent(value)}`)
    .join("&");
  const tokenResponse = await postToken(settings, body);
  if (!tokenResponse.ok) {
    const detail = await readOAuthError(tokenResponse);
    const suffix = detail ? `: ${detail}` : "";
    const authError = new AuthError(
      `Pocket ID token exchange failed with HTTP ${tokenResponse.status}${suffix}.`,
      "token_rejected",
      tokenResponse.status,
    );
    logError("auth.token", authError, { status: tokenResponse.status });
    throw authError;
  }

  let token: { access_token?: unknown };
  try {
    token = (await tokenResponse.json()) as { access_token?: unknown };
  } catch (error) {
    logError("auth.token", error, { operation: "parse_token_response" });
    throw new AuthError("Pocket ID returned an unreadable token response.", "token_response_invalid");
  }
  if (typeof token.access_token !== "string" || !token.access_token) {
    throw new AuthError("Pocket ID returned no usable access token.", "access_token_missing");
  }
  try {
    await SecureStore.setItemAsync(STORAGE_ID, token.access_token);
  } catch (error) {
    logError("auth.storage", error, { operation: "write_access_token" });
    throw new AuthError(`Sign-in succeeded, but the access token could not be stored securely: ${errorMessage(error)}`, "secure_store_failed");
  }
  logInfo("auth", "Pocket ID sign-in completed and access token stored");
}
