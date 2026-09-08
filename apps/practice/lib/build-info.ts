import Constants from "expo-constants";

const expoConfig = Constants.expoConfig;

export const APP_VERSION = expoConfig?.version ?? "unknown";
export const APP_ANDROID_VERSION_CODE = expoConfig?.android?.versionCode ?? null;
export const APP_IDENTIFIER =
  expoConfig?.android?.package ?? expoConfig?.ios?.bundleIdentifier ?? expoConfig?.slug ?? "unknown";
export const APP_BUILD_DATE =
  (process.env.EXPO_PUBLIC_PRACTICE_BUILD_DATE ?? "").trim() || "development build (timestamp not set)";
export const APP_BUILD_VARIANT =
  (process.env.EXPO_PUBLIC_PRACTICE_BUILD_VARIANT ?? "").trim() ||
  (process.env.NODE_ENV === "production" ? "release" : "development");

