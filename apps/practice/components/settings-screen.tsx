import { useEffect, useMemo, useState } from "react";
import { Link } from "expo-router";
import { Pressable, ScrollView, StyleSheet, Switch, Text, TextInput, View } from "react-native";

import { formatQuotaHours, quotaBytes, selectEvictions } from "@/lib/storage/quota";
import { getSettings, getStorageUsage, listRecordings, updateSettings, type StorageUsage } from "@/lib/storage/repository";
import type { PracticeSettings } from "@/lib/types";
import { clearAccessToken, getAccessToken, getOAuthConfiguration, signInWithBrowser } from "@/lib/auth";
import { errorMessage, logError, logWarn } from "@/lib/logging";
import {
  APP_ANDROID_VERSION_CODE,
  APP_BUILD_DATE,
  APP_BUILD_VARIANT,
  APP_IDENTIFIER,
  APP_VERSION,
} from "@/lib/build-info";

type SettingsScreenProps = { onBack?: () => void };
type Notice = { kind: "error" | "success"; text: string };

export function SettingsScreen({ onBack }: SettingsScreenProps) {
  const [settings, setSettings] = useState<PracticeSettings | null>(null);
  const [usage, setUsage] = useState<StorageUsage>({ audioBytes: 0, reservedBytes: 0, recordingCount: 0 });
  const [recordings, setRecordings] = useState<Awaited<ReturnType<typeof listRecordings>>>([]);
  const [quotaText, setQuotaText] = useState("1");
  const [preparationText, setPreparationText] = useState("120");
  const [signedIn, setSignedIn] = useState(false);
  const [authBusy, setAuthBusy] = useState(false);
  const [message, setMessage] = useState<Notice | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    void Promise.all([getSettings(), getStorageUsage(), listRecordings()])
      .then(([nextSettings, nextUsage, nextRecordings]) => {
        setSettings(nextSettings);
        setQuotaText(String(nextSettings.quotaGb));
        setPreparationText(String(nextSettings.preparationSeconds));
        setUsage(nextUsage);
        setRecordings(nextRecordings);
        setLoadError(null);
      })
      .catch((error) => {
        logError("settings.load", error);
        setLoadError(`Settings could not be loaded: ${errorMessage(error)}`);
      });
    void getAccessToken()
      .then((token) => setSignedIn(Boolean(token)))
      .catch((error) => {
        logError("settings.auth", error, { operation: "read_access_token" });
        setMessage({ kind: "error", text: `The sign-in state could not be read: ${errorMessage(error)}` });
      });
  }, []);

  const preview = useMemo(() => {
    if (!settings) return [];
    try {
      return selectEvictions(
        recordings.map((recording) => ({
          id: recording.id,
          createdOrder: recording.createdOrder,
          bytes: recording.bytes,
          protected: false,
          status: recording.mediaAvailability,
        })),
        quotaBytes(Number(quotaText)),
        usage.audioBytes,
        usage.reservedBytes,
        0,
      );
    } catch {
      return [];
    }
  }, [recordings, quotaText, settings, usage.audioBytes, usage.reservedBytes]);

  const save = async (patch: Partial<PracticeSettings>) => {
    if (!settings) return;
    try {
      const next = await updateSettings(patch);
      setSettings(next);
      setMessage({ kind: "success", text: "Settings saved on this device." });
    } catch (error) {
      logError("settings.save", error, { patch });
      setMessage({ kind: "error", text: `Settings could not be saved: ${errorMessage(error, "unknown storage error")}` });
    }
  };

  if (!settings) {
    return (
      <View style={styles.loading}>
        <Text style={loadError ? styles.error : styles.helper}>{loadError ?? "Loading settings…"}</Text>
        {loadError ? (
          <Link href={"/logs" as any} asChild>
            <Pressable style={styles.linkButton}><Text style={styles.linkText}>Open system logs</Text></Pressable>
          </Link>
        ) : null}
      </View>
    );
  }

  const quotaNumber = Number(quotaText);
  const quotaIsValid = Number.isFinite(quotaNumber) && quotaNumber > 0;
  const preparationNumber = Number(preparationText);
  const preparationIsValid = Number.isInteger(preparationNumber) && preparationNumber >= 0 && preparationNumber <= 900;
  const oauth = getOAuthConfiguration();

  const authenticate = async () => {
    if (!oauth.configured) {
      const text = `Cloud sign-in is unavailable: ${oauth.issues.join(" ")}`;
      logWarn("settings.auth", "Sign-in blocked by OAuth configuration", { issues: oauth.issues });
      setMessage({ kind: "error", text });
      return;
    }
    setAuthBusy(true);
    try {
      await signInWithBrowser();
      setSignedIn(true);
      setMessage({ kind: "success", text: "Cloud coaching access token stored securely on this device. The practice API will verify it on the next request." });
    } catch (error) {
      logError("settings.auth", error, { operation: "sign_in" });
      setMessage({ kind: "error", text: errorMessage(error, "Cloud sign-in could not be completed.") });
    } finally {
      setAuthBusy(false);
    }
  };

  const signOut = async () => {
    try {
      await clearAccessToken();
      setSignedIn(false);
      setMessage({ kind: "success", text: "Cloud coaching sign-in was removed from this device." });
    } catch (error) {
      logError("settings.auth", error, { operation: "sign_out" });
      setMessage({ kind: "error", text: errorMessage(error, "Cloud sign-out could not be completed.") });
    }
  };

  return (
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.scrollContent} style={styles.scroll}>
      <View style={styles.page}>
        {onBack ? <Pressable onPress={onBack} style={styles.backButton}><Text style={styles.backText}>‹ Practice</Text></Pressable> : null}
        <View style={styles.header}>
          <Text style={styles.title} selectable>Settings & storage</Text>
          <Text style={styles.subtitle} selectable>Control the device audio budget, assistance labels, and paid analysis consent.</Text>
        </View>
        {message ? <Text accessibilityLiveRegion="polite" style={[styles.message, message.kind === "error" && styles.messageError]} selectable>{message.text}</Text> : null}

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Audio retention</Text>
          <Text style={styles.helper} selectable>Keep up to N decimal GB of recent audio. Oldest eligible audio is removed automatically; playback does not make a file recent.</Text>
          <View style={styles.inputRow}>
            <TextInput
              accessibilityLabel="Audio quota in decimal gigabytes"
              keyboardType="decimal-pad"
              onChangeText={setQuotaText}
              style={styles.numberInput}
              value={quotaText}
            />
            <Text style={styles.unit} selectable>GB</Text>
            <Pressable disabled={!quotaIsValid} onPress={() => void save({ quotaGb: quotaNumber })} style={[styles.smallButton, !quotaIsValid && styles.disabledButton]}>
              <Text style={styles.smallButtonText}>Apply</Text>
            </Pressable>
          </View>
          {quotaIsValid ? <Text style={styles.helper} selectable>{formatQuotaHours(quotaNumber, 64_000)} · {preview.length} recording(s) would be eligible for eviction under this preview.</Text> : <Text style={styles.error} selectable>Enter a finite positive number.</Text>}
          <Text style={styles.usage} selectable>{usage.audioBytes.toLocaleString()} bytes audio · {usage.reservedBytes.toLocaleString()} protected/reserved bytes · {usage.recordingCount} saved file(s)</Text>
          <Text style={styles.method} selectable>The budget covers managed audio, staging files, upload copies, and playback caches. Database and image caches are shown separately when measured.</Text>
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Analysis controls</Text>
          <View style={styles.settingRow}>
            <View style={styles.settingCopy}><Text style={styles.settingTitle} selectable>Allow paid coaching</Text><Text style={styles.helper} selectable>Recording and local history remain available when this is off.</Text></View>
            <Switch accessibilityLabel="Allow paid coaching" onValueChange={(value) => void save({ analysisConsent: value })} value={settings.analysisConsent} />
          </View>
          <View style={styles.inputRow}>
            <Text style={styles.settingTitle} selectable>Monthly analysis cap</Text>
            <TextInput accessibilityLabel="Monthly analysis spending cap" keyboardType="decimal-pad" onChangeText={(value) => { const parsed = Number(value); if (Number.isFinite(parsed)) void save({ monthlySpendLimitUsd: Math.max(0, parsed) }); }} style={styles.capInput} value={String(settings.monthlySpendLimitUsd)} />
            <Text style={styles.unit} selectable>USD</Text>
          </View>
          <Text style={styles.method} selectable>Actual provider usage is returned when available. The estimate is not an invoice, and a timeout after provider acceptance may have unknown billing.</Text>
        </View>

        {process.env.EXPO_OS !== "web" ? (
          <View style={styles.card}>
            <Text style={styles.sectionTitle} selectable>Cloud account</Text>
            <Text style={styles.helper} selectable>Sign in through Pocket ID in the system browser. The app stores only the returned access token in secure device storage. A stored token is reported locally; the practice API verifies it when feedback is requested.</Text>
            <View style={styles.inputRow}>
              <Text style={styles.settingTitle} selectable>{signedIn ? "Access token stored" : "Not connected"}</Text>
              {signedIn ? (
                <Pressable onPress={() => void signOut()} style={styles.smallButton}><Text style={styles.smallButtonText}>Sign out</Text></Pressable>
              ) : (
                <Pressable disabled={authBusy} onPress={() => void authenticate()} style={[styles.smallButton, authBusy && styles.disabledButton]}><Text style={styles.smallButtonText}>{authBusy ? "Opening sign-in…" : "Sign in"}</Text></Pressable>
              )}
            </View>
            <View style={[styles.configBox, !oauth.configured && styles.configErrorBox]}>
              <Text style={styles.configTitle} selectable>{oauth.configured ? "Pocket ID configuration ready" : "Pocket ID configuration needs attention"}</Text>
              {oauth.issues.map((issue) => <Text key={issue} style={styles.error} selectable>• {issue}</Text>)}
              <Text style={styles.configValue} selectable>Issuer: {oauth.issuer || "missing"}</Text>
              <Text style={styles.configValue} selectable>Authorize: {oauth.authorizationUrl || "missing"}</Text>
              <Text style={styles.configValue} selectable>Token: {oauth.tokenUrl || "missing"}</Text>
              <Text style={styles.configValue} selectable>Client: {oauth.clientId || "missing"}</Text>
              <Text style={styles.configValue} selectable>Scope: {oauth.scope || "missing"}</Text>
              <Text style={styles.configValue} selectable>Resource: {oauth.resource || "missing"}</Text>
              <Text style={styles.configValue} selectable>Redirect: {oauth.redirectUri || "missing"}</Text>
            </View>
            <Text style={styles.method} selectable>Cloud analysis remains queued locally when you are offline. OAuth errors include the failing stage and HTTP status; open system logs for the full developer trace.</Text>
          </View>
        ) : null}

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Build information</Text>
          <Text style={styles.configValue} selectable>Version {APP_VERSION}{APP_ANDROID_VERSION_CODE === null ? "" : ` · Android version code ${APP_ANDROID_VERSION_CODE}`}</Text>
          <Text style={styles.configValue} selectable>Build date {APP_BUILD_DATE}</Text>
          <Text style={styles.configValue} selectable>Variant {APP_BUILD_VARIANT} · {APP_IDENTIFIER}</Text>
          <Link href={"/logs" as any} asChild>
            <Pressable style={styles.linkButton}><Text style={styles.linkText}>Open system logs</Text></Pressable>
          </Link>
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Photo practice</Text>
          <Text style={styles.helper} selectable>Choose whether the photo task offers optional guidance, and set its local preparation timer. This is an app practice choice, not the full official oral exam timing.</Text>
          <View style={styles.choiceRow}>
            {(["guided", "simulation"] as const).map((style) => (
              <Pressable key={style} onPress={() => void save({ photoPracticeStyle: style })} style={[styles.choiceButton, settings.photoPracticeStyle === style && styles.choiceSelected]}>
                <Text style={styles.languageText}>{style === "guided" ? "Guided" : "Simulation"}</Text>
              </Pressable>
            ))}
          </View>
          <View style={styles.inputRow}>
            <TextInput accessibilityLabel="Photo preparation seconds" keyboardType="number-pad" onChangeText={setPreparationText} style={styles.numberInput} value={preparationText} />
            <Text style={styles.unit} selectable>seconds</Text>
            <Pressable disabled={!preparationIsValid} onPress={() => void save({ preparationSeconds: preparationNumber })} style={[styles.smallButton, !preparationIsValid && styles.disabledButton]}><Text style={styles.smallButtonText}>Apply</Text></Pressable>
          </View>
          {!preparationIsValid ? <Text style={styles.error} selectable>Use a whole number from 0 to 900 seconds.</Text> : null}
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Accessibility & language</Text>
          <View style={styles.settingRow}>
            <View style={styles.settingCopy}><Text style={styles.settingTitle} selectable>Explanation language</Text><Text style={styles.helper} selectable>Spanish speech and task prompts stay in Spanish; this controls coaching explanation copy when supported.</Text></View>
            <View style={styles.languageRow}>
              {(["en", "es"] as const).map((language) => <Pressable key={language} onPress={() => void save({ explanationLanguage: language })} style={[styles.languageButton, settings.explanationLanguage === language && styles.languageSelected]}><Text style={styles.languageText}>{language.toUpperCase()}</Text></Pressable>)}
            </View>
          </View>
          <Text style={styles.method} selectable>Controls are designed for screen readers, large text, and 44-point touch targets. Timer updates are visual and are not announced every second.</Text>
        </View>

        <View style={styles.linksRow}>
          <Link href={"/history" as any} asChild><Pressable style={styles.linkButton}><Text style={styles.linkText}>Open history & export</Text></Pressable></Link>
          <Text style={styles.apiUrl} selectable>Practice API endpoint: {oauth.apiUrl || "not configured"}</Text>
        </View>
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { backgroundColor: "#f7efe8", flex: 1 },
  scrollContent: { paddingBottom: 48, paddingHorizontal: 18, paddingTop: 20 },
  loading: { alignItems: "center", backgroundColor: "#f7efe8", flex: 1, justifyContent: "center" },
  page: { alignSelf: "center", gap: 16, maxWidth: 760, width: "100%" },
  backButton: { alignSelf: "flex-start", minHeight: 44, justifyContent: "center" },
  backText: { color: "#6f3c26", fontSize: 14, fontWeight: "800" },
  header: { gap: 5 },
  title: { color: "#2d241f", fontSize: 30, fontWeight: "900" },
  subtitle: { color: "#75685e", fontSize: 15, lineHeight: 22 },
  message: { backgroundColor: "#edf6ee", borderRadius: 10, color: "#356345", fontSize: 13, lineHeight: 19, padding: 11 },
  messageError: { backgroundColor: "#fbe8e5", color: "#8c302a" },
  card: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 13, padding: 18 },
  sectionTitle: { color: "#2d241f", fontSize: 20, fontWeight: "800" },
  helper: { color: "#75685e", flex: 1, fontSize: 13, lineHeight: 20 },
  method: { color: "#8a7b70", fontSize: 12, lineHeight: 18 },
  inputRow: { alignItems: "center", flexDirection: "row", flexWrap: "wrap", gap: 9 },
  numberInput: { backgroundColor: "#fffaf6", borderColor: "#d9c9ba", borderRadius: 10, borderWidth: 1, color: "#2d241f", fontSize: 16, minHeight: 46, paddingHorizontal: 11, width: 100 },
  capInput: { backgroundColor: "#fffaf6", borderColor: "#d9c9ba", borderRadius: 10, borderWidth: 1, color: "#2d241f", fontSize: 15, minHeight: 44, paddingHorizontal: 10, width: 90 },
  unit: { color: "#6f3c26", fontSize: 14, fontWeight: "800" },
  smallButton: { alignItems: "center", backgroundColor: "#a34f2d", borderRadius: 10, justifyContent: "center", minHeight: 46, paddingHorizontal: 13 },
  smallButtonText: { color: "#fffaf6", fontSize: 13, fontWeight: "800" },
  disabledButton: { backgroundColor: "#cbbdb1" },
  usage: { color: "#493d35", fontSize: 13, fontVariant: ["tabular-nums"], lineHeight: 19 },
  error: { color: "#8c302a", fontSize: 13, lineHeight: 19 },
  configBox: { backgroundColor: "#f4f7f2", borderColor: "#c8d9c8", borderRadius: 12, borderWidth: 1, gap: 4, padding: 12 },
  configErrorBox: { backgroundColor: "#fff5f1", borderColor: "#d9a49a" },
  configTitle: { color: "#356345", fontSize: 13, fontWeight: "900" },
  configValue: { color: "#63564d", fontFamily: "monospace", fontSize: 11, lineHeight: 16 },
  settingRow: { alignItems: "center", flexDirection: "row", gap: 12, justifyContent: "space-between" },
  settingCopy: { flex: 1, gap: 3 },
  settingTitle: { color: "#493d35", fontSize: 14, fontWeight: "800" },
  languageRow: { flexDirection: "row", gap: 6 },
  choiceRow: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  choiceButton: { alignItems: "center", borderColor: "#c9b8a8", borderRadius: 9, borderWidth: 1, justifyContent: "center", minHeight: 44, paddingHorizontal: 13 },
  choiceSelected: { backgroundColor: "#fff1e7", borderColor: "#a34f2d" },
  languageButton: { alignItems: "center", borderColor: "#c9b8a8", borderRadius: 9, borderWidth: 1, justifyContent: "center", minHeight: 42, minWidth: 48 },
  languageSelected: { backgroundColor: "#fff1e7", borderColor: "#a34f2d" },
  languageText: { color: "#6f3c26", fontSize: 12, fontWeight: "800" },
  linksRow: { alignItems: "center", flexDirection: "row", flexWrap: "wrap", gap: 12 },
  linkButton: { alignItems: "center", borderColor: "#c9b8a8", borderRadius: 11, borderWidth: 1, justifyContent: "center", minHeight: 46, paddingHorizontal: 13 },
  linkText: { color: "#6f3c26", fontSize: 13, fontWeight: "800" },
  apiUrl: { color: "#8a7b70", flex: 1, fontSize: 11, lineHeight: 17 },
});
