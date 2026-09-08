export function configuredHttpOrigin(value: string | undefined): string {
  const candidate = value?.trim().replace(/\/+$/, "") ?? "";
  if (!candidate) return "";
  try {
    const parsed = new URL(candidate);
    if (
      !["http:", "https:"].includes(parsed.protocol) ||
      !parsed.hostname ||
      parsed.username ||
      parsed.password ||
      parsed.search ||
      parsed.hash ||
      (parsed.pathname !== "" && parsed.pathname !== "/")
    ) {
      return "";
    }
    return parsed.origin;
  } catch {
    return "";
  }
}

export function allowlistedHttpUrl(
  value: string,
  allowedOrigins: readonly string[],
  label: string,
): string {
  const parsed = new URL(value);
  if (
    !["http:", "https:"].includes(parsed.protocol) ||
    !parsed.hostname ||
    parsed.username ||
    parsed.password ||
    parsed.search ||
    parsed.hash ||
    !allowedOrigins.includes(parsed.origin)
  ) {
    throw new Error(`${label} is not an allowed HTTP(S) destination.`);
  }
  return parsed.toString();
}

export function allowlistedApiPath(baseUrl: string, path: string): string {
  if (
    !path.startsWith("/") ||
    path.includes("\\") ||
    path.includes("://") ||
    path.includes("?") ||
    path.includes("#") ||
    path.split("/").some((segment) => segment === "." || segment === "..")
  ) {
    throw new Error("The practice API path is not allowed.");
  }
  const base = new URL(baseUrl);
  return allowlistedHttpUrl(
    new URL(path, `${base.origin}/`).toString(),
    [base.origin],
    "Practice API request",
  );
}

export function localBlobUrl(value: string): string {
  if (!value.startsWith("blob:")) {
    throw new Error("Only browser-local object URLs may be read here.");
  }
  const parsed = new URL(value);
  if (parsed.protocol !== "blob:") {
    throw new Error("Only browser-local object URLs may be read here.");
  }
  return value;
}

export function allowlistedBrowserAssetUrl(value: string): string {
  if (value.startsWith("blob:")) return localBlobUrl(value);
  const currentOrigin =
    typeof globalThis.location === "undefined" ? "" : globalThis.location.origin;
  const parsed = new URL(value);
  if (
    !currentOrigin ||
    !["http:", "https:"].includes(parsed.protocol) ||
    parsed.username ||
    parsed.password ||
    parsed.hash ||
    parsed.origin !== currentOrigin
  ) {
    throw new Error("Only browser-local or same-origin image URLs may be read here.");
  }
  return parsed.toString();
}
